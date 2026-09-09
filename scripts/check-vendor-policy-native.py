#!/usr/bin/env python3
"""Generate actual zypper XML in a private root, never update the host.

Run with unshare --user --map-root-user. Only scriptless fixture RPM headers
are installed, using --justdb/--noscripts/--nodeps and an explicit private root.
Every zypper transaction is a dry-run. No network repository is configured.
"""
import argparse
import gzip
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import xml.etree.ElementTree as ET


def run(args, *, env=None):
    result = subprocess.run([str(a) for a in args], stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, env=env, check=False)
    if result.returncode:
        raise RuntimeError(f"{args[0]} exited {result.returncode}: {result.stderr.decode(errors='replace')}\n{result.stdout.decode(errors='replace')}")
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--export-vm', action='store_true', help='Export only fixture RPMs, public keys and the private RPM database for an offline VM')
    args = parser.parse_args()
    # Root capabilities must be confined to a user namespace, not host root.
    uid_map = Path('/proc/self/uid_map').read_text().split()
    if os.geteuid() != 0 or uid_map[2] != '1' or uid_map[1] == '0':
        raise SystemExit('Run as a normal user through unshare --user --map-root-user')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='lyra-vendor-native-') as temp:
        work = Path(temp)
        build = work / 'build'
        (build / 'SPECS').mkdir(parents=True)
        (build / 'tmp').mkdir()
        rpms = {}
        for tag, version, vendor in [('old', '1', 'Lyra Fixture A'), ('same', '1', 'Lyra Fixture B'), ('new', '2', 'Lyra Fixture B')]:
            spec = build / 'SPECS' / 'fixture.spec'
            spec.write_text(f'''Name: lyra-vendor-fixture
Version: {version}
Release: 1
Summary: Isolated vendor policy qualification
License: MIT
Vendor: {vendor}
BuildArch: noarch
AutoReqProv: no
%description
Scriptless, dependency-free package used only in a disposable RPM database.
%install
mkdir -p %{{buildroot}}/usr/share/lyra-vendor-fixture
printf fixture > %{{buildroot}}/usr/share/lyra-vendor-fixture/data
%files
/usr/share/lyra-vendor-fixture
''')
            run(['rpmbuild', '--define', f'_topdir {build}', '--define', f'_tmppath {build / "tmp"}',
                 '--define', '__os_install_post %{nil}', '-bb', spec])
            rpm = work / f'{tag}.rpm'
            rpm.write_bytes((build / 'RPMS/noarch' / f'lyra-vendor-fixture-{version}-1.noarch.rpm').read_bytes())
            rpms[tag] = rpm
        gpg = work / 'gpg'
        gpg.mkdir(mode=0o700)
        run(['gpg', '--homedir', gpg, '--batch', '--passphrase', '', '--quick-generate-key',
             'Lyra Vendor Fixture <fixture@invalid.test>', 'ed25519', 'sign', '0'])
        key = run(['gpg', '--homedir', gpg, '--armor', '--export'])
        if args.export_vm:
            fingerprint = run(['gpg', '--homedir', gpg, '--with-colons', '--list-keys']).decode().split('fpr:::::::::')[1].split(':')[0]
            for rpm in rpms.values():
                run(['rpmsign', '--define', f'_gpg_name {fingerprint}', '--define', f'_gpg_path {gpg}', '--addsign', rpm])

        for scenario, tag, version in [('vendor-only', 'same', '1'), ('upgrade-vendor', 'new', '2')]:
            case = output / scenario
            case.mkdir(exist_ok=False)
            root = work / scenario
            repos = root / 'etc/zypp/repos.d'
            repos.mkdir(parents=True)
            repo = work / f'{scenario}-repo'
            (repo / 'repodata').mkdir(parents=True)
            rpm = repo / f'lyra-vendor-fixture-{version}-1.noarch.rpm'
            rpm.write_bytes(rpms[tag].read_bytes())
            primary = f'''<?xml version="1.0" encoding="UTF-8"?>
<metadata xmlns="http://linux.duke.edu/metadata/common" xmlns:rpm="http://linux.duke.edu/metadata/rpm" packages="1">
<package type="rpm"><name>lyra-vendor-fixture</name><arch>noarch</arch><version epoch="0" ver="{version}" rel="1"/>
<checksum type="sha256" pkgid="YES">{hashlib.sha256(rpm.read_bytes()).hexdigest()}</checksum>
<summary>Isolated vendor fixture</summary><description>Vendor policy qualification</description><packager>Lyra</packager><url/>
<time file="1788912000" build="1788912000"/><size package="{rpm.stat().st_size}" installed="7" archive="0"/>
<location href="{rpm.name}"/><format><rpm:license>MIT</rpm:license><rpm:vendor>Lyra Fixture B</rpm:vendor><rpm:group>System</rpm:group><rpm:buildhost>fixture</rpm:buildhost><rpm:sourcerpm/>
<rpm:provides><rpm:entry name="lyra-vendor-fixture" flags="EQ" epoch="0" ver="{version}" rel="1"/></rpm:provides>
</format></package></metadata>'''.encode()
            compressed = gzip.compress(primary, mtime=0)
            (repo / 'repodata/primary.xml.gz').write_bytes(compressed)
            (case / 'primary.xml').write_bytes(primary)
            repomd = repo / 'repodata/repomd.xml'
            repomd.write_text(f'''<repomd xmlns="http://linux.duke.edu/metadata/repo"><revision>1788912000</revision><data type="primary"><checksum type="sha256">{hashlib.sha256(compressed).hexdigest()}</checksum><open-checksum type="sha256">{hashlib.sha256(primary).hexdigest()}</open-checksum><location href="repodata/primary.xml.gz"/><timestamp>1788912000</timestamp><size>{len(compressed)}</size><open-size>{len(primary)}</open-size></data></repomd>''')
            (repo / 'repodata/repomd.xml.key').write_bytes(key)
            run(['gpg', '--homedir', gpg, '--batch', '--yes', '--armor', '--detach-sign', repomd])
            (repos / 'fixture.repo').write_text(f'[fixture]\nname=fixture\nbaseurl=file://{repo}\nenabled=1\nautorefresh=0\ntype=rpm-md\ngpgcheck=1\nrepo_gpgcheck=1\n')
            run(['rpm', '--root', root, '--initdb'])
            run(['rpm', '--root', root, '--import', repo / 'repodata/repomd.xml.key'])
            run(['rpm', '--root', root, '--install', '--justdb', '--nodeps', '--noscripts', rpms['old']])
            env = dict(os.environ, ZYPP_LOGFILE=str(case / 'zypper.log'), ZYPPTMPDIR=str(work / 'zypp-tmp'), LC_ALL='C')
            (work / 'zypp-tmp').mkdir(exist_ok=True)
            common = ['zypper', '--root', root, '--xmlout', '--non-interactive']
            (case / 'refresh.xml').write_bytes(run(common + ['refresh'], env=env))
            xml = run(common + ['--no-refresh', 'dist-upgrade', '--dry-run', '--details',
                '--no-allow-downgrade', '--no-allow-name-change', '--no-allow-arch-change', '--allow-vendor-change'], env=env)
            (case / 'solver.xml').write_bytes(xml)
            summary = ET.fromstring(xml).find('install-summary')
            if summary is None or summary.find('to-change-vendor') is None:
                raise RuntimeError(f'{scenario}: real zypper did not report vendor change')
            (case / 'summary.xml').write_bytes(b'<stream>' + ET.tostring(summary) + b'</stream>\n')
            query = '<package>%{NAME:xml}%{EPOCHNUM:xml}%{VERSION:xml}%{RELEASE:xml}%|ARCH?{%{ARCH:xml}}:{<string/>}|%|VENDOR?{%{VENDOR:xml}}:{<string/>}|</package>\n'
            (case / 'installed.xml').write_bytes(b'<rpmdb>' + run(['rpm', '--root', root, '-qa', '--queryformat', query]) + b'</rpmdb>\n')
            cache = case / 'raw/fixture/repodata'
            cache.mkdir(parents=True)
            for name in ['repomd.xml', 'primary.xml.gz']:
                (cache / name).write_bytes((root / 'var/cache/zypp/raw/fixture/repodata' / name).read_bytes())
            for name in ['repomd.xml.asc', 'repomd.xml.key']:
                (case / name).write_bytes((repo / 'repodata' / name).read_bytes())

            if args.export_vm:
                import shutil
                shutil.copytree(root / 'usr/lib/sysimage/rpm', case / 'rpmdb')
                shutil.copytree(root / 'var/cache/zypp', case / 'zypp-cache', symlinks=True)
                (case / 'candidate.rpm').write_bytes(rpm.read_bytes())
                (case / 'candidate-filename.txt').write_text(rpm.name + '\n')
                (case / 'old.rpm').write_bytes(rpms['old'].read_bytes())
            # The native dry-run must not have applied the candidate.
            assert run(['rpm', '--root', root, '-q', '--queryformat', '%{VENDOR}', 'lyra-vendor-fixture']) == b'Lyra Fixture A'
            print(f'PASS {scenario}: signed refresh, actual solver XML, unchanged private RPM database', flush=True)
        run(['gpgconf', '--homedir', gpg, '--kill', 'gpg-agent'])
    print(f'Fixtures: {output}')


if __name__ == '__main__':
    main()
