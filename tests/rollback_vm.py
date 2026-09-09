"""Guest only: native RPM/Snapper rollback with persistent state and real systemd boot."""
import copy
import json
from pathlib import Path
import shutil
import sys
import time
from offline_vm_scenarios import OP, FIXTURE, WORKER, OfflineVmTests, assert_vm, bootstrap, command

STATE = OP/'state.json'
SEQUENCE = OP.parent.parent/'last-manifest-sequence'
UNIT = 'lyra-upgrade-verify.service'


def write_state(value):
    STATE.write_text(json.dumps(value)); STATE.chmod(0o600)


def prepare():
    Path("/run/lock").mkdir(parents=True, exist_ok=True)
    bootstrap()
    test = OfflineVmTests(); test.setUp()
    manifest = json.loads((OP/'manifest.json').read_text()); manifest['sequence'] = 8
    (OP/'manifest.json').write_text(json.dumps(manifest))
    Path('/system-update').unlink(); command(['/test/prepare-plan'])
    command(['rpm','--replacepkgs','--nodeps','-U',FIXTURE/'old.rpm'])
    # Source repositories and their cache remain usable after restoration.
    shutil.rmtree('/etc/zypp/repos.d'); shutil.copytree(OP/'repos.d', '/etc/zypp/repos.d')
    shutil.copytree(OP/'cache', '/var/cache/zypp', dirs_exist_ok=True, symlinks=True)
    SEQUENCE.write_text('7\n')
    # Do not include the pending offline marker in the restored system.
    Path('/system-update').unlink()
    number = int(command(['snapper','--no-dbus','-c','root','create','--read-only','--print-number']).stdout.strip())
    value = json.loads(STATE.read_text()); value['snapshot_number'] = number; write_state(value)
    Path('/system-update').symlink_to(OP)
    result = command([WORKER], check=False)
    print('FAILED_UPGRADE:', result.returncode, result.stdout, result.stderr, flush=True)
    assert result.returncode != 0 and test.vendor() == 'Lyra Fixture B'
    value = json.loads(STATE.read_text()); assert value['state'] == 'NeedsRecovery', value
    # The signed fixture RPM changes package payload, not distro branding.
    Path('/usr/lib/lyra-os/product-release').write_text("LYRA_VERSION_ID='2.0'\nLYRA_ARCHITECTURE='x86_64'\nLYRA_BUILD_ID='target'\n")
    source = Path(f'/.snapshots/{number}/snapshot')
    default = command(['btrfs','subvolume','get-default','/']).stdout.split()[1]
    # A pre-contract snapshot must be refused before boot selection changes.
    command(['btrfs','property','set',source,'ro','false'])
    marker = source/'usr/lib/lyra-upgrade/recovery-format'; marker.unlink()
    rejected = command(['/test/schedule-rollback'], check=False)
    assert rejected.returncode and 'SNAPSHOT_RECOVERY_UNSUPPORTED' in rejected.stderr, rejected
    assert command(['btrfs','subvolume','get-default','/']).stdout.split()[1] == default
    marker.write_text('1\n'); command(['btrfs','property','set',source,'ro','true'])
    volatile = Path('/test/volatile-operations')
    shutil.copytree(OP.parent, volatile, symlinks=True)
    command(['mount','--bind',volatile,OP.parent])
    try:
        refused = command(['/test/schedule-rollback'], check=False)
        assert refused.returncode and 'RECOVERY_STATE_NOT_PERSISTENT' in refused.stderr, refused
        assert command(['btrfs','subvolume','get-default','/']).stdout.split()[1] == default
    finally:
        command(['umount',OP.parent])
    print('NEGATIVE_PASS: recovery state inside restored root refused', flush=True)
    result = command(['/test/schedule-rollback'], check=False)
    print('SCHEDULE:' , result.returncode, result.stdout, result.stderr, flush=True)
    assert result.returncode == 0
    scheduled = json.loads(STATE.read_text()); assert scheduled['state'] == 'AwaitingReboot', scheduled
    assert scheduled['source']['version'] == '1.0' and scheduled['target']['version'] == '2.0'
    (OP/'scheduled.json').write_text(json.dumps(scheduled))
    # Interrupted intent cannot execute another Snapper rollback blindly.
    incomplete = copy.deepcopy(scheduled); incomplete['state'] = 'NeedsRecovery'
    incomplete['recovery']['boot_snapshot'] = None; write_state(incomplete)
    selected = command(['btrfs','subvolume','get-default','/']).stdout.split()[1]
    retry = command(['/test/schedule-rollback'], check=False)
    assert retry.returncode and 'ROLLBACK_INTENT_INCOMPLETE' in retry.stderr
    assert command(['btrfs','subvolume','get-default','/']).stdout.split()[1] == selected
    write_state(scheduled)
    print('PREPARED_GOAL:', scheduled['recovery'], flush=True)


def wait_verifier():
    start = time.monotonic()
    while time.monotonic()-start < 205:
        status = command(['systemctl','show',UNIT,'--property=ActiveState','--value']).stdout.strip()
        value = json.loads(STATE.read_text())
        if status in ['inactive','failed'] and value['state'] in ['Completed','NeedsRecovery']:
            return value
        time.sleep(.25)
    raise AssertionError('verifier did not finish')


def verify():
    assert_vm()
    assert Path('/proc/1/comm').read_text().strip() == 'systemd'
    value = wait_verifier()
    print('BOOT_RESULT:', value, flush=True)
    assert value['state'] == 'Completed' and value['last_completed_step'] == 'rollback-verified', value
    assert value['boot_verification'] == 'Passed' and value['recovery']['boot_snapshot']
    assert command(['rpm','-q','--queryformat','%{VERSION}|%{VENDOR}','lyra-vendor-fixture']).stdout == '1|Lyra Fixture A'
    assert SEQUENCE.read_text() == '7\n'
    assert UNIT not in command(['systemctl','list-jobs','--no-legend','--no-pager']).stdout
    print('RESTORED_ROOT:', command(['btrfs','subvolume','show','/']).stdout, flush=True)
    scheduled = json.loads((OP/'scheduled.json').read_text())
    # Reproduce the old verifier's target-vs-source failure on the real restored root.
    legacy = copy.deepcopy(scheduled); del legacy['recovery']; write_state(legacy)
    result = command(['/test/baseline-verifier'], check=False)
    failed = json.loads(STATE.read_text())
    assert result.returncode and failed['error_code'] == 'POST_BOOT_IDENTITY_FAILED', (result, failed)
    print('BASELINE_REPRODUCED:', failed['error_code'], flush=True)
    for label, change, expected in [
        ('wrong source build', lambda s: s['source'].update(build_id='wrong'), 'POST_BOOT_IDENTITY_FAILED'),
        ('wrong clone UUID', lambda s: s['recovery']['boot_snapshot'].update(uuid='aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee'), 'POST_BOOT_ROLLBACK_IDENTITY_FAILED'),
        ('missing legacy goal', lambda s: s.pop('recovery'), 'POST_BOOT_ROLLBACK_IDENTITY_FAILED'),
    ]:
        broken = copy.deepcopy(scheduled); change(broken); write_state(broken)
        result = command(['/usr/libexec/lyra-upgrade-verify'], check=False)
        failed = json.loads(STATE.read_text())
        assert result.returncode and failed['state']=='NeedsRecovery' and failed['error_code']==expected, (label, result, failed)
        assert SEQUENCE.read_text() == '7\n'
        print('NEGATIVE_PASS:', label, expected, flush=True)
    command(['btrfs','subvolume','set-default',str(scheduled['recovery']['source_snapshot']['id']),'/'])
    try:
        write_state(scheduled)
        result = command(['/usr/libexec/lyra-upgrade-verify'], check=False)
        failed = json.loads(STATE.read_text())
        assert result.returncode and failed['error_code'] == 'POST_BOOT_ROLLBACK_IDENTITY_FAILED', failed
        assert SEQUENCE.read_text() == '7\n'
        print('NEGATIVE_PASS: wrong next boot selection', flush=True)
    finally:
        command(['btrfs','subvolume','set-default',str(scheduled['recovery']['boot_snapshot']['id']),'/'])
    write_state(value)
    print('LYRA_ROLLBACK_VM_PASS' , flush=True)


if __name__ == '__main__':
    assert_vm()
    {'prepare': prepare, 'verify': verify}[sys.argv[1]]()
