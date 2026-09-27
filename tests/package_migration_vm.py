"""Guest-only signed maintenance through service, offline worker and boot verifier.

The guest trusts an ephemeral manifest key/HTTPS certificate. Authenticated UID
is supplied to the root service as a fixture; interactive Polkit is not qualified.
"""
from datetime import datetime, timedelta, timezone
import functools
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import ssl
import subprocess
import sys
import threading

BASE = Path('/var/lib/lyra-upgrade')
FIXTURE = Path('/test/fixture')
WEB = Path('/test/web')
SERVICE = '/test/service'
SEQUENCE = BASE/'last-manifest-sequence'


def command(args, check=True, **kwargs):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=True,
        timeout=160, env=dict(PATH='/usr/bin:/bin', LC_ALL='C', PKEXEC_UID='1000'), **kwargs)
    if check and result.returncode:
        raise AssertionError(f'{args}: {result.returncode}\n{result.stdout}\n{result.stderr}')
    return result


def rpc(kind, **fields):
    payload = dict(kind=kind, protocol_version=3, request_id=kind, **fields)
    result = command([SERVICE], input=json.dumps(payload)+'\n')
    value = json.loads(result.stdout)
    print('RPC', kind, value.get('kind'), value.get('error_code', ''), flush=True)
    return value


def state():
    op = BASE/'operations'/(BASE/'test-operation-id').read_text()
    return op, json.loads((op/'state.json').read_text())


def identity():
    return command(['rpm', '-q', '--queryformat', '%{VERSION}-%{RELEASE}|%{VENDOR}', 'lyra-vendor-fixture']).stdout


def repositories():
    return {p.name: p.read_text() for p in Path('/etc/zypp/repos.d').glob('*.repo')}


def online():
    for path in ['/etc/snapper/configs', '/etc/sysconfig', '/etc/zypp/repos.d', '/run/lock', '/var/tmp/zypp.tmp', '/var/log', '/etc/ssl', str(BASE)]:
        Path(path).mkdir(parents=True, exist_ok=True)
    Path('/etc/sysconfig/snapper').write_text('SNAPPER_CONFIGS=""\n')
    command(['snapper', '--no-dbus', '-c', 'root', 'create-config', '/'])
    command(['rpm', '--initdb'])
    command(['rpm', '--import', FIXTURE/'repomd.xml.key'])
    command(['rpm', '--install', '--noscripts', '--nodeps', FIXTURE/'old.rpm'])
    shutil.copytree(FIXTURE/'raw/fixture', WEB/'repo')
    for name in ['repomd.xml.asc', 'repomd.xml.key']:
        shutil.copyfile(FIXTURE/name, WEB/'repo/repodata'/name)
    filename = (FIXTURE/'candidate-filename.txt').read_text().strip()
    shutil.copyfile(FIXTURE/'candidate.rpm', WEB/'repo'/filename)
    shutil.copyfile(FIXTURE/'repomd.xml.key', WEB/'key.asc')
    Path('/etc/lyra-upgrade').mkdir()
    Path('/etc/lyra-upgrade/channel').write_text('testing\n')
    Path('/etc/lyra-upgrade/testing-manifest-base-url').write_text('https://fixture.invalid:8443/')
    Path('/usr/lib/lyra-os').mkdir(parents=True, exist_ok=True)
    Path('/usr/lib/lyra-os/product-release').write_text("LYRA_VERSION_ID='1.1'\nLYRA_EDITION='desktop'\nLYRA_ARCHITECTURE='x86_64'\nLYRA_BUILD_ID='lyra-release-1.1'\n")
    SEQUENCE.write_text('7\n')
    command(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', '/test/tls.key', '-out', '/etc/ssl/ca-bundle.pem',
             '-days', '2', '-subj', '/CN=fixture.invalid', '-config', '/dev/null', '-addext', 'subjectAltName=DNS:fixture.invalid'])
    Path('/etc/ssl/certs').mkdir(exist_ok=True)
    shutil.copyfile('/etc/ssl/ca-bundle.pem', '/etc/ssl/certs/ca-certificates.crt')
    Path('/var/lib/ca-certificates').mkdir(parents=True, exist_ok=True)
    shutil.copyfile('/etc/ssl/ca-bundle.pem', '/var/lib/ca-certificates/ca-bundle.pem')
    command(['openssl', 'rehash', '/etc/ssl/certs'])
    signing = Path('/tmp/manifest-key'); signing.mkdir(mode=0o700)
    command(['gpg', '--homedir', signing, '--batch', '--passphrase', '', '--quick-generate-key', 'VM fixture <vm@invalid.test>', 'ed25519', 'sign', '0'])
    public = subprocess.check_output(['gpg', '--homedir', str(signing), '--export'])
    Path('/usr/share/lyra-upgrade').mkdir(parents=True, exist_ok=True)
    Path('/usr/share/lyra-upgrade/release-signing-key.gpg').write_bytes(public)
    key_info = command(['gpg', '--homedir', signing, '--batch', '--with-colons', '--show-keys', WEB/'key.asc']).stdout
    fingerprint = next(line.split(':')[9] for line in key_info.splitlines() if line.startswith('fpr:'))
    now = datetime.now(timezone.utc)
    product = dict(version='1.1', edition='desktop', architecture='x86_64', build_id='lyra-release-1.1')
    document = dict(schema_version=1, sequence=8, status='testing',
        valid_from=(now-timedelta(minutes=5)).strftime('%Y-%m-%dT%H:%M:%SZ'), valid_until=(now+timedelta(days=1)).strftime('%Y-%m-%dT%H:%M:%SZ'),
        source=product, target=product, minimum_updater_version='0.2.7', minimum_free_space_bytes=1024**3,
        repositories=[dict(alias='fixture', base_url='https://fixture.invalid:8443/repo/', signing_key_url='https://fixture.invalid:8443/key.asc', signing_key_fingerprint=fingerprint, priority=99)],
        allowed_removals=[], lockstep_packages=[], allowed_vendor_transitions=[{'from': 'Lyra Fixture A', 'to': 'Lyra Fixture B', 'packages': ['lyra-vendor-fixture']}],
        package_migration=[dict(name='lyra-vendor-fixture', architecture='noarch', from_version='1-1', from_vendor='Lyra Fixture A', to_version='2-1', to_vendor='Lyra Fixture B',
            repository_alias='fixture', sha256=hashlib.sha256((WEB/'repo'/filename).read_bytes()).hexdigest(), if_installed=False)])
    # Deliberately omit the trailing newline: persisted signature must remain valid.
    (WEB/'releases-v1.json').write_text(json.dumps(document, separators=(',', ':')))
    command(['gpg', '--homedir', signing, '--batch', '--armor', '--detach-sign', WEB/'releases-v1.json'])
    command(['gpgconf', '--homedir', signing, '--kill', 'gpg-agent'])
    shutil.rmtree(signing)
    command(['ip', 'link', 'set', 'lo', 'up'])
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 8443), functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(WEB)))
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain('/etc/ssl/ca-bundle.pem', '/test/tls.key')
    server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        command(['curl', '--fail', '--verbose', 'https://fixture.invalid:8443/key.asc'])
        Path('/etc/zypp/repos.d/fixture.repo').write_text('[fixture]\nname=fixture\nbaseurl=https://fixture.invalid:8443/repo/\nenabled=1\nautorefresh=0\ngpgcheck=1\nrepo_gpgcheck=1\npriority=99\n')
        command(['zypper', '--non-interactive', 'refresh'])
        (BASE/'test-repositories.json').write_text(json.dumps(repositories()))
        offer = rpc('CheckRelease'); assert offer['kind'] == 'ReleaseOffer', offer
        plan = rpc('PlanReleaseUpgrade', manifest_sha256=offer['manifest_sha256'])
        assert plan['kind'] == 'Plan', plan
        assert plan['plan']['operation'] == 'PackageMigration'
        assert plan['plan']['source'] == plan['plan']['target'] == product
        assert plan['plan']['reboot_required'] and len(plan['plan']['package_changes']) == 1
        start = dict(operation_id=plan['operation_id'], plan_sha256=plan['plan_sha256'], planned=plan['planned'])
        rejected = rpc('Start', confirmed=False, **start)
        assert rejected.get('error_code') == 'CONFIRMATION_REQUIRED', rejected
        accepted = rpc('Start', confirmed=True, **start)
        assert accepted['kind'] == 'Accepted', accepted
        (BASE/'test-operation-id').write_text(plan['operation_id'])
        op, value = state()
        assert value['state'] == 'ReadyToReboot', value
        assert value['snapshot_number'] and Path('/system-update').resolve() == op
        assert identity() == '1-1|Lyra Fixture A'
        assert (op/'manifest.signed.json').read_bytes() == (WEB/'releases-v1.json').read_bytes()
        assert repositories() == json.loads((BASE/'test-repositories.json').read_text())
        print('ONLINE_PASS: HTTPS signed offer, confirmation, production staging and snapshot', flush=True)
    finally:
        server.shutdown(); server.server_close()


def offline():
    op, saved = state()
    assert saved['state'] == 'ReadyToReboot'
    # Each negative starts from the same disposable staged fixture. The worker
    # itself must fail closed and remove only its own offline marker.
    for label, path in [('signature', op/'manifest.signed.json'), ('payload', next((op/'cache/packages').rglob('*.rpm')))]:
        original = path.read_bytes(); path.write_bytes(original+b'changed')
        result = command(['/test/offline-worker'], check=False)
        _, failed = state()
        assert result.returncode and failed['state'] == 'NeedsRecovery', (label, result, failed)
        assert identity() == '1-1|Lyra Fixture A'
        assert SEQUENCE.read_text() == '7\n' and not Path('/system-update').is_symlink()
        path.write_bytes(original)
        (op/'state.json').write_text(json.dumps(saved))
        Path('/system-update').symlink_to(op)
        print('NEGATIVE_PASS:', label, result.stderr.strip(), flush=True)
    result = command(['/test/offline-worker'])
    _, value = state()
    assert value['state'] == 'AwaitingReboot', (result, value)
    assert identity() == '2-1|Lyra Fixture B' and SEQUENCE.read_text() == '7\n'
    assert not Path('/system-update').is_symlink()
    assert not Path('/test/boot-rebuilds').exists()
    assert repositories() == json.loads((BASE/'test-repositories.json').read_text())
    assert Path('/usr/lib/lyra-os/product-release').read_text().startswith("LYRA_VERSION_ID='1.1'")
    if Path('/test/scenario').read_text() == 'rollback':
        # Controlled post-application damage must be detected at next boot.
        command(['rpm', '--upgrade', '--oldpackage', '--noscripts', '--nodeps', FIXTURE/'old.rpm'])
    print('OFFLINE_PASS: authenticated cache, exact transaction, same identity and repositories', flush=True)


def verify(rollback=False):
    assert Path('/proc/1/comm').read_text().strip() == 'systemd'
    result = command(['/usr/libexec/lyra-upgrade-verify'], check=False)
    op, value = state()
    if not rollback and Path('/test/scenario').read_text() == 'rollback':
        assert result.returncode and value['state'] == 'NeedsRecovery', (result, value)
        assert value['error_code'] == 'POST_BOOT_INVENTORY_FAILED', value
        assert SEQUENCE.read_text() == '7\n'
        scheduled = rpc('AcknowledgeRecovery', operation_id=value['operation_id'], recovery_action='Rollback')
        assert scheduled['kind'] == 'Status' and scheduled['state'] == 'AwaitingReboot', {k: v for k, v in scheduled.items() if k != 'events'}
        _, value = state()
        assert value['recovery']['boot_snapshot'] and value['state'] == 'AwaitingReboot', value
        print('RECOVERY_SCHEDULED:', json.dumps(value['recovery']), flush=True)
        return
    assert result.returncode == 0 and value['state'] == 'Completed', (result, value)
    assert value['boot_verification'] == 'Passed'
    assert identity() == ('1-1|Lyra Fixture A' if rollback else '2-1|Lyra Fixture B')
    assert SEQUENCE.read_text() == ('7\n' if rollback else '8\n')
    assert repositories() == json.loads((BASE/'test-repositories.json').read_text())
    if rollback:
        assert value['last_completed_step'] == 'rollback-verified'
    print('VERIFIED_PASS:', json.dumps(dict(rollback=rollback, state=value['state'], identity=identity(), sequence=SEQUENCE.read_text().strip())), flush=True)


if __name__ == '__main__':
    assert 'lyra.package-migration-test=1' in Path('/proc/cmdline').read_text().split()
    assert Path('/sys/block/vda/serial').read_text().strip() == 'lyra-migration-test'
    assert command(['findmnt', '-n', '-o', 'FSTYPE', '/']).stdout.strip() == 'btrfs'
    assert sorted(p.name for p in Path('/sys/class/net').iterdir()) == ['lo']
    {'online': online, 'offline': offline, 'verify': verify, 'rollback': lambda: verify(True)}[sys.argv[1]]()
