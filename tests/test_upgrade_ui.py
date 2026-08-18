from __future__ import annotations

import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
UPGRADE = ROOT / "upgrade"


class UpgradeUiContractTests(unittest.TestCase):
    def test_javascript_is_valid(self) -> None:
        for name in ("app.js", "i18n.js", "errors.js"):
            result = subprocess.run(
                ["node", "--check", str(UPGRADE / "ui" / name)],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_layout_preview_is_explicit_and_does_not_resume_operations(self) -> None:
        rust = (UPGRADE / "src-tauri/src/main.rs").read_text(encoding="utf-8")
        app = (UPGRADE / "ui/app.js").read_text(encoding="utf-8")
        self.assertIn('var_os("LYRA_UPGRADE_LAYOUT_PREVIEW")', rust)
        self.assertIn('if (!await invoke("layout_preview_enabled")) return false', app)
        self.assertIn("if(!enabled)resumeOperation()", app)

    def test_operation_resume_keeps_only_identifiers_in_web_storage(self) -> None:
        app = (UPGRADE / "ui/app.js").read_text(encoding="utf-8")
        self.assertIn('persistedOperationKey="lyra-upgrade-active-operation-v1"', app)
        self.assertIn("operationId:state.operationId,planHash:state.planHash", app)
        self.assertNotIn("localStorage.setItem(persistedOperationKey,JSON.stringify(state)", app)
        self.assertIn('request("Status"', app)

    def test_service_binds_operations_to_authenticated_caller(self) -> None:
        service = (UPGRADE / "service/src/main.rs").read_text(encoding="utf-8")
        self.assertIn('std::env::var("PKEXEC_UID")', service)
        self.assertIn("save_operation_owner", service)
        self.assertIn("operation_owned_by", service)
        self.assertIn('return rejected(request_id, "OPERATION_NOT_FOUND")', service)


if __name__ == "__main__":
    unittest.main()
