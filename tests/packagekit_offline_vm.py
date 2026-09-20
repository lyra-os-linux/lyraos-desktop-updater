"""Guest-only controller; real PackageKit, zypp, RPM and systemd."""
import fcntl
import hashlib
import shutil
import json
from pathlib import Path
import subprocess
import sys

def failed(kind, value, traceback):
    sys.__excepthook__(kind, value, traceback)
    subprocess.run(['systemctl', '--force', '--force', 'poweroff'])
sys.excepthook = failed

assert 'lyra.packagekit-test=1' in Path('/proc/cmdline').read_text().split()
assert Path('/sys/block/vda/serial').read_text().strip()=='lyra-pk-test-only'

def run(argv,**kw):
    print('RUN',argv,flush=True)
    result=subprocess.run(argv,text=True,capture_output=True,timeout=120,**kw)
    print(result.stdout,result.stderr,flush=True)
    if result.returncode:raise RuntimeError((argv,result.returncode))
    return result.stdout

marker=Path('/system-update');root=Path('/var/lib/lyra-upgrade/operations');expected=Path('/test/expected').read_text()
if not Path('/test/prepared').exists():
    run(['rpm','--initdb']);run(['rpm','-i','--nodeps','/test/old.rpm'])
    # Exercise worker startup under another holder's Lyra lock. Foreign requests
    # must not even try taking it, writing state, or touching the foreign files.
    if expected=='success':
        Path('/run/lock').mkdir(exist_ok=True)
        with Path('/run/lock/lyra-upgrade.lock').open('w') as lock:
            fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            for target in [None,'/var/lib/PackageKit/prepared-update','/missing/foreign']:
                if target:marker.symlink_to(target)
                run(['/usr/libexec/lyra-upgrade-offline'])
                assert not root.exists()
                if target:
                    assert str(marker.readlink())==target;marker.unlink()
            run(['/usr/libexec/lyra-upgrade-offline'])
    run(['rpm','-qa'])
    run(['zypper','--non-interactive','refresh'])
    run(['zypper','--no-refresh','list-updates'])
    run(['pkcon','refresh','force'])
    run(['pkcon','--noninteractive','--allow-untrusted','--only-download','update','lyra-offline-fixture'])
    run(['pkcon','offline-trigger'])
    assert marker.is_symlink() and 'PackageKit' in str(marker.readlink())
    assert not root.exists()
    Path('/test/prepared').write_text('prepared')
    print('LYRA_PACKAGEKIT_PREPARED',flush=True)
    run(['systemctl','reboot'])
else:
    candidates=list(Path('/var/lib/PackageKit').glob('offline-update-*'))
    print('PK RESULT FILES',candidates,flush=True)
    results=Path('/var/lib/PackageKit/offline-update-competed').read_text()
    print('PACKAGEKIT_RESULT', results, flush=True)
    assert 'Success=true' in results,results
    assert run(['rpm','-q','--qf','%{VERSION}','lyra-offline-fixture']).strip()=='2'
    assert not marker.is_symlink() and not marker.exists()
    assert not root.exists()
    record=json.loads(Path('/test/offline-result.json').read_text())
    print('LYRA_OFFLINE_RESULT',record,flush=True)
    assert (record['result']=='success')==(expected=='success'),record
    if expected=='success':
        assert record['status']=='0',record
        # A broken Lyra request is still a failure, never a foreign no-op.
        marker.symlink_to(root/'missing')
        failed=subprocess.run(['/usr/libexec/lyra-upgrade-offline'],capture_output=True,text=True)
        assert failed.returncode==1 and 'Lyra offline' in failed.stderr,failed
        assert marker.is_symlink();marker.unlink()
        operation=root/'00000000-0000-4000-8000-000000000001'
        operation.mkdir(parents=True,mode=0o700)
        state_file=operation/'state.json';state_file.write_text('{broken-state')
        # A foreign request ignores even corrupt local state and preserves all
        # foreign data and repository bytes. Missing marker does the same.
        def digest():
            paths=[Path('/var/lib/PackageKit'),Path('/etc/zypp/repos.d'),root]
            return {str(p):hashlib.sha256(p.read_bytes()).hexdigest()
                    for base in paths for p in base.rglob('*') if p.is_file()}
        before=digest()
        marker.symlink_to('/var/lib/PackageKit')
        run(['/usr/libexec/lyra-upgrade-offline'])
        assert str(marker.readlink())=='/var/lib/PackageKit' and digest()==before
        marker.unlink();run(['/usr/libexec/lyra-upgrade-offline']);assert digest()==before
        Path('/run/lock').mkdir(exist_ok=True)
        marker.symlink_to(operation)
        failed=subprocess.run(['/usr/libexec/lyra-upgrade-offline'],capture_output=True,text=True)
        assert failed.returncode==1 and 'cannot load operation state' in failed.stderr,failed
        assert state_file.read_text()=='{broken-state'
        assert not marker.is_symlink()
        # A valid ready Lyra record is recognized and retains failure recovery
        # when its required manifest is missing; no package command is reached.
        source={'version':'1.0','edition':'desktop','architecture':'x86_64','build_id':'baseline'}
        state={'schema_version':1,'operation_id':operation.name,'sequence':1,
               'operation':'ReleaseUpgrade','state':'ReadyToReboot','source':source,
               'target':dict(source,version='1.1',build_id='candidate'),
               'plan_sha256':'a'*64,'manifest_sha256':'b'*64,'snapshot_number':7,
               'recovery':None,'last_completed_step':'downloaded','error_code':None,
               'boot_verification':'Pending','created_at':'2026-09-19T00:00:00Z',
               'updated_at':'2026-09-19T00:00:00Z'}
        state_file.write_text(json.dumps(state));state_file.chmod(0o600)
        marker.symlink_to(operation)
        failed=subprocess.run(['/usr/libexec/lyra-upgrade-offline'],capture_output=True,text=True)
        assert failed.returncode==1,failed
        assert json.loads(state_file.read_text())['state']=='NeedsRecovery'
        assert not marker.is_symlink()
        assert run(['rpm','-q','--qf','%{VERSION}','lyra-offline-fixture']).strip()=='2'
        print('LYRA_OWNERSHIP_AND_RECOVERY_PASS',flush=True)
    print('LYRA_PACKAGEKIT_VM_PASS '+expected,flush=True)
    run(['systemctl','poweroff'])
