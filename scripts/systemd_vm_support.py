"""Disposable QEMU/systemd test images; never copy host accounts or disks."""
import gzip
from pathlib import Path
import re
import shutil
import subprocess
import sys
import sysconfig
import tempfile


class SystemdVM:
    def __init__(self, name, os_id):
        self.base = Path(tempfile.mkdtemp(prefix=f'lyra-{name}-vm-'))
        self.root = self.base / 'root'
        for directory in ['dev', 'proc', 'sys', 'run', 'tmp', 'var/tmp', 'root', 'usr/bin', 'etc/systemd/system']:
            (self.root / directory).mkdir(parents=True, exist_ok=True)
        (self.root / 'tmp').chmod(0o1777)
        (self.root / 'var/tmp').chmod(0o1777)
        (self.root / 'var/run').symlink_to('../run')
        for name in ['bash', 'mount', 'systemctl', 'mkdir', 'touch']:
            self.tool(name)
        (self.root / 'bin').symlink_to('usr/bin')
        (self.root / 'usr/bin/sh').symlink_to('bash')
        for name in ['systemd', 'systemd-shutdown']:
            source = next((p for p in [Path('/usr/lib/systemd') / name,
                                      Path('/lib/systemd') / name] if p.is_file()), None)
            if source is None:
                raise FileNotFoundError(name)
            self.binary(source, '/usr/lib/systemd/' + name)
        executor = Path('/usr/lib/systemd/systemd-executor')
        if executor.is_file():
            self.binary(executor)
        self.binary(sys.executable, '/usr/bin/python3')
        stdlib = Path(sysconfig.get_path('stdlib'))
        shutil.copytree(stdlib, self.root / str(stdlib).lstrip('/'), ignore=shutil.ignore_patterns(
            'site-packages', 'dist-packages', '__pycache__', 'test', 'tests', 'ensurepip',
            'idlelib', 'tkinter', 'turtledemo', 'config-*'))
        for extension in (stdlib / 'lib-dynload').glob('*.so'):
            self.binary(extension)
        if Path('/etc/ld.so.cache').is_file():
            shutil.copyfile('/etc/ld.so.cache', self.root / 'etc/ld.so.cache')
        self.put('/etc/nsswitch.conf', 'passwd: files\ngroup: files\nshadow: files\nhosts: files\n')
        self.put('/etc/machine-id', '')
        self.put('/etc/os-release', f'ID={os_id}\nPRETTY_NAME="Lyra disposable test VM"\n')
        for name in ['sysinit', 'basic', 'multi-user', 'sockets', 'shutdown', 'network']:
            self.put(f'/usr/lib/systemd/system/{name}.target',
                     f'[Unit]\nDescription=Test {name}\nDefaultDependencies=no\n')

    def put(self, path, text, mode=0o644):
        target = self.root / path.lstrip('/')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
        target.chmod(mode)

    def tool(self, name):
        source = shutil.which(name)
        if not source:
            raise FileNotFoundError(name)
        self.binary(source, '/usr/bin/' + name)

    def binary(self, source, destination=None):
        source = Path(source)
        target = self.root / str(destination or source).lstrip('/')
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        target.chmod(0o755)
        result = subprocess.run(['ldd', str(source)], capture_output=True, text=True, check=True)
        for dependency in re.findall(r'(?:=>\s+|^\s*)(/[^\s]+)', result.stdout, re.M):
            dest = self.root / dependency.lstrip('/')
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(dependency, dest)
            dest.chmod(0o755)
            if '/glibc-hwcaps/' in dependency:
                dynamic = subprocess.check_output(['readelf', '-d', dependency], text=True)
                soname = re.search(r'\(SONAME\).*\[([^]]+)\]', dynamic)
                if soname:
                    baseline = Path(dependency.split('/glibc-hwcaps/')[0]) / soname[1]
                    destination = self.root / str(baseline).lstrip('/')
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(baseline, destination)

    def run(self, kernel, log, guest_source, guest_marker, success_marker):
        self.put('/etc/systemd/system/lyra-test.target', '[Unit]\nDefaultDependencies=no\n'
                 'Wants=basic.target sysinit.target multi-user.target lyra-upgrade-verify.service lyra-test.service\n')
        self.put('/etc/systemd/system/lyra-test.service', '''[Unit]
DefaultDependencies=no
After=basic.target sysinit.target
[Service]
Type=simple
ExecStart=/bin/bash /test.sh
StandardOutput=tty
StandardError=inherit
TTYPath=/dev/console
TimeoutStartSec=210
''')
        self.put('/test_guest.py', Path(guest_source).read_text())
        self.put('/test.sh', f'''#!/bin/bash
export PATH=/usr/bin:/bin
export PYTHON_COLORS=0
touch /run/{guest_marker}
python3 -u /test_guest.py
result=$?
echo "VM_GUEST_EXIT=$result"
systemctl --force --force poweroff
''', 0o755)
        self.put('/init', '''#!/bin/bash
export PATH=/usr/bin:/bin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mount -t tmpfs tmpfs /run
mkdir -p /dev/pts /sys/fs/cgroup
mount -t devpts devpts /dev/pts
mount -t cgroup2 cgroup2 /sys/fs/cgroup
exec /usr/lib/systemd/systemd --system --log-level=warning --log-target=console --unit=lyra-test.target
''', 0o755)
        files = b'\0'.join(str(path.relative_to(self.root)).encode() for path in self.root.rglob('*')) + b'\0'
        archive = subprocess.run(['cpio', '--null', '-o', '--format=newc', '--owner=0:0'],
                                 cwd=self.root, input=files, capture_output=True, check=True)
        initrd = self.base / 'initramfs.cpio.gz'
        with gzip.open(initrd, 'wb', compresslevel=1) as stream:
            stream.write(archive.stdout)
        print(f'VM image: {initrd} ({initrd.stat().st_size} bytes)', flush=True)
        log.parent.mkdir(parents=True, exist_ok=True)
        command = ['qemu-system-x86_64', '-accel', 'tcg', '-cpu', 'max', '-smp', '2', '-m', '1536',
                   '-kernel', str(kernel.resolve()), '-initrd', str(initrd),
                   '-append', 'rdinit=/init console=ttyS0 quiet panic=1 selinux=0 systemd.log_level=warning',
                   '-display', 'none', '-serial', 'stdio', '-monitor', 'none', '-no-reboot', '-nic', 'none']
        with log.open('w') as stream:
            result = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT, timeout=260)
        content = log.read_text(errors='replace')
        print(content[-18000:])
        if result.returncode or success_marker not in content or 'VM_GUEST_EXIT=0' not in content:
            raise SystemExit('VM validation failed; see ' + str(log))
        print('PASS: disposable systemd VM; evidence: ' + str(log))
