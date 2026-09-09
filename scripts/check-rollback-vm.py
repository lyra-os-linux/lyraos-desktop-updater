#!/usr/bin/env python3
"""Two cold boots on a disposable Btrfs disk: actual failed RPM upgrade and Snapper rollback."""
import argparse
import gzip
from pathlib import Path
import shutil
import subprocess
from systemd_vm_support import SystemdVM

REPO = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['kernel', 'modules-dir', 'fixtures', 'binary-dir', 'baseline-verifier', 'log-dir']:
        parser.add_argument('--'+name, type=Path, required=True)
    args = parser.parse_args()
    vm = SystemdVM('rollback', 'lyra-rollback-test')
    try:
        for name in ['umount', 'findmnt', 'cp', 'mv', 'rm', 'cat', 'date', 'sleep', 'chroot', 'switch_root', 'modprobe',
                     'mkfs.btrfs', 'btrfs', 'snapper', 'rpm', 'rpmdb', 'rpmkeys', 'zypper', 'repo2solv', 'rpmdb2solv',
                     'gpg', 'gpg2', 'gpg-agent', 'gzip', 'xz', 'zstd', 'getent', 'stat', 'head', 'sed', 'readlink', 'test', 'timeout']:
            vm.tool(name)
        (vm.root/'sbin').symlink_to('usr/bin'); (vm.root/'usr/sbin').symlink_to('bin')
        for source in Path('/usr/lib64/rpm-plugins').glob('*.so'): vm.binary(source)
        for source in ['/usr/lib64/libnss_files.so.2', '/usr/lib64/libnss_dns.so.2']:
            if Path(source).is_file(): vm.binary(source)
        for source in ['/usr/lib/rpm', '/usr/share/snapper/config-templates']:
            shutil.copytree(source, vm.root/source.lstrip('/'), dirs_exist_ok=True, symlinks=True)
        vm.put('/etc/passwd', 'root:x:0:0:root:/root:/bin/bash\n')
        vm.put('/etc/group', 'root:x:0:\n')
        vm.put('/etc/hosts', '127.0.0.1 localhost\n')
        (vm.root/'etc/mtab').symlink_to('../proc/self/mounts')
        for name, body in [('mokutil', 'echo "SecureBoot disabled"'), ('dracut', 'echo dracut >> /test/boot-rebuilds'),
                           ('grub2-mkconfig', 'echo grub >> /test/boot-rebuilds; exit 1')]:
            vm.put('/usr/bin/'+name, '#!/bin/sh\n'+body+'\n', 0o755)
        vm.put('/boot/grub2/grub.cfg', '# GRUB fixture for rollback identity qualification\n')
        vm.put('/usr/lib/lyra-upgrade/recovery-format', (REPO/'packaging/recovery-format').read_text())
        for source, dest in [('lyra-upgrade-offline', '/test/offline-worker'), ('examples/offline-vm-plan', '/test/prepare-plan'),
                             ('examples/rollback-vm', '/test/schedule-rollback'), ('lyra-upgrade-verify', '/usr/libexec/lyra-upgrade-verify')]:
            vm.binary(args.binary_dir/source, dest)
        vm.binary(args.baseline_verifier, '/test/baseline-verifier')
        shutil.copytree(args.fixtures, vm.root/'test/fixture', symlinks=True)
        vm.put('/test/offline_vm_scenarios.py', (REPO/'tests/offline_vm_scenarios.py').read_text())
        vm.put('/test/rollback_vm.py', (REPO/'tests/rollback_vm.py').read_text())
        vm.put('/usr/lib/systemd/system/lyra-upgrade-verify.service', (REPO/'packaging/lyra-upgrade-verify.service').read_text())
        vm.put('/etc/systemd/system/lyra-upgrade-verify.service.d/console.conf', '[Service]\nStandardOutput=tty\nStandardError=inherit\nTTYPath=/dev/console\n')
        vm.put('/etc/systemd/system/lyra-test.target', '[Unit]\nDefaultDependencies=no\nWants=basic.target sysinit.target multi-user.target lyra-upgrade-verify.service lyra-test.service\n')
        vm.put('/etc/systemd/system/lyra-test.service', '[Unit]\nDefaultDependencies=no\nAfter=basic.target sysinit.target\n[Service]\nType=simple\nExecStart=/bin/bash /test/observe.sh\nStandardOutput=tty\nStandardError=inherit\nTTYPath=/dev/console\n')
        vm.put('/test/observe.sh', '#!/bin/bash\npython3 -u /test/rollback_vm.py verify\nresult=$?\necho "ROLLBACK_VERIFY_EXIT=$result"\nsystemctl --force --force poweroff\n', 0o755)
        modules = vm.root/'usr/lib/modules'/args.modules_dir.name; modules.mkdir(parents=True)
        (vm.root/'lib').mkdir(exist_ok=True); (vm.root/'lib/modules').symlink_to('../usr/lib/modules')
        for path in args.modules_dir.glob('modules.*'): shutil.copyfile(path, modules/path.name)
        for name in ['btrfs', 'virtio_blk', 'virtio_pci']:
            for line in subprocess.check_output(['modprobe', '--show-depends', '-S', args.modules_dir.name, name], text=True).splitlines():
                if line.startswith('insmod '):
                    source = Path(line.split()[1]); dest = modules/source.resolve().relative_to(args.modules_dir.resolve())
                    dest.parent.mkdir(parents=True, exist_ok=True); shutil.copyfile(source, dest)
        vm.put('/init', '''#!/bin/bash
export PATH=/usr/bin:/bin
export PYTHONDONTWRITEBYTECODE=1
export PYTHON_COLORS=0
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mount -t tmpfs tmpfs /run
modprobe btrfs
modprobe virtio_pci
modprobe virtio_blk
mkdir -p /target /top
if [[ "$(cat /proc/cmdline)" == *lyra.rollback-prepare=1* ]]; then
    mkfs.btrfs -f /dev/vda || exit 1
    mount /dev/vda /top || exit 1
    btrfs subvolume create /top/@ || exit 1
    btrfs subvolume create /top/@var || exit 1
    btrfs subvolume set-default /top/@ || exit 1
    mount /dev/vda /target || exit 1
    cp -a /usr /etc /test /bin /sbin /lib /lib64 /boot /target/ || exit 1
    mkdir -p /target/{var,dev,proc,sys,tmp,run,root}
    mount -o subvol=@var /dev/vda /target/var || exit 1
    cp -a /var/. /target/var/ || exit 1
    mkdir -p /target/var/tmp/zypp.tmp /target/var/log
    mount --rbind /dev /target/dev
    mount -t proc proc /target/proc
    mount -t sysfs sysfs /target/sys
    chroot /target python3 -u /test/rollback_vm.py prepare
    result=$?
    echo "ROLLBACK_PREPARE_EXIT=$result"
    systemctl --force --force poweroff
else
    mount /dev/vda /target || exit 1
    mount -o subvol=@var /dev/vda /target/var || exit 1
    mount -o subvol=@/.snapshots /dev/vda /target/.snapshots || exit 1
    mount --move /dev /target/dev
    mount --move /proc /target/proc
    mount --move /sys /target/sys
    mount --move /run /target/run
    exec switch_root /target /usr/lib/systemd/systemd --system --log-level=warning --log-target=console --unit=lyra-test.target
fi
''', 0o755)
        files = b'\0'.join(str(path.relative_to(vm.root)).encode() for path in vm.root.rglob('*'))+b'\0'
        archive = subprocess.run(['cpio','--null','-o','-H','newc','--owner=0:0','--quiet'],input=files,cwd=vm.root,capture_output=True,check=True)
        initrd = vm.base/'initramfs.gz'
        with gzip.open(initrd, 'wb', compresslevel=1) as stream: stream.write(archive.stdout)
        disk = vm.base/'root.raw'
        with disk.open('wb') as stream: stream.truncate(4*1024**3)
        args.log_dir.mkdir(parents=True, exist_ok=True)
        for phase in ['prepare', 'verify']:
            log = args.log_dir/(phase+'.log')
            cmdline = 'rdinit=/init console=ttyS0 quiet panic=1 selinux=0 lyra.updater-offline-test=1'
            if phase == 'prepare': cmdline += ' lyra.rollback-prepare=1'
            with log.open('w') as stream:
                result = subprocess.run(['qemu-system-x86_64','-accel','tcg','-cpu','max','-smp','2','-m','1536',
                    '-kernel',str(args.kernel.resolve()),'-initrd',str(initrd),'-append',cmdline,
                    '-drive',f'file={disk},format=raw,if=virtio','-display','none','-serial','stdio','-monitor','none','-nic','none','-no-reboot'],
                    stdout=stream,stderr=subprocess.STDOUT,timeout=300)
            content = log.read_text(errors='replace'); print(content[-16000:], flush=True)
            if result.returncode or f'ROLLBACK_{phase.upper()}_EXIT=0' not in content:
                raise SystemExit('rollback VM failed: '+str(log))
        print('PASS: actual A→B failed RPM upgrade, Snapper rollback and cold boot into verified A')
    finally:
        shutil.rmtree(vm.base)


if __name__ == '__main__': main()
