#!/usr/bin/env python3
"""Cold-boot the verifier with corrupt operation entries; no host execution or NIC."""
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
    
    parser.add_argument('--scenario', choices=['baseline', 'mixed', 'corrupt'], required=True)
    parser.add_argument('--log', type=Path, required=True)
    args = parser.parse_args()
    vm = SystemdVM('invalid-state', 'lyra-invalid-state-vm')
    for name in ['sleep', 'false', 'true', 'test', 'timeout', 'cat']:
        vm.tool(name)
    (vm.root/'usr/sbin').symlink_to('bin')
    (vm.root/'sbin').symlink_to('usr/bin')
    vm.binary(args.binary, '/usr/libexec/lyra-upgrade-verify')
    unit = REPO/'packaging/lyra-upgrade-verify.service'
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
    vm.put('/test/dependencies-healthy.xml', (REPO/'verifier/tests/fixtures/verify-healthy.xml').read_text())
    probes = {
        'findmnt': 'echo btrfs',
        'snapper': 'exit 0',
        'mokutil': 'echo "SecureBoot disabled"',
        'rpm': 'exit 0',
        'zypper': '''case "$*" in
  *"lr --details") echo '1 | repo-lyra | Lyra | Yes | Yes | No | 1 | rpm-md | https://fixture.invalid' ;;
  *"locks") echo 'There are no package locks defined.' ;;
  *"packages --orphaned") echo 'No packages found.' ;;
  *"verify"*) cat /test/dependencies-healthy.xml ;;
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
    # Deliberately mixed directory entries. The original scanner fails even
    # when it encounters the valid candidate before the broken entry.
    root = vm.root/'var/lib/lyra-upgrade/operations'
    if args.scenario == 'corrupt':
        (root/ID/'state.json').write_bytes(b'{"state":"AwaitingReboot","private":"do-not-log')
    else:
        (root/'00000000-0000-4000-8000-000000000011').mkdir()
        broken = root/'00000000-0000-4000-8000-000000000012'
        broken.mkdir(); (broken/'state.json').write_bytes(b'{"state":')
        (root/'unexpected-file').write_bytes(b'private-do-not-log')
        (root/('invalid'+chr(10)+'name')).mkdir()
    # Record exact bytes and paths; the guest must prove damaged entries survive.
    snapshot = {str(p.relative_to(root)): p.read_bytes().hex()
                for p in root.rglob('*') if p.is_file()}
    vm.put('/test/before.json', json.dumps(snapshot))
    try:
        vm.run(args.kernel, args.log, REPO/'tests/invalid_state_vm.py',
               'lyra-invalid-state-test', 'LYRA_INVALID_STATE_VM_PASS='+args.scenario)
        content = args.log.read_text(errors='replace')
        if args.scenario != 'baseline':
            expected = 1 if args.scenario == 'corrupt' else 4
            assert content.count('POST_BOOT_STATE_ENTRY_INVALID entry=') == expected
            assert f'POST_BOOT_STATE_SCAN_INCOMPLETE invalid_entries={expected}' in content
            assert 'do-not-log' not in content
            if args.scenario == 'mixed':
                assert 'entry="invalid\\nname"' in content
            else:
                assert f'entry="{ID}" reason=state-validation-failed' in content

    finally:
        import shutil
        shutil.rmtree(vm.base)


if __name__ == '__main__':
    main()
