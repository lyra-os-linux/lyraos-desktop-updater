"""Run only inside the disposable VM; real RPM/zypper/Btrfs, no network NIC."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import unittest

ID='00000000-0000-4000-8000-000000000006'
OP=Path('/var/lib/lyra-upgrade/operations')/ID
FIXTURE=Path('/test/fixture')
WORKER='/test/offline-worker'


def command(args, *, check=True):
    result=subprocess.run([str(arg) for arg in args],text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=60,
        env={'PATH':'/usr/bin:/bin','LC_ALL':'C'})
    if check and result.returncode:
        raise RuntimeError(f'{args}: exit {result.returncode}\n{result.stdout}\n{result.stderr}')
    return result


def tree_digest(path):
    result={}
    for item in sorted(path.rglob('*')):
        if item.is_symlink(): result[str(item.relative_to(path))]=('link',os.readlink(item))
        elif item.is_file(): result[str(item.relative_to(path))]=hashlib.sha256(item.read_bytes()).hexdigest()
    return result


def assert_vm():
    assert 'lyra.updater-offline-test=1' in Path('/proc/cmdline').read_text().split()
    assert command(['findmnt','-n','-o','FSTYPE','/']).stdout.strip()=='btrfs'
    assert sorted(path.name for path in Path('/sys/class/net').iterdir())==['lo']


def bootstrap():
    assert_vm()
    Path('/etc/snapper/configs').mkdir(parents=True,exist_ok=True)
    Path('/etc/sysconfig').mkdir(parents=True,exist_ok=True)
    Path('/etc/sysconfig/snapper').write_text('SNAPPER_CONFIGS=""\n')
    command(['snapper','--no-dbus','-c','root','create-config','/'])
    snapshot=command(['snapper','--no-dbus','-c','root','create','--read-only','--print-number']).stdout.strip()
    assert int(snapshot)>0
    Path('/test/snapshot-number').write_text(snapshot+'\n')
    gpg=Path('/tmp/key-inspection'); gpg.mkdir(mode=0o700)
    lines=command(['gpg','--homedir',gpg,'--batch','--with-colons','--show-keys',FIXTURE/'repomd.xml.key']).stdout.splitlines()
    fingerprint=next(line.split(':')[9] for line in lines if line.startswith('fpr:'))
    Path('/test/fingerprint').write_text(fingerprint)


class OfflineVmTests(unittest.TestCase):
    def setUp(self):
        assert_vm()
        marker=Path('/system-update')
        if marker.is_symlink(): marker.unlink()
        for path in [Path('/usr/lib/sysimage/rpm'),Path('/var/lib/lyra-upgrade'),Path('/var/cache/zypp'),Path('/etc/zypp')]:
            if path.exists(): shutil.rmtree(path)
        shutil.copytree(FIXTURE/'rpmdb','/usr/lib/sysimage/rpm',symlinks=True)
        Path('/var/lib').mkdir(parents=True,exist_ok=True)
        compat=Path('/var/lib/rpm')
        if not compat.exists(): compat.symlink_to('../../usr/lib/sysimage/rpm')
        OP.mkdir(parents=True,mode=0o700)
        shutil.copytree(FIXTURE/'zypp-cache',OP/'cache',symlinks=True)
        # Reuse exactly the staged cache hierarchy, including raw and solv.
        (OP/'repos.d').mkdir(); (OP/'keys').mkdir()
        shutil.copyfile(FIXTURE/'repomd.xml.key',OP/'keys/fixture.asc')
        config=f'[fixture]\nname=fixture\nbaseurl=https://offline.invalid/repo\nenabled=1\nautorefresh=0\nkeeppackages=1\ntype=rpm-md\ngpgcheck=1\nrepo_gpgcheck=1\ngpgkey=file://{OP}/keys/fixture.asc\npriority=99\n'
        (OP/'repos.d/fixture.repo').write_text(config)
        packages=OP/'cache/packages/fixture'; packages.mkdir(parents=True,exist_ok=True)
        self.payload=packages/(FIXTURE/'candidate-filename.txt').read_text().strip()
        shutil.copyfile(FIXTURE/'candidate.rpm',self.payload)
        Path('/etc/zypp/repos.d').mkdir(parents=True)
        Path('/etc/zypp/repos.d/source.repo').write_text('[old-source]\nname=old-source\nbaseurl=https://source.invalid/old\nenabled=1\nautorefresh=0\ngpgcheck=1\n')
        release=Path('/usr/lib/lyra-os/product-release'); release.parent.mkdir(parents=True,exist_ok=True)
        release.write_text("LYRA_VERSION_ID='1.0'\nLYRA_ARCHITECTURE='x86_64'\nLYRA_BUILD_ID='fixture'\n")
        manifest={'schema_version':1,'sequence':1,'status':'testing','valid_from':'2026-09-09T00:00:00Z','valid_until':'2026-12-31T00:00:00Z',
            'source':{'version':'1.0','edition':'desktop','architecture':'x86_64','build_id':'fixture'},
            'target':{'version':'2.0','edition':'desktop','architecture':'x86_64','build_id':'target'},
            'minimum_updater_version':'0.2.3','minimum_free_space_bytes':1048576,
            'repositories':[{'alias':'fixture','base_url':'https://offline.invalid/repo','signing_key_url':'https://offline.invalid/key',
                'signing_key_fingerprint':Path('/test/fingerprint').read_text().strip(),'priority':99}],
            'allowed_removals':[],'allowed_vendor_transitions':[{'from':'Lyra Fixture A','to':'Lyra Fixture B'}],'lockstep_packages':[]}
        (OP/'manifest.json').write_text(json.dumps(manifest))
        for path in [Path('/test/boot-rebuilds'),Path('/run/zypp.pid')]:
            if path.exists(): path.unlink()
        command(['rpm','--verifydb'])
        print(command(['/test/prepare-plan']).stdout,flush=True)
        self.before=tree_digest(Path('/etc/zypp/repos.d'))
        self.ready=(OP/'state.json').read_bytes()
        self.assertEqual(self.vendor(),'Lyra Fixture A')

    def vendor(self):
        return command(['rpm','-q','--queryformat','%{VENDOR}','lyra-vendor-fixture']).stdout

    def assert_recoverable(self,result):
        self.assertNotEqual(result.returncode,0,result.stdout+result.stderr)
        self.assertEqual(tree_digest(Path('/etc/zypp/repos.d')),self.before)
        self.assertEqual(self.vendor(),'Lyra Fixture A')
        self.assertFalse(Path('/system-update').is_symlink())
        self.assertEqual(json.loads((OP/'state.json').read_text())['state'],'NeedsRecovery')
        self.assertFalse(Path('/test/boot-rebuilds').exists())
        self.assertFalse(list(Path('/etc/zypp').glob('repos.d.lyra-*')))

    def assert_applied(self,result):
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        self.assertEqual(self.vendor(),'Lyra Fixture B')
        self.assertEqual(command(['rpm','-q','--queryformat','%{VERSION}','lyra-vendor-fixture']).stdout,'2')
        self.assertEqual(json.loads((OP/'state.json').read_text())['state'],'AwaitingReboot')
        self.assertFalse(Path('/system-update').is_symlink())
        self.assertTrue(Path('/etc/zypp/repos.d/fixture.repo').is_file())
        backup=Path('/etc/zypp')/('repos.d.lyra-backup-'+ID)
        self.assertEqual(tree_digest(backup),self.before)
        self.assertEqual(Path('/test/boot-rebuilds').read_text().splitlines(),['dracut','grub'])

    def test_boot_without_network_or_global_cache(self):
        self.assertFalse(Path('/var/cache/zypp').exists())
        self.assert_applied(command([WORKER],check=False))

    def test_poisoned_global_cache_is_ignored(self):
        global_cache=Path('/var/cache/zypp'); (global_cache/'raw/old-source/repodata').mkdir(parents=True)
        (global_cache/'raw/old-source/repodata/repomd.xml').write_text('BROKEN OLD METADATA')
        before=tree_digest(global_cache)
        self.assert_applied(command([WORKER],check=False))
        self.assertEqual(tree_digest(global_cache),before)

    def test_stale_prepared_metadata_preserves_active_repositories(self):
        primary=OP/'cache/raw/fixture/repodata/primary.xml.gz'; primary.write_bytes(b'corrupt')
        self.assert_recoverable(command([WORKER],check=False))

    def test_missing_prepared_metadata_preserves_active_repositories(self):
        (OP/'cache/raw/fixture/repodata/repomd.xml').unlink()
        self.assert_recoverable(command([WORKER],check=False))

    def test_preflight_failure_and_repeated_attempt_preserve_source(self):
        # Make only the factual filesystem/Snapper gate fail, after a valid plan.
        config=Path('/etc/snapper/configs/root'); content=config.read_bytes(); config.unlink()
        try: self.assert_recoverable(command([WORKER],check=False))
        finally: config.write_bytes(content)
        # Running again without a prepared marker must not mutate anything.
        self.assert_recoverable(command([WORKER],check=False))
        # A fresh explicit readiness decision can reuse the intact staged cache.
        (OP/'state.json').write_bytes(self.ready); Path('/system-update').symlink_to(OP)
        self.assert_applied(command([WORKER],check=False))

    def test_missing_payload_cannot_publish_target_repositories(self):
        self.payload.unlink()
        self.assert_recoverable(command([WORKER],check=False))

    def test_new_lock_changes_the_confirmed_plan(self):
        Path('/etc/zypp/locks').write_text('type: package\nmatch_type: glob\ncase_sensitive: on\nsolvable_name: future-package-*\n\n')
        result=command([WORKER],check=False)
        self.assert_recoverable(result)
        self.assertIn('PlanChanged',result.stderr)

    def test_disabled_source_third_party_is_not_a_target_repository(self):
        disabled=Path('/etc/zypp/repos.d/third-party.repo')
        disabled.write_text('[disabled-third-party]\nbaseurl=https://unreachable.invalid/repo\nenabled=0\nautorefresh=1\ngpgcheck=1\n')
        self.before=tree_digest(Path('/etc/zypp/repos.d'))
        Path('/system-update').unlink()
        command(['/test/prepare-plan'])
        plan=json.loads((OP/'plan.json').read_text())
        self.assertEqual([repo['alias'] for repo in plan['repositories']],['fixture'])
        self.assertEqual(plan['source']['version'],'1.0')
        self.assertEqual(plan['target']['version'],'2.0')
        self.assert_applied(command([WORKER],check=False))

    def test_manifest_space_floor_survives_offline_revalidation(self):
        floor=1536*1024*1024
        manifest=json.loads((OP/'manifest.json').read_text())
        manifest['minimum_free_space_bytes']=floor
        (OP/'manifest.json').write_text(json.dumps(manifest))
        Path('/system-update').unlink()
        command(['/test/prepare-plan'])
        plan=json.loads((OP/'plan.json').read_text())
        self.assertEqual(plan['required_bytes'],floor)
        self.assert_applied(command([WORKER],check=False))

    @unittest.skipUnless(Path('/test/baseline-worker').exists(),'baseline supplied only for regression reproduction')
    def test_previous_worker_cannot_use_a_prepared_transaction_without_global_cache(self):
        result=command(['/test/baseline-worker'],check=False)
        self.assertNotEqual(result.returncode,0,'baseline unexpectedly accepted the prepared cache')
        self.assertEqual(self.vendor(),'Lyra Fixture A')
        print('BASELINE_REPRODUCED:',result.stderr.strip(),flush=True)


if __name__=='__main__':
    bootstrap()
    unittest.main(verbosity=2)
