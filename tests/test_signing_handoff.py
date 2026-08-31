from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests"))
from test_release_manifest_tool import fixture, release_manifest

SPEC = importlib.util.spec_from_file_location("signing_handoff", ROOT / "scripts/signing-handoff.py")
assert SPEC and SPEC.loader
handoff = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = handoff
SPEC.loader.exec_module(handoff)


class SigningHandoffTests(unittest.TestCase):
    def test_handoff_contains_only_canonical_public_material(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "manifest.json"
            manifest.write_bytes(release_manifest.canonical_bytes(release_manifest.validate(fixture())))
            output = root / "handoff"
            request = handoff.prepare(manifest, output)
            self.assertEqual(set(path.name for path in output.iterdir()), {
                "releases-v1.json", "signing-request.json", "SHA256SUMS"
            })
            self.assertEqual(request["status"], "awaiting-signature")
            self.assertEqual(request["signing_key_fingerprint"], handoff.SIGNING_FINGERPRINT)
            self.assertNotIn("private", json.dumps(request).lower())
            with self.assertRaisesRegex(ValueError, "must not already exist"):
                handoff.prepare(manifest, output)

    def test_noncanonical_manifest_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps(fixture()), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "not canonical"):
                handoff.prepare(manifest, root / "handoff")


if __name__ == "__main__":
    unittest.main()
