"""Run only in the disposable systemd VM; package/boot probes are fixtures."""
import json
from pathlib import Path
import subprocess
import time

UNIT = 'lyra-upgrade-verify.service'
OP = Path('/var/lib/lyra-upgrade/operations/00000000-0000-4000-8000-000000000008')


def ctl(*args):
    return subprocess.run(['systemctl', *args], capture_output=True, text=True, timeout=5)


def main():
    assert Path('/run/lyra-boot-verifier-test').exists()
    assert Path('/proc/1/comm').read_text().strip() == 'systemd'
    assert sorted(p.name for p in Path('/sys/class/net').iterdir()) == ['lo']
    scenario = Path('/test/scenario').read_text()
    started = time.monotonic()
    if scenario == 'baseline':
        # The observer is Type=simple: only the verifier's own boot job blocks
        # StartupFinished. Keep its state pending until the reproduction is proven.
        time.sleep(10)
        status = ctl('show', UNIT, '--property=ActiveState', '--value').stdout.strip()
        jobs = ctl('list-jobs', '--no-legend', '--no-pager').stdout
        assert status == 'activating', status
        assert UNIT in jobs, jobs
        assert ctl('is-system-running').stdout.strip() == 'starting'
        assert json.loads((OP/'state.json').read_text())['state'] == 'AwaitingReboot'
        print('BASELINE_REPRODUCED: verifier boot job waits for StartupFinished', flush=True)
        ctl('stop', '--no-block', UNIT)
    else:
        while time.monotonic()-started < 205:
            status = ctl('show', UNIT, '--property=ActiveState', '--value').stdout.strip()
            state = json.loads((OP/'state.json').read_text())
            if status in ['inactive','failed'] and state['state'] in ['Completed','NeedsRecovery']:
                break
            time.sleep(0.25)
        else:
            raise AssertionError('verifier did not finish within its deadline')
        elapsed = time.monotonic()-started
        assert UNIT not in ctl('list-jobs','--no-legend','--no-pager').stdout
        assert state['sequence'] >= 3, state
        if scenario == 'healthy':
            assert state['state'] == 'Completed', state
            assert state['boot_verification'] == 'Passed', state
            assert ctl('show',UNIT,'--property=Result','--value').stdout.strip() == 'success'
            assert Path('/var/lib/lyra-upgrade/last-manifest-sequence').read_text().strip() == '8'
        else:
            expected = 'POST_BOOT_FAILED_UNITS' if scenario == 'degraded' else 'POST_BOOT_VERIFICATION_TIMEOUT'
            assert state['state'] == 'NeedsRecovery' and state['error_code'] == expected, state
            assert state['boot_verification'] == 'Failed'
            assert not Path('/var/lib/lyra-upgrade/last-manifest-sequence').exists()
            assert status == 'failed'
            if scenario == 'timeout':
                assert 175 <= elapsed < 200, elapsed
                for process in Path('/proc').glob('[0-9]*/comm'):
                    try:
                        assert process.read_text().strip() != 'sleep', str(process)
                    except FileNotFoundError:
                        pass
        print(f'STATE={state["state"]} ERROR={state["error_code"]} ELAPSED={elapsed:.2f}', flush=True)
    print('LYRA_BOOT_VERIFIER_VM_PASS='+scenario, flush=True)


if __name__ == '__main__':
    main()
