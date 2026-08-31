from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("rehearsal_observations", ROOT / "scripts/rehearsal-observations.py")
assert SPEC and SPEC.loader
observer = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = observer
SPEC.loader.exec_module(observer)


class RehearsalObservationTests(unittest.TestCase):
    def observation(self, boot: str, version: str, build: str, completed: bool = False) -> dict:
        return {
            "schema": 1, "status": "observed", "mode": "guest-upgrade-state",
            "installation_uuid": "12345678-1234-4234-8234-123456789abc",
            "boot_id": boot, "session": "installed",
            "release": {"id": "lyra-os", "version_id": version, "edition": "desktop", "architecture": "x86_64", "build_id": build},
            "upgrade": {
                "package_version": "0.2.2-1", "operation_id": "operation",
                "operation_state": "Completed" if completed else None,
                "operation_sequence": 10 if completed else None,
                "source_version": "1.0" if completed else None,
                "target_version": "1.1-beta.1" if completed else None,
                "snapshot_recorded": True if completed else None,
            },
        }

    def test_accepts_only_three_boot_atomic_identity_sequence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            trace = {
                "schema": 1, "status": "in-progress",
                "installation_uuid": "12345678-1234-4234-8234-123456789abc",
                "qemu_launch_count": 3,
                "launches": [{"mode": "installed"}] * 3,
            }
            (root / "trace").write_text(json.dumps(trace), encoding="utf-8")
            observations = [
                self.observation("boot-1", "1.0", "lyra-release-1.0"),
                self.observation("boot-2", "1.1-beta.1", "lyra-release-1.1-beta.1", True),
                self.observation("boot-3", "1.0", "lyra-release-1.0"),
            ]
            (root / "observations").write_text("\n".join(map(json.dumps, observations)), encoding="utf-8")
            result = observer.aggregate(
                root / "trace", root / "observations",
                ("1.0", "lyra-release-1.0"),
                ("1.1-beta.1", "lyra-release-1.1-beta.1"),
            )
            self.assertEqual(result["status"], "observed")
            self.assertEqual(result["phase"], "rollback-observed")

            observations[2]["release"]["build_id"] = "forged"
            (root / "observations").write_text("\n".join(map(json.dumps, observations)), encoding="utf-8")
            with self.assertRaisesRegex(observer.ObservationError, "baseline-target-baseline"):
                observer.aggregate(
                    root / "trace", root / "observations",
                    ("1.0", "lyra-release-1.0"),
                    ("1.1-beta.1", "lyra-release-1.1-beta.1"),
                )

    def test_rejects_reused_boot_and_unknown_fields(self) -> None:
        value = self.observation("boot", "1.0", "lyra-release-1.0")
        value["unexpected"] = True
        with self.assertRaisesRegex(observer.ObservationError, "fields differ"):
            observer.validate_observation(value, value["installation_uuid"])


if __name__ == "__main__":
    unittest.main()
