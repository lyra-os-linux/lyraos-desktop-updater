#!/usr/bin/env python3
"""Cold boots of signed same-release maintenance in a disposable Btrfs VM.

No host disk, NIC, credentials or production private key enters the guest.
The guest has isolated /var and /home and verifies its disk serial before mkfs.
"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
from systemd_vm_support import SystemdVM

REPO = Path(__file__).resolve().parents[1]


def main():
    os.environ['PATH'] = '/usr/sbin:/usr/bin:/sbin:/bin'
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['kernel', 'modules-dir', 'fixtures', 'binary-dir', 'log-dir']:
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--scenario', choices=['success', 'rollback'], required=True)
    parser.add_argument('--accel', choices=['tcg', 'kvm'], default='tcg')
    args = parser.parse_args()
    args.log_dir.mkdir(parents=True, exist_ok=True)
    vm = SystemdVM('package-migration', 'lyra-package-migration-test')
    try:
        for name in ['umount', 'findmnt', 'cp', 'mv', 'rm', 'cat', 'date', 'sleep', 'chroot', 'switch_root', 'modprobe',
                     'mkfs.btrfs', 'btrfs', 'snapper', 'rpm', 'rpmdb', 'rpmkeys', 'zypper', 'repo2solv', 'rpmdb2solv',
                     'gpg', 'gpg2', 'gpgv', 'gpg-agent', 'gpgconf', 'gzip', 'xz', 'zstd', 'getent', 'stat', 'head', 'sed',
                     'readlink', 'test', 'timeout', 'curl', 'openssl', 'ip']:
            vm.tool(name)
        (vm.root/'sbin').symlink_to('usr/bin'); (vm.root/'usr/sbin').symlink_to('bin')
        for source in Path('/usr/lib64/rpm-plugins').glob('*.so'): vm.binary(source)
        for source in ['/usr/lib64/libnss_files.so.2', '/usr/lib64/libnss_dns.so.2']:
            if Path(source).is_file(): vm.binary(source)
        for source in ['/usr/lib/rpm', '/usr/share/snapper/config-templates']:
            shutil.copytree(source, vm.root/source.lstrip('/'), dirs_exist_ok=True, symlinks=True)
        vm.put('/etc/passwd', 'root:x:0:0:root:/root:/bin/bash\nfixture:x:1000:1000:fixture:/home/fixture:/bin/bash\n')
        vm.put('/etc/group', 'root:x:0:\nfixture:x:1000:\n')
        vm.put('/etc/hosts', '127.0.0.1 localhost fixture.invalid\n')
        (vm.root/'etc/mtab').symlink_to('../proc/self/mounts')
        vm.put('/usr/bin/mokutil', '#!/bin/sh\necho "SecureBoot disabled"\n', 0o755)
        for name in ['dracut', 'grub2-mkconfig']:
            vm.put('/usr/bin/'+name, '#!/bin/sh\necho unexpected-boot-rebuild >> /test/boot-rebuilds\nexit 1\n', 0o755)
        vm.put('/boot/grub2/grub.cfg', '# Fixture: no bootloader changes in this maintenance\n')
        vm.put('/usr/lib/lyra-upgrade/recovery-format', (REPO/'packaging/recovery-format').read_text())
        binaries = {}
        for source, dest in [('lyra-upgrade-offline', '/test/offline-worker'), ('lyra-upgrade-service', '/test/service'),
                             ('lyra-upgrade-verify', '/usr/libexec/lyra-upgrade-verify')]:
            vm.binary(args.binary_dir/source, dest)
            binaries[source] = hashlib.sha256((args.binary_dir/source).read_bytes()).hexdigest()
        shutil.copytree(args.fixtures, vm.root/'test/fixture', symlinks=True)
        vm.put('/test/guest.py', (REPO/'tests/package_migration_vm.py').read_text())
        vm.put('/test/scenario', args.scenario)
        vm.put('/etc/systemd/system/lyra-test.target', '[Unit]\nDefaultDependencies=no\nWants=basic.target sysinit.target multi-user.target lyra-test.service\n')
        vm.put('/etc/systemd/system/lyra-test.service', '[Unit]\nDefaultDependencies=no\nAfter=basic.target sysinit.target multi-user.target\n[Service]\nType=simple\nExecStart=/bin/bash /test/observe.sh\nStandardOutput=tty\nStandardError=inherit\nTTYPath=/dev/console\n')
        vm.put('/test/observe.sh', '''#!/bin/bash
export PATH=/usr/bin:/bin PYTHONDONTWRITEBYTECODE=1 PYTHON_COLORS=0
phase=verify
if [[ "$(cat /proc/cmdline)" == *lyra.migration-phase=rollback* ]]; then phase=rollback; fi
mkdir -p /run/lock
python3 -u /test/guest.py "$phase"
result=$?
echo "MIGRATION_${phase}_EXIT=$result"
systemctl --force --force poweroff
''', 0o755)
        modules = vm.root/'usr/lib/modules'/args.modules_dir.name; modules.mkdir(parents=True)
        (vm.root/'lib').mkdir(exist_ok=True); (vm.root/'lib/modules').symlink_to('../usr/lib/modules')
        for path in args.modules_dir.glob('modules.*'): shutil.copyfile(path, modules/path.name)
        for name in ['btrfs', 'virtio_blk', 'virtio_pci']:
            for line in subprocess.check_output(['modprobe', '--show-depends', '-S', args.modules_dir.name, name], text=True).splitlines():
                if line.startswith('insmod '):
                    source = Path(line.split()[1]); dest = modules/source.resolve().relative_to(args.modules_dir.resolve())
                    dest.parent.mkdir(parents=True, exist_ok=True); shutil.copyfile(source, dest)
        vm.put('/init', '''#!/bin/bash
export PATH=/usr/bin:/bin PYTHONDONTWRITEBYTECODE=1 PYTHON_COLORS=0
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mount -t tmpfs tmpfs /run
modprobe btrfs
modprobe virtio_pci
modprobe virtio_blk
[[ "$(cat /sys/block/vda/serial)" == lyra-migration-test ]] || exit 1
[[ "$(cat /proc/cmdline)" == *lyra.package-migration-test=1* ]] || exit 1
mkdir -p /target /top
phase=verify
for item in $(cat /proc/cmdline); do
    case "$item" in lyra.migration-phase=*) phase=${item#*=} ;; esac
done
if [[ "$phase" == online ]]; then
    mkfs.btrfs -f /dev/vda || exit 1
    mount /dev/vda /top || exit 1
    for volume in @ @var @home; do btrfs subvolume create /top/$volume || exit 1; done
    btrfs subvolume set-default /top/@ || exit 1
    mount /dev/vda /target || exit 1
    cp -a /usr /etc /test /bin /sbin /lib /lib64 /boot /target/ || exit 1
    mkdir -p /target/{var,home,dev,proc,sys,tmp,run,root}
    mount -o subvol=@var /dev/vda /target/var || exit 1
    cp -a /var/. /target/var/ || exit 1
else
    mount /dev/vda /target || exit 1
    mount -o subvol=@var /dev/vda /target/var || exit 1
    mount -o subvol=@/.snapshots /dev/vda /target/.snapshots || exit 1
fi
mount -o subvol=@home /dev/vda /target/home || exit 1
mkdir -p /target/run/lock
if [[ "$phase" == online || "$phase" == offline ]]; then
    mount --rbind /dev /target/dev
    mount -t proc proc /target/proc
    mount -t sysfs sysfs /target/sys
    chroot /target python3 -u /test/guest.py "$phase"
    result=$?
    echo "MIGRATION_${phase}_EXIT=$result"
    systemctl --force --force poweroff
else
    mount --move /dev /target/dev
    mount --move /proc /target/proc
    mount --move /sys /target/sys
    mount --move /run /target/run
    exec switch_root /target /usr/lib/systemd/systemd --system --log-level=warning --log-target=console --unit=lyra-test.target
fi
''', 0o755)
        files = b'\0'.join(str(path.relative_to(vm.root)).encode() for path in vm.root.rglob('*'))+b'\0'
        archive = subprocess.run(['cpio', '--null', '-o', '-H', 'newc', '--owner=0:0', '--quiet'], input=files, cwd=vm.root, capture_output=True, check=True)
        initrd = vm.base/'initramfs.gz'
        with gzip.open(initrd, 'wb', compresslevel=1) as stream: stream.write(archive.stdout)
        disk = vm.base/'root.raw'
        with disk.open('wb') as stream: stream.truncate(12*1024**3)
        phases = ['online', 'offline', 'verify'] + (['rollback'] if args.scenario == 'rollback' else [])
        print(f'Private VM ready: {vm.base}; {args.scenario}', flush=True)
        for phase in phases:
            log = args.log_dir/(phase+'.log')
            cmdline = f'rdinit=/init console=ttyS0 quiet panic=1 selinux=0 lyra.package-migration-test=1 lyra.migration-phase={phase}'
            with log.open('w') as stream:
                result = subprocess.run(['qemu-system-x86_64', '-accel', args.accel, '-cpu', 'host' if args.accel == 'kvm' else 'max', '-smp', '2', '-m', '2048',
                    '-kernel', str(args.kernel.resolve()), '-initrd', str(initrd), '-append', cmdline,
                    '-drive', f'file={disk},format=raw,if=none,id=root', '-device', 'virtio-blk-pci,drive=root,serial=lyra-migration-test',
                    '-display', 'none', '-serial', 'stdio', '-monitor', 'none', '-nic', 'none', '-no-reboot'], stdout=stream, stderr=subprocess.STDOUT, timeout=360)
            content = log.read_text(errors='replace')
            if result.returncode or f'MIGRATION_{phase}_EXIT=0' not in content:
                print(content[-14000:], flush=True)
                raise SystemExit('Migration VM failed: '+str(log))
            print(f'PASS {phase}: {log}', flush=True)
        report = dict(result='PASS', scenario=args.scenario, phases=phases, binaries=binaries,
            kernel=args.kernel.name, limits=['Fixture packages and ephemeral manifest signing key', 'Authenticated UID fixture, not interactive Polkit', 'Secure Boot disabled fixture; no bootloader rebuild', 'Does not qualify portal RPM or ISO'])
        (args.log_dir/'result.json').write_text(json.dumps(report, indent=2)+'\n')
    finally:
        shutil.rmtree(vm.base)


if __name__ == '__main__':
    main()
