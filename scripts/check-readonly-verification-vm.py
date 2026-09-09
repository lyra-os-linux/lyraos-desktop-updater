#!/usr/bin/env python3
"""Native RPM/zypper verification in disposable QEMU/systemd, without a NIC."""
import argparse
from pathlib import Path
import shutil
import subprocess
from systemd_vm_support import SystemdVM

REPO = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['kernel', 'binary', 'baseline-binary', 'log']:
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--probe-only', action='store_true')
    args = parser.parse_args()
    vm = SystemdVM('readonly-verify', 'lyra-readonly-verify-vm')
    try:
        for name in ['rpm', 'rpmdb', 'rpmkeys', 'zypper', 'repo2solv', 'rpmdb2solv',
                     'cat', 'test', 'timeout', 'sleep', 'gzip', 'xz', 'zstd', 'getent']:
            vm.tool(name)
        (vm.root/'sbin').symlink_to('usr/bin'); (vm.root/'usr/sbin').symlink_to('bin')
        for source in Path('/usr/lib64/rpm-plugins').glob('*.so'): vm.binary(source)
        for source in ['/usr/lib64/libnss_files.so.2', '/usr/lib64/libnss_dns.so.2']:
            if Path(source).is_file(): vm.binary(source)
        shutil.copytree('/usr/lib/rpm', vm.root/'usr/lib/rpm', dirs_exist_ok=True, symlinks=True)
        vm.put('/etc/passwd', 'root:x:0:0:root:/root:/bin/bash\n')
        vm.put('/etc/group', 'root:x:0:\n'); vm.put('/etc/hosts', '127.0.0.1 localhost\n')
        (vm.root/'etc/mtab').symlink_to('../proc/self/mounts')
        vm.put('/usr/lib/lyra-os/product-release', "LYRA_VERSION_ID='2.0'\nLYRA_ARCHITECTURE='x86_64'\nLYRA_BUILD_ID='target'\n")
        vm.put('/boot/grub2/grub.cfg', '# Fixture; GRUB boot is outside this dependency gate\n')
        for name, body in [('findmnt', 'echo btrfs'), ('snapper', 'exit 0'), ('mokutil', 'echo "SecureBoot disabled"')]:
            vm.put('/usr/bin/'+name, '#!/bin/sh\n'+body+'\n', 0o755)
        vm.binary(args.binary, '/usr/libexec/lyra-upgrade-verify')
        vm.binary(args.baseline_binary, '/test/baseline-verifier')
        vm.put('/usr/lib/systemd/system/lyra-upgrade-verify.service', (REPO/'packaging/lyra-upgrade-verify.service').read_text())
        vm.put('/etc/systemd/system/lyra-upgrade-verify.service.d/console.conf', '[Service]\nStandardOutput=tty\nStandardError=inherit\nTTYPath=/dev/console\n')
        vm.put('/test/probe-only', str(int(args.probe_only)))
        # Build tiny scriptless RPM fixtures; only the guest installs them.
        build = vm.base/'rpmbuild'; (build/'SPECS').mkdir(parents=True); (build/'tmp').mkdir()
        packages = vm.root/'test/packages'; packages.mkdir(parents=True)
        for name in ['lyra-dependency-provider', 'lyra-dependency-consumer']:
            dependency = 'Requires: lyra-dependency-provider >= 1\n' if name.endswith('consumer') else ''
            spec = build/'SPECS/fixture.spec'
            spec.write_text(f'''Name: {name}
Version: 1
Release: 1
Summary: Disposable post-boot dependency fixture
License: MIT
BuildArch: noarch
AutoReqProv: no
{dependency}%description
Scriptless RPM installed exclusively in a disposable VM.
%install
mkdir -p %{{buildroot}}/usr/share/{name}
printf fixture > %{{buildroot}}/usr/share/{name}/data
%files
/usr/share/{name}
''')
            result = subprocess.run(['rpmbuild', '--define', f'_topdir {build}', '--define', f'_tmppath {build/"tmp"}',
                                    '--define', '__os_install_post %{nil}', '-bb', str(spec)], capture_output=True, text=True)
            if result.returncode: raise RuntimeError(result.stdout+result.stderr)
            shutil.copy2(build/'RPMS/noarch'/f'{name}-1-1.noarch.rpm', packages/f'{name}.rpm')
        vm.run(args.kernel, args.log, REPO/'tests/readonly_verification_vm.py',
               'lyra-readonly-verification-test', 'LYRA_READONLY_VERIFICATION_VM_PASS')
    finally:
        shutil.rmtree(vm.base)


if __name__ == '__main__': main()
