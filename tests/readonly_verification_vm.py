"""Actual RPM/zypper only inside the disposable systemd VM."""
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ID = '00000000-0000-4000-8000-000000000012'
ROOT = Path('/var/lib/lyra-upgrade/operations')
UNIT = 'lyra-upgrade-verify.service'
PACKAGES = Path('/test/packages')
ENV = dict(os.environ, LC_ALL='C', PATH='/usr/sbin:/usr/bin:/sbin:/bin')


def run(args, check=True):
    result = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=ENV, timeout=30)
    if check and result.returncode:
        raise AssertionError((args, result.returncode, result.stdout.decode(errors='replace'), result.stderr.decode(errors='replace')))
    return result


def setup(scenario):
    for path in ['/usr/lib/sysimage/rpm', '/var/lib/rpm', '/var/cache/zypp', '/var/lib/zypp', '/etc/zypp', str(ROOT), '/test/repo', '/usr/share/lyra-dependency-consumer', '/usr/share/lyra-dependency-provider']:
        p=Path(path)
        if p.is_symlink(): p.unlink()
        elif p.exists(): shutil.rmtree(p)
    Path('/var/lib/lyra-upgrade/last-manifest-sequence').unlink(missing_ok=True)
    Path('/var/tmp/zypp.tmp').mkdir(parents=True,exist_ok=True)
    Path('/var/log').mkdir(parents=True,exist_ok=True)
    repo=Path('/test/repo'); repo.mkdir()
    if scenario != 'remove': shutil.copy2(PACKAGES/'lyra-dependency-provider.rpm',repo/'provider.rpm')
    repos=Path('/etc/zypp/repos.d');repos.mkdir(parents=True)
    (repos/'fixture.repo').write_text('[fixture]\nname=fixture\nbaseurl=file:///test/repo\nenabled=1\nautorefresh=0\ntype=plaindir\ngpgcheck=0\nrepo_gpgcheck=0\npkg_gpgcheck=0\n')
    run(['rpm','--initdb'])
    names=['lyra-dependency-consumer']
    if scenario=='healthy': names.append('lyra-dependency-provider')
    run(['rpm','-i','--nodeps','--noscripts',*[str(PACKAGES/(n+'.rpm')) for n in names]])
    run(['zypper','--non-interactive','refresh'])


def rpm_state():
    # Include the database bytes and package payloads, not just package names.
    paths=[p for p in Path('/usr/lib/sysimage/rpm').rglob('*') if p.is_file() and p.name not in ['.rpm.lock']]
    paths += list(Path('/usr/share').glob('lyra-dependency-*/data'))
    return {'files':{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths},
            'inventory':run(['rpm','-qa','--qf','%{NAME}-%{VERSION}-%{RELEASE}.%{ARCH}\n']).stdout.decode()}


def write_operation():
    source={'version':'1.0','edition':'desktop','architecture':'x86_64','build_id':'source'}
    target=dict(source,version='2.0',build_id='target')
    state={'schema_version':1,'operation_id':ID,'sequence':1,'operation':'ReleaseUpgrade',
           'state':'AwaitingReboot','source':source,'target':target,'plan_sha256':'a'*64,
           'manifest_sha256':'b'*64,'snapshot_number':12,'last_completed_step':'offline-apply',
           'error_code':None,'boot_verification':'Pending','created_at':'2026-09-09T00:00:00Z','updated_at':'2026-09-09T00:00:00Z'}
    operation=ROOT/ID;operation.mkdir(parents=True)
    (operation/'state.json').write_text(json.dumps(state))
    (operation/'manifest.json').write_text('{"sequence":8}')


def main():
    assert Path('/run/lyra-readonly-verification-test').exists()
    assert Path('/proc/1/comm').read_text().strip()=='systemd'
    assert sorted(p.name for p in Path('/sys/class/net').iterdir())==['lo']
    for scenario in ['healthy','install','remove']:
        setup(scenario)
        before=rpm_state()
        result=run(['zypper','--xmlout','--non-interactive','--no-refresh','verify','--dry-run','--details'],check=False)
        after=rpm_state();assert before==after,(scenario,before,after)
        print('ZYPPER_XML '+scenario+' '+str(result.returncode)+' '+base64.b64encode(result.stdout).decode(),flush=True)
    if Path('/test/probe-only').read_text()=='1':
        print('LYRA_READONLY_VERIFICATION_VM_PASS',flush=True);return
    # Reproduce the old verifier's real repair outside its authorized plan.
    setup('install'); write_operation(); before=rpm_state()
    baseline=run(['/test/baseline-verifier'],check=False)
    assert baseline.returncode==0, (baseline.stdout,baseline.stderr)
    after=rpm_state();assert before!=after
    assert 'lyra-dependency-provider-1-1.noarch' in after['inventory']
    assert json.loads((ROOT/ID/'state.json').read_text())['state']=='Completed'
    print('BASELINE_REPRODUCED: post-boot verification installed unplanned provider and marked Completed',flush=True)
    for scenario in ['healthy','install','remove']:
        setup(scenario);write_operation();before=rpm_state()
        run(['systemctl','reset-failed'])
        result=run(['systemctl','start',UNIT],check=False)
        state=json.loads((ROOT/ID/'state.json').read_text())
        after=rpm_state();assert before==after,(scenario,before,after)
        unit_result=run(['systemctl','show',UNIT,'--property=Result','--value']).stdout.decode().strip()
        sequence=Path('/var/lib/lyra-upgrade/last-manifest-sequence')
        if scenario=='healthy':
            assert result.returncode==0 and unit_result=='success', (result.stderr,state,unit_result)
            assert state['state']=='Completed' and state['boot_verification']=='Passed',state
            assert sequence.read_text().strip()=='8'
        else:
            assert result.returncode!=0 and unit_result=='exit-code',(state,unit_result)
            assert state['state']=='NeedsRecovery' and state['boot_verification']=='Failed',state
            assert state['error_code']=='POST_BOOT_DEPENDENCIES_FAILED',state
            assert not sequence.exists()
        print('READONLY_CASE '+json.dumps({'scenario':scenario,'state':state['state'],'error':state['error_code'],'rpmdb_unchanged':True,'payloads_unchanged':True,'inventory':after['inventory']}),flush=True)
    print('LYRA_READONLY_VERIFICATION_VM_PASS',flush=True)


if __name__ == '__main__':main()
