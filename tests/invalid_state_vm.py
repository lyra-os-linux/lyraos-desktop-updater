"""Only inside the disposable systemd VM; package/filesystem probes are fixtures."""
import json
from pathlib import Path
import subprocess
import time

UNIT = 'lyra-upgrade-verify.service'
ROOT = Path('/var/lib/lyra-upgrade/operations')
ID = '00000000-0000-4000-8000-000000000008'


def ctl(*args):
    return subprocess.run(['systemctl', *args], capture_output=True, text=True, timeout=5)


def main():
    assert Path('/run/lyra-invalid-state-test').exists()
    assert Path('/proc/1/comm').read_text().strip() == 'systemd'
    assert sorted(p.name for p in Path('/sys/class/net').iterdir()) == ['lo']
    scenario = Path('/test/scenario').read_text()
    before = json.loads(Path('/test/before.json').read_text())
    start = time.monotonic()
    while time.monotonic() - start < 30:
        status = ctl('show', UNIT, '--property=ActiveState', '--value').stdout.strip()
        jobs = ctl('list-jobs', '--no-legend', '--no-pager').stdout
        if status in ['inactive', 'failed'] and UNIT not in jobs:
            break
        time.sleep(.2)
    else:
        raise AssertionError('verifier did not finish')
    assert time.monotonic() - start < 30
    result = ctl('show', UNIT, '--property=Result', '--value').stdout.strip()
    exit_code = ctl('show', UNIT, '--property=ExecMainStatus', '--value').stdout.strip()
    after = {str(p.relative_to(ROOT)): p.read_bytes().hex()
             for p in ROOT.rglob('*') if p.is_file()}
    assert set(after) == set(before), 'scanner created or removed state artifacts'
    for name, data in before.items():
        if scenario != 'mixed' or name != ID + '/state.json':
            assert after[name] == data, name
    sequence = Path('/var/lib/lyra-upgrade/last-manifest-sequence')
    if scenario == 'baseline':
        assert result == 'success' and exit_code == '0'
        assert json.loads((ROOT/ID/'state.json').read_text())['state'] == 'AwaitingReboot'
        assert not sequence.exists()
        print('BASELINE_REPRODUCED: invalid entry hides valid pending operation; false service success')
    else:
        assert status == 'failed' and result == 'exit-code' and exit_code == '1', (status, result, exit_code)
        if scenario == 'mixed':
            state = json.loads((ROOT/ID/'state.json').read_text())
            assert state['state'] == 'Completed' and state['boot_verification'] == 'Passed', state
            assert state['sequence'] >= 3 and state['error_code'] is None
            assert sequence.read_text().strip() == '8'
            assert (ROOT/'00000000-0000-4000-8000-000000000011').is_dir()
            assert not list((ROOT/'00000000-0000-4000-8000-000000000011').iterdir())
            print('VALID_OPERATION_COMPLETED: invalid neighbors preserved; service remains failed for unresolved state')
        else:
            assert not sequence.exists()
            print('CORRUPT_PENDING_PRESERVED: no replay sequence, no fabricated state, service failure')
    print('LYRA_INVALID_STATE_VM_PASS=' + scenario, flush=True)


if __name__ == '__main__':
    main()
