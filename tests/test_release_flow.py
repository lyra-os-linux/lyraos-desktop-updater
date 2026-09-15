import subprocess
import unittest
from pathlib import Path

class ReleaseFlowTests(unittest.TestCase):
    def test_release_and_recovery_interactions(self):
        root = Path(__file__).resolve().parents[1]
        result = subprocess.run(['node', '--test', 'tests/release_flow.mjs'], cwd=root, text=True, capture_output=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
