#!/usr/bin/env python3
"""Cold-boot the actual verifier/systemd with controlled health probes, no NIC."""
import argparse
import json
from pathlib import Path
from systemd_vm_support import SystemdVM

REPO = Path(__file__).resolve().parents[1]
ID = '00000000-0000-4000-8000-000000000008'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--kernel', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--baseline-unit', type=Path)
    parser.add_argument('--scenario', choices=['baseline', 'healthy', 'degraded', 'timeout'], required=True)
    parser.add_argument('--log', type=Path, required=True)
    args = parser.parse_args()
    vm = SystemdVM('boot-verifier', 'lyra-boot-verifier-vm')
    for name in ['sleep', 'false', 'true', 'test', 'timeout']:
        vm.tool(name)
    (vm.root/'usr/sbin').symlink_to('bin')
    (vm.root/'sbin').symlink_to('usr/bin')
    vm.binary(args.binary, '/usr/libexec/lyra-upgrade-verify')
    unit = args.baseline_unit or REPO/'packaging/lyra-upgrade-verify.service'
    vm.put('/usr/lib/systemd/system/lyra-upgrade-verify.service', unit.read_text())
    # Only logging is redirected; timeout and ordering are the packaged values.
    vm.put('/etc/systemd/system/lyra-upgrade-verify.service.d/console.conf',
           '[Service]\nStandardOutput=tty\nStandardError=inherit\nTTYPath=/dev/console\n')
    vm.put('/usr/lib/systemd/system/network-online.target', '[Unit]\nDefaultDependencies=no\n')
    vm.put('/etc/passwd', 'root:x:0:0:root:/root:/bin/bash\n')
    vm.put('/etc/group', 'root:x:0:\n')
    vm.put('/test/scenario', args.scenario)
    vm.put('/usr/lib/lyra-os/product-release', "LYRA_VERSION_ID='2.0'\nLYRA_ARCHITECTURE='x86_64'\nLYRA_BUILD_ID='target'\n")
    vm.put('/boot/grub2/grub.cfg', '# VM fixture; GRUB itself is outside this boot-wait gate\n')
    probes = {
        'findmnt': 'echo btrfs',
        'snapper': 'exit 0',
        'mokutil': 'echo "SecureBoot disabled"',
        'rpm': "trap '' TERM\nsleep 300" if args.scenario == 'timeout' else 'exit 0',
        'zypper': '''case "$*" in
  *"lr --details") echo '1 | repo-lyra | Lyra | Yes | Yes | No | 1 | rpm-md | https://fixture.invalid' ;;
  *"locks") echo 'There are no package locks defined.' ;;
  *"packages --orphaned") echo 'No packages found.' ;;
  *"verify"*) exit 0 ;;
  *) exit 99 ;;
esac''',
    }
    for name, body in probes.items():
        vm.put('/usr/bin/'+name, '#!/bin/bash\n'+body+'\n', 0o755)
    source = {'version':'1.0', 'edition':'desktop', 'architecture':'x86_64', 'build_id':'source'}
    target = dict(source, version='2.0', build_id='target')
    state = {'schema_version':1, 'operation_id':ID, 'sequence':1, 'operation':'ReleaseUpgrade',
             'state':'AwaitingReboot', 'source':source, 'target':target, 'plan_sha256':'a'*64,
             'manifest_sha256':'b'*64, 'snapshot_number':8, 'last_completed_step':'offline-apply',
             'error_code':None, 'boot_verification':'Pending',
             'created_at':'2026-09-09T00:00:00Z', 'updated_at':'2026-09-09T00:00:00Z'}
    operation = '/var/lib/lyra-upgrade/operations/'+ID
    vm.put(operation+'/state.json', json.dumps(state), 0o600)
    vm.put(operation+'/manifest.json', '{"sequence":8}', 0o600)
    if args.scenario == 'degraded':
        vm.put('/etc/systemd/system/fixture-failure.service',
               '[Unit]\nDefaultDependencies=no\nBefore=multi-user.target\n[Service]\nType=oneshot\nExecStart=/usr/bin/false\n')
        vm.put('/etc/systemd/system/multi-user.target.d/failure.conf',
               '[Unit]\nWants=fixture-failure.service\nAfter=fixture-failure.service\n')
    try:
        vm.run(args.kernel, args.log, REPO/'tests/boot_verifier_vm.py',
               'lyra-boot-verifier-test', 'LYRA_BOOT_VERIFIER_VM_PASS='+args.scenario)
    finally:
        import shutil
        shutil.rmtree(vm.base)


if __name__ == '__main__':
    main()
