from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/release-manifest.py"
SPEC = importlib.util.spec_from_file_location("release_manifest", SCRIPT)
assert SPEC and SPEC.loader
release_manifest = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = release_manifest
SPEC.loader.exec_module(release_manifest)


def fixture() -> dict:
    identity = {
        "version": "1.0",
        "edition": "desktop",
        "architecture": "x86_64",
        "build_id": "controlled-source",
    }
    return {
        "schema_version": 1,
        "sequence": 1,
        "status": "testing",
        "valid_from": "2027-01-01T00:00:00Z",
        "valid_until": "2027-02-01T00:00:00Z",
        "source": identity,
        "target": {**identity, "version": "1.1-beta.1", "build_id": "controlled-target"},
        "minimum_updater_version": "0.2.1",
        "minimum_free_space_bytes": 8 * 1024 * 1024 * 1024,
        "repositories": [
            {
                "alias": "repo-lyra-successor",
                "base_url": "https://example.test/repository/",
                "signing_key_url": "https://example.test/repository.key",
                "signing_key_fingerprint": "A" * 40,
                "priority": 10,
            }
        ],
        "allowed_removals": [],
        "allowed_vendor_transitions": [],
        "lockstep_packages": [["lyra-release", "lyra-upgrade"]],
    }


class ReleaseManifestToolTests(unittest.TestCase):
    def test_controlled_successor_is_canonical_and_deterministic(self) -> None:
        document = release_manifest.validate(fixture())
        first = release_manifest.canonical_bytes(document)
        second = release_manifest.canonical_bytes(json.loads(first))
        self.assertEqual(first, second)
        self.assertTrue(first.endswith(b"\n"))

    def test_unknown_fields_and_unsafe_urls_fail_closed(self) -> None:
        unknown = fixture()
        unknown["command"] = "zypper dup"
        with self.assertRaises(release_manifest.ManifestError):
            release_manifest.validate(unknown)
        unsafe = fixture()
        unsafe["repositories"][0]["base_url"] = "https://user:secret@example.test/?accept=1"
        with self.assertRaises(release_manifest.ManifestError):
            release_manifest.validate(unsafe)

    def test_invalid_window_policy_and_lockstep_fail_closed(self) -> None:
        invalid = fixture()
        invalid["valid_until"] = invalid["valid_from"]
        with self.assertRaises(release_manifest.ManifestError):
            release_manifest.validate(invalid)
        invalid = fixture()
        invalid["target"]["version"] = "0.9"
        with self.assertRaises(release_manifest.ManifestError):
            release_manifest.validate(invalid)
        invalid = fixture()
        invalid["lockstep_packages"] = [["lyra-release", "lyra-release"]]
        with self.assertRaises(release_manifest.ManifestError):
            release_manifest.validate(invalid)

    def test_output_is_new_regular_file_and_never_overwritten(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "releases-v1.json"
            release_manifest.write_new(output, b"{}\n")
            self.assertEqual(output.read_bytes(), b"{}\n")
            with self.assertRaises(release_manifest.ManifestError):
                release_manifest.write_new(output, b"changed\n")


if __name__ == "__main__":
    unittest.main()
