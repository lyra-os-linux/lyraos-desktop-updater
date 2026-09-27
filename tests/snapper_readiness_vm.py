#!/usr/bin/env python3
"""Native regression; explicitly run as root in the disposable portal VM only.

Uses the installed candidate's actual socket-activated broker. Temporarily
masks snapperd, then restores it; never relaxes Snapper user permissions.
"""
import json
import os
from pathlib import Path
import subprocess


def main():
    assert os.geteuid() == 0
    assert 'lyra.portal-migration-test=1' in Path('/proc/cmdline').read_text().split()
    assert Path('/sys/block/vda/serial').read_text().strip() == 'lyra-portal-test'
    assert subprocess.check_output(['findmnt', '-n', '-o', 'FSTYPE', '/'], text=True).strip() == 'btrfs'
    service = '/usr/libexec/lyra-upgrade-service'
    request = {'kind': 'ReadRecoveryReadiness', 'protocol_version': 3, 'request_id': 'snapper-native'}
    results = {}

    def query(label):
        result = subprocess.run(
            ['runuser', '-u', 'installeduser', '--', service, '--read-only'],
            input=json.dumps(request) + '\n', capture_output=True, text=True,
            timeout=20, check=True,
        )
        response = json.loads(result.stdout)
        assert response['kind'] == 'RecoveryReadiness'
        assert response['request_id'] == request['request_id']
        results[label] = response['snapper_root_configured']
        return results[label]

    assert query('configured') is True
    subprocess.run(['systemctl', 'mask', '--runtime', '--now', 'snapperd.service'], check=True)
    try:
        assert query('daemon_unavailable') is False
    finally:
        subprocess.run(['systemctl', 'unmask', '--runtime', 'snapperd.service'], check=True)
    assert query('restored') is True
    denied = subprocess.run(
        ['runuser', '-u', 'installeduser', '--', 'snapper', '--config', 'root', 'get-config'],
        capture_output=True, text=True, timeout=10,
    )
    assert denied.returncode != 0
    results['direct_user_access_still_denied'] = True
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
