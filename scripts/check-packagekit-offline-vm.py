#!/usr/bin/env python3
"""Real PackageKit/zypp offline transaction across three disposable VM boots.

Copies public installed executables/libraries, never host accounts, RPM state or
repositories. The only RPM updated is a generated inert fixture. No guest NIC.
"""
import argparse
import gzip
import os
from pathlib import Path
import shutil
import subprocess
from systemd_vm_support import SystemdVM

REPO = Path(__file__).resolve().parents[1]


def main():
    os.environ['PATH'] = os.environ.get('PATH', '') + ':/usr/sbin:/sbin'
    p = argparse.ArgumentParser(description=__doc__)
    for name in ['kernel', 'modules-dir', 'offline-binary', 'output-dir']:
        p.add_argument('--'+name, type=Path, required=True)
    p.add_argument('--expect-lyra-failure', action='store_true')
    args = p.parse_args()
    assert args.kernel.is_file() and args.offline_binary.is_file()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    vm = SystemdVM('packagekit-offline', 'opensuse-leap')
    try:
        root = vm.root
        for name in ['cp', 'rm', 'cat', 'chmod', 'sync', 'sleep', 'umount', 'switch_root', 'modprobe',
                     'mkfs.btrfs', 'blkid', 'rpm', 'rpmdb', 'rpmkeys', 'zypper', 'repo2solv',
                     'rpmdb2solv', 'pkcon', 'dbus-daemon', 'journalctl', 'uname', 'sed',
                     'grep', 'gzip', 'xz', 'zstd', 'gpg', 'gpg-agent', 'getent']:
            vm.tool(name)
        for path in ['/usr/libexec/packagekitd', '/usr/libexec/pk-offline-update',
                     '/usr/lib64/packagekit-backend/libpk_backend_zypp.so',
                     '/usr/libexec/polkit-1/polkitd',
                     '/usr/lib/systemd/systemd-journald',
                     '/usr/lib/systemd/system-generators/systemd-system-update-generator']:
            vm.binary(path)
        for path in Path('/usr/lib64/rpm-plugins').glob('*.so'):
            vm.binary(path)
        for source in ['/usr/lib/rpm', '/usr/share/zypp', '/usr/share/polkit-1',
                       '/usr/share/dbus-1', '/usr/share/PackageKit']:
            shutil.copytree(source, root/source.lstrip('/'), dirs_exist_ok=True, symlinks=True)
        for directory in root.rglob('*'):
            if directory.is_dir() and not directory.is_symlink():
                directory.chmod(directory.stat().st_mode | 0o700)
        vm.binary(args.offline_binary, '/usr/libexec/lyra-upgrade-offline')
        for unit in ['packagekit.service', 'packagekit-offline-update.service', 'polkit.service',
                     'system-update.target', 'system-update-cleanup.service', 'system-update-pre.target',
                     'systemd-journald.service', 'systemd-journald.socket', 'reboot.target',
                     'poweroff.target', 'final.target', 'umount.target',
                     'systemd-reboot.service', 'systemd-poweroff.service']:
            source = Path('/usr/lib/systemd/system')/unit
            if source.exists(): vm.put('/usr/lib/systemd/system/'+unit, source.read_text())
        vm.put('/usr/lib/systemd/system/lyra-upgrade-offline.service',
               (REPO/'packaging/lyra-upgrade-offline.service').read_text())
        for unit in ['lyra-upgrade-offline', 'packagekit-offline-update']:
            dest=root/'usr/lib/systemd/system/system-update.target.wants';dest.mkdir(exist_ok=True)
            (dest/(unit+'.service')).symlink_to('../'+unit+'.service')
        vm.put('/etc/systemd/system/lyra-upgrade-offline.service.d/evidence.conf',
               '[Service]\nStandardOutput=tty\nStandardError=inherit\nTTYPath=/dev/console\nExecStopPost=/usr/bin/python3 /test/record.py\n')
        vm.put('/test/record.py', '''import json,os
from pathlib import Path
Path('/test/offline-result.json').write_text(json.dumps({'result':os.environ.get('SERVICE_RESULT'),'status':os.environ.get('EXIT_STATUS')}))
''')
        vm.put('/etc/passwd', 'root:x:0:0:root:/root:/bin/bash\nmessagebus:x:99:99:dbus:/:/bin/false\npolkitd:x:100:100:polkit:/:/bin/false\n')
        vm.put('/etc/group', 'root:x:0:\nmessagebus:x:99:\npolkitd:x:100:\n')
        vm.put('/etc/hosts', '127.0.0.1 localhost\n')
        vm.put('/etc/os-release', 'ID=opensuse-leap\nNAME="openSUSE Leap"\nVERSION_ID=16.1\n')
        vm.put('/etc/machine-id', 'c2dc8ab0a73947c898e9c9808cdce919\n')
        vm.put('/etc/systemd/journald.conf', '[Journal]\nStorage=persistent\n')
        vm.put('/etc/PackageKit/PackageKit.conf', '[Daemon]\nDefaultBackend=zypp\nKeepCache=true\n')
        (root/'etc/mtab').symlink_to('/proc/self/mounts')
        (root/'usr/sbin').symlink_to('bin'); (root/'sbin').symlink_to('usr/bin')
        for directory in ['var/log/journal','var/lib/PackageKit','var/cache/PackageKit',
                          'usr/lib/sysimage/rpm','etc/zypp/repos.d','run/lock','test/repo','newroot']:
            (root/directory).mkdir(parents=True, exist_ok=True)
        (root/'var/lib/rpm').symlink_to('../../usr/lib/sysimage/rpm')
        shutil.copyfile('/usr/share/PackageKit/transactions.db',root/'var/lib/PackageKit/transactions.db')
        vm.put('/etc/systemd/system/dbus.socket', '[Unit]\nDefaultDependencies=no\n[Socket]\nListenStream=/run/dbus/system_bus_socket\nSocketMode=0666\n')
        vm.put('/etc/systemd/system/dbus.service', '[Unit]\nDefaultDependencies=no\nRequires=dbus.socket\nAfter=dbus.socket\n[Service]\nType=notify\nExecStart=/usr/bin/dbus-daemon --system --address=systemd: --nofork --nopidfile --systemd-activation\n')
        # Empty boot scaffolding targets: services under test use their shipped units.
        vm.put('/usr/lib/systemd/system/network-online.target', '[Unit]\nDefaultDependencies=no\n')
        vm.put('/etc/systemd/system/default.target', '[Unit]\nWants=lyra-test.service\nAfter=lyra-test.service\n')
        vm.put('/etc/systemd/system/lyra-test.service', '''[Unit]
Requires=dbus.socket systemd-journald.socket
After=dbus.socket systemd-journald.socket
[Service]
Type=oneshot
ExecStart=/usr/bin/python3 -u /test/guest.py
StandardOutput=tty
StandardError=inherit
TTYPath=/dev/console
TimeoutStartSec=180
''')
        vm.put('/test/guest.py',(REPO/'tests/packagekit_offline_vm.py').read_text())
        vm.put('/test/expected', 'failure' if args.expect_lyra_failure else 'success')
        # Build two inert packages using only this harness's private build root.
        top=vm.base/'rpmbuild';top.mkdir()
        for version in [1,2]:
            spec=top/'fixture.spec';spec.write_text(f'''Name: lyra-offline-fixture
Version: {version}
Release: 1
Summary: Disposable PackageKit offline test
License: MIT
BuildArch: noarch
AutoReqProv: no
%description
Inert text used only by a disposable VM test.
%install
mkdir -p %{{buildroot}}/usr/share/lyra-offline-fixture
echo {version} > %{{buildroot}}/usr/share/lyra-offline-fixture/version
%files
/usr/share/lyra-offline-fixture
''')
            built=subprocess.run(['rpmbuild','-bb','--define',f'_topdir {top}',str(spec)],capture_output=True,text=True)
            if built.returncode:
                raise RuntimeError(built.stdout+'\n'+built.stderr)
            rpm=top/f'RPMS/noarch/lyra-offline-fixture-{version}-1.noarch.rpm'
            shutil.copyfile(rpm,root/('test/old.rpm' if version==1 else 'test/repo/'+rpm.name))
        vm.put('/etc/zypp/repos.d/fixture.repo', '[fixture]\nname=Disposable fixture\nbaseurl=dir:/test/repo\ntype=plaindir\nenabled=1\nautorefresh=0\ngpgcheck=0\n')
        modules=root/'usr/lib/modules'/args.modules_dir.name;modules.mkdir(parents=True)
        (root/'lib').mkdir(exist_ok=True)
        (root/'lib/modules').symlink_to('../usr/lib/modules')
        for source in args.modules_dir.glob('modules.*'):shutil.copyfile(source,modules/source.name)
        for name in ['btrfs','virtio_blk','virtio_pci']:
            output=subprocess.check_output(['modprobe','--show-depends','-S',args.modules_dir.name,name],text=True)
            for line in output.splitlines():
                if line.startswith('insmod '):
                    source=Path(line.split()[1]);dest=modules/source.resolve().relative_to(args.modules_dir.resolve());dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(source,dest)
        vm.put('/init', '''#!/bin/bash
set -eux
export PATH=/usr/bin:/bin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
modprobe virtio_pci
modprobe virtio_blk
modprobe btrfs
test "$(cat /sys/block/vda/serial)" = lyra-pk-test-only
if ! blkid /dev/vda; then
  mkfs.btrfs -f /dev/vda
  mount /dev/vda /newroot
  cp -a /usr /etc /var /test /bin /sbin /lib /lib64 /newroot/
  mkdir -p /newroot/{dev,proc,sys,run,tmp,root}
  chmod 1777 /newroot/tmp
  umount /newroot
fi
mount /dev/vda /newroot
mount --move /dev /newroot/dev
mount --move /proc /newroot/proc
mount --move /sys /newroot/sys
mount -t tmpfs tmpfs /newroot/run
mkdir -p /newroot/run/lock
exec switch_root /newroot /usr/lib/systemd/systemd --system --log-target=console
''',0o755)
        files=b'\0'.join(str(path.relative_to(root)).encode() for path in root.rglob('*'))+b'\0'
        packed=subprocess.run(['cpio','--null','-o','-H','newc','--owner=0:0','--quiet'],cwd=root,input=files,capture_output=True,check=True)
        initrd=vm.base/'initrd.gz'
        with gzip.open(initrd,'wb',compresslevel=1) as stream:stream.write(packed.stdout)
        disk=vm.base/'disk.raw'
        with disk.open('wb') as stream:stream.truncate(4*1024**3)
        for phase in ['prepare','offline','verify']:
            print('Running '+phase,flush=True)
            command=['qemu-system-x86_64','-accel','kvm','-accel','tcg','-cpu','max','-m','2048','-smp','2',
                     '-kernel',str(args.kernel.resolve()),'-initrd',str(initrd),'-append','rdinit=/init console=ttyS0 panic=1 selinux=0 apparmor=0 lyra.packagekit-test=1',
                     '-drive',f'file={disk},format=raw,if=none,id=test','-device','virtio-blk-pci,drive=test,serial=lyra-pk-test-only',
                     '-display','none','-serial','stdio','-monitor','none','-nic','none','-no-reboot']
            log=args.output_dir/(phase+'.log')
            with log.open('w') as stream:result=subprocess.run(command,stdout=stream,stderr=subprocess.STDOUT,timeout=240)
            content=log.read_text(errors='replace')
            marker={'prepare':'LYRA_PACKAGEKIT_PREPARED','offline':'packagekit-offline-update.service: Deactivated successfully.','verify':'LYRA_PACKAGEKIT_VM_PASS'}[phase]
            rebooted = phase == 'verify' or 'reboot: Restarting system' in content
            if result.returncode or marker not in content or not rebooted:
                print(content[-14000:]);raise SystemExit('FAIL '+str(log))
        print('PASS: PackageKit offline transaction and subsequent boot; disposable disk removed',flush=True)
    finally:
        for directory in vm.base.rglob('*'):
            if directory.is_dir() and not directory.is_symlink():
                directory.chmod(directory.stat().st_mode | 0o700)
        shutil.rmtree(vm.base)

if __name__=='__main__':main()
