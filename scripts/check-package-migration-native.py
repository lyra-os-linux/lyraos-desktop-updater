#!/usr/bin/env python3
"""Exercise production command selection against signed fixture RPMs in a user namespace.

Only scriptless fixture RPMs are applied inside explicit disposable roots. This
qualifies native solver/download/apply behavior, not boot, Polkit or production keys.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

REPO = Path(__file__).resolve().parents[1]


def run(args, *, check=True):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=True,
                            env=dict(os.environ, LC_ALL='C'), timeout=180)
    if check and result.returncode:
        raise RuntimeError(f'{args}: {result.returncode}\n{result.stdout}\n{result.stderr}')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--adapter', type=Path, required=True)
    parser.add_argument('--fixtures', type=Path, help='Reuse previously generated signed public fixtures')
    args = parser.parse_args()
    uid_map = Path('/proc/self/uid_map').read_text().split()
    if os.geteuid() != 0 or uid_map[2] != '1' or uid_map[1] == '0':
        raise SystemExit('Run as a normal user through unshare --user --map-root-user')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    fixtures = args.fixtures.resolve() if args.fixtures else output/'fixtures'
    if not args.fixtures:
        print(run(['python3', REPO/'scripts/check-vendor-policy-native.py', '--output', fixtures, '--export-vm']).stdout, flush=True)
    adapter = args.adapter.resolve()
    results = []
    for scenario, version in [('upgrade-vendor', '2-1'), ('vendor-only', '1-1')]:
        case = output/scenario
        case.mkdir()
        fixture = fixtures/scenario
        with tempfile.TemporaryDirectory(prefix='lyra-migration-native-') as temp:
            private = Path(temp)
            os.environ['ZYPPTMPDIR'] = str(private/'zypp-tmp')
            (private/'zypp-tmp').mkdir()
            os.environ['ZYPP_LOGFILE'] = str(case/'zypper.log')
            root = private/'root'
            repo = private/'repository'
            shutil.copytree(fixture/'raw/fixture', repo)
            for name in ['repomd.xml.asc', 'repomd.xml.key']:
                shutil.copyfile(fixture/name, repo/'repodata'/name)
            filename = (fixture/'candidate-filename.txt').read_text().strip()
            shutil.copyfile(fixture/'candidate.rpm', repo/filename)
            repos = root/'etc/zypp/repos.d'
            repos.mkdir(parents=True)
            (root/'var/tmp').mkdir(parents=True)
            (root/'tmp').mkdir()
            config = f'[fixture]\nname=fixture\nbaseurl=file://{repo}\nenabled=1\nautorefresh=0\nkeeppackages=0\ntype=rpm-md\ngpgcheck=1\nrepo_gpgcheck=1\n'
            (repos/'fixture.repo').write_text(config)
            run(['rpm', '--root', root, '--initdb'])
            run(['rpm', '--root', root, '--import', repo/'repodata/repomd.xml.key'])
            run(['rpm', '--root', root, '--install', '--noscripts', '--nodeps', fixture/'old.rpm'])
            identity = dict(version='1.1', edition='desktop', architecture='x86_64', build_id='lyra-release-1.1')
            manifest = dict(schema_version=1, sequence=8, status='testing',
                valid_from='2026-09-27T00:00:00Z', valid_until='2026-10-04T00:00:00Z',
                source=identity, target=identity, minimum_updater_version='0.2.7', minimum_free_space_bytes=1,
                repositories=[dict(alias='fixture', base_url='https://fixture.invalid/repo/', signing_key_url='https://fixture.invalid/key', signing_key_fingerprint='A'*40, priority=150)],
                allowed_removals=[], lockstep_packages=[],
                allowed_vendor_transitions=[dict(from_='Lyra Fixture A', to='Lyra Fixture B', packages=['lyra-vendor-fixture'])],
                package_migration=[dict(name='lyra-vendor-fixture', architecture='noarch', from_version='1-1', from_vendor='Lyra Fixture A',
                    to_version=version, to_vendor='Lyra Fixture B', repository_alias='fixture', sha256=hashlib.sha256((repo/filename).read_bytes()).hexdigest(), if_installed=False)])
            rule = manifest['allowed_vendor_transitions'][0]
            rule['from'] = rule.pop('from_')
            document = case/'manifest.json'
            document.write_text(json.dumps(manifest))
            if scenario == 'vendor-only':
                rejected = run([adapter, 'arguments', document, root, 'apply'], check=False)
                assert rejected.returncode and 'InvalidPolicy' in rejected.stderr
                assert run(['rpm', '--root', root, '-q', '--queryformat', '%{VENDOR}', 'lyra-vendor-fixture']).stdout == 'Lyra Fixture A'
                results.append(dict(scenario=scenario, result='PASS', rejected='InvalidPolicy', forced_reinstall=False))
                print('PASS vendor-only: identical RPM version refused without forcing replacement', flush=True)
                continue
            common = ['zypper', '--root', root, '--xmlout', '--non-interactive']
            (case/'refresh.xml').write_text(run(common+['refresh']).stdout)
            for mode in ['plan', 'download', 'apply']:
                command = json.loads(run([adapter, 'arguments', document, root, mode]).stdout)
                result = run(common+['--no-refresh']+command)
                xml = case/(mode+'.xml'); xml.write_text(result.stdout)
                if mode != 'apply':
                    changes = json.loads(run([adapter, 'validate', document, root, xml]).stdout)
                    assert len(changes) == 1
                    assert run(['rpm', '--root', root, '-q', '--queryformat', '%{VENDOR}', 'lyra-vendor-fixture']).stdout == 'Lyra Fixture A'
                if mode == 'download':
                    cache = root/'var/cache/zypp/packages'
                    run([adapter, 'payloads', document, root, cache])
                    # Apply must succeed without access to the repository payload.
                    (repo/filename).unlink()
            assert run(['rpm', '--root', root, '-q', '--queryformat', '%{VERSION}-%{RELEASE}|%{VENDOR}', 'lyra-vendor-fixture']).stdout == version+'|Lyra Fixture B'
            assert (repos/'fixture.repo').read_text() == config
            repeated = run([adapter, 'arguments', document, root, 'apply'], check=False)
            assert repeated.returncode and 'AlreadyApplied' in repeated.stderr
            results.append(dict(scenario=scenario, result='PASS', cached_apply=True, repositories_preserved=True, repeat='AlreadyApplied'))
            print(f'PASS {scenario}: native solver, download, exact digest, cached apply and idempotence', flush=True)
    (output/'result.json').write_text(json.dumps(dict(results=results, limits=['No boot or Polkit qualification', 'Fixture signing key, not release key']), indent=2)+'\n')


if __name__ == '__main__':
    main()
