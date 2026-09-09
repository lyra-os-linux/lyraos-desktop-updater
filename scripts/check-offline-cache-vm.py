#!/usr/bin/env python3
"""Boot the real offline worker on a disposable Btrfs disk with no network NIC."""
import argparse
import gzip
from pathlib import Path
import re
import shutil
import subprocess
import sys
import sysconfig
import tempfile

REPO = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['kernel', 'modules-dir', 'fixtures', 'offline-binary', 'planner-binary', 'log']:
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--baseline-binary', type=Path)
    args = parser.parse_args()
    assert all(path.is_file() for path in [args.kernel,args.offline_binary,args.planner_binary])
    with tempfile.TemporaryDirectory(prefix='lyra-offline-cache-vm-') as directory:
        base = Path(directory); root = base / 'root'
        for name in ['usr/bin','etc','var/tmp/zypp.tmp','var/log','dev','proc','sys','tmp','run','target','test']:
            (root / name).mkdir(parents=True,exist_ok=True)
        (root/'bin').symlink_to('usr/bin'); (root/'sbin').symlink_to('usr/bin'); (root/'usr/sbin').symlink_to('bin')
        def copy(source,destination):
            dest=root/str(destination).lstrip('/'); dest.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(source,dest); dest.chmod(0o755)
        def binary(source,destination):
            copy(source,destination)
            linked=subprocess.check_output(['ldd',str(source)],text=True)
            for library in re.findall(r'(?:=>\s+|^\s*)(/[^\s]+)',linked,re.M):
                copy(library,library)
                if '/glibc-hwcaps/' in library:
                    dynamic=subprocess.check_output(['readelf','-d',library],text=True)
                    soname=re.search(r'\(SONAME\).*\[([^]]+)\]',dynamic)
                    if soname:
                        baseline=Path(library.split('/glibc-hwcaps/')[0])/soname[1]
                        copy(baseline,baseline)
        for name in ['bash','mount','umount','findmnt','mkdir','cp','mv','rm','cat','date','sleep','chroot','modprobe',
                     'mkfs.btrfs','btrfs','snapper','rpm','rpmdb','rpmkeys','zypper','repo2solv','rpmdb2solv','gpg','gpg2','gpg-agent',
                     'gzip','xz','zstd','systemctl','getent','stat','head','sed','readlink']:
            source=shutil.which(name)
            if not source: parser.error('missing VM prerequisite: '+name)
            binary(source,'/usr/bin/'+name)
        for library in Path('/usr/lib64/rpm-plugins').glob('*.so'):
            binary(library,library)
        for library in ['/usr/lib64/libnss_files.so.2','/usr/lib64/libnss_dns.so.2']:
            if Path(library).is_file(): binary(library,library)
        (root/'usr/bin/sh').symlink_to('bash')
        binary(sys.executable,'/usr/bin/python3')
        stdlib=Path(sysconfig.get_path('stdlib'))
        shutil.copytree(stdlib,root/str(stdlib).lstrip('/'),dirs_exist_ok=True,
            ignore=shutil.ignore_patterns('__pycache__','site-packages','dist-packages','test','tests','ensurepip','idlelib','tkinter','turtledemo'))
        for extension in stdlib.glob('lib-dynload/*.so'): binary(extension,extension)
        for path in ['/usr/lib/rpm','/usr/share/snapper/config-templates']:
            shutil.copytree(path,root/path.lstrip('/'),dirs_exist_ok=True,symlinks=True)
        copy('/etc/ld.so.cache','/etc/ld.so.cache')
        copy('/usr/share/zoneinfo/UTC','/usr/share/zoneinfo/UTC')
        # These probes/rebuilds are outside this cache/repository qualification.
        # RPM/zypper, the filesystem and Snapper are native in the guest.
        for name,body in [('mokutil','echo "SecureBoot disabled"'),('dracut','echo dracut >> /test/boot-rebuilds'),('grub2-mkconfig','echo grub >> /test/boot-rebuilds')]:
            path=root/'usr/bin'/name; path.write_text('#!/bin/sh\n'+body+'\n'); path.chmod(0o755)
        (root/'etc/passwd').write_text('root:x:0:0:root:/root:/bin/bash\n')
        (root/'etc/group').write_text('root:x:0:\n')
        (root/'etc/nsswitch.conf').write_text('passwd: files\ngroup: files\nhosts: files dns\n')
        (root/'etc/hosts').write_text('127.0.0.1 localhost\n')
        (root/'etc/mtab').symlink_to('../proc/self/mounts')
        (root/'etc/os-release').write_text('ID=opensuse-leap\nVERSION_ID=16.1\n')
        binary(args.offline_binary,'/test/offline-worker')
        binary(args.planner_binary,'/test/prepare-plan')
        if args.baseline_binary: binary(args.baseline_binary,'/test/baseline-worker')
        shutil.copytree(args.fixtures,root/'test/fixture',symlinks=True)
        copy(REPO/'tests/offline_vm_scenarios.py','/test/scenarios.py')
        modules=root/'usr/lib/modules'/args.modules_dir.name; modules.mkdir(parents=True)
        (root/'lib').mkdir(exist_ok=True); (root/'lib/modules').symlink_to('../usr/lib/modules')
        for path in args.modules_dir.glob('modules.*'): shutil.copyfile(path,modules/path.name)
        for name in ['btrfs','virtio_blk','virtio_pci']:
            output=subprocess.check_output(['modprobe','--show-depends','-S',args.modules_dir.name,name],text=True)
            for line in output.splitlines():
                if line.startswith('insmod '):
                    source=Path(line.split()[1]); dest=modules/source.resolve().relative_to(args.modules_dir.resolve())
                    dest.parent.mkdir(parents=True,exist_ok=True); shutil.copyfile(source,dest)
        init=root/'init'
        init.write_text('''#!/bin/bash
export PATH=/usr/bin:/bin
export PYTHONDONTWRITEBYTECODE=1
export PYTHON_COLORS=0
mount -t proc proc /proc || exit 1
mount -t sysfs sysfs /sys || exit 1
mount -t devtmpfs devtmpfs /dev || exit 1
modprobe btrfs || exit 1
modprobe virtio_pci || exit 1
modprobe virtio_blk || exit 1
mkfs.btrfs -f -L lyra-offline-test /dev/vda || exit 1
mount /dev/vda /target || exit 1
cp -a /usr /etc /var /test /bin /sbin /lib /lib64 /target/ || exit 1
mkdir -p /target/{dev,proc,sys,tmp,run,root}
mount --rbind /dev /target/dev || exit 1
mount -t proc proc /target/proc || exit 1
mount -t sysfs sysfs /target/sys || exit 1
chroot /target python3 -u /test/scenarios.py
result=$?
echo "LYRA_OFFLINE_CACHE_VM_EXIT=$result"
systemctl --force --force poweroff
''')
        init.chmod(0o755)
        files=b'\0'.join(str(path.relative_to(root)).encode() for path in root.rglob('*'))+b'\0'
        archive=subprocess.run(['cpio','--null','-o','-H','newc','--owner=0:0','--quiet'],input=files,cwd=root,capture_output=True,check=True)
        initrd=base/'initramfs.cpio.gz'
        with gzip.open(initrd,'wb',compresslevel=1) as output: output.write(archive.stdout)
        disk=base/'root.raw'
        with disk.open('wb') as output: output.truncate(4*1024*1024*1024)
        args.log.parent.mkdir(parents=True,exist_ok=True)
        with args.log.open('w') as log:
            result=subprocess.run(['qemu-system-x86_64','-accel','tcg','-cpu','max','-smp','2','-m','1536',
                '-kernel',str(args.kernel.resolve()),'-initrd',str(initrd),'-append','rdinit=/init console=ttyS0 quiet panic=1 lyra.updater-offline-test=1',
                '-drive',f'file={disk},format=raw,if=virtio','-display','none','-serial','stdio','-monitor','none','-nic','none','-no-reboot'],
                stdout=log,stderr=subprocess.STDOUT,timeout=300)
        content=args.log.read_text(errors='replace'); print(content[-16000:])
        if result.returncode or 'LYRA_OFFLINE_CACHE_VM_EXIT=0' not in content:
            raise SystemExit('offline VM qualification failed: '+str(args.log))
        print('PASS: offline cache/repository scenarios on disposable Btrfs without NIC')


if __name__=='__main__': main()
