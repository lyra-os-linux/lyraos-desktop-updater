import stat
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PACKAGING = ROOT / "upgrade" / "packaging"


class UpgradePackagingTests(unittest.TestCase):
    def test_obs_source_generator_is_executable_and_reproducible_by_contract(self) -> None:
        script = PACKAGING / "make-obs-sources.sh"
        mode = script.stat().st_mode
        self.assertTrue(mode & stat.S_IXUSR)
        contents = script.read_text(encoding="utf-8")
        for invariant in (
            "cargo vendor --locked",
            "git -C \"$REPO_ROOT\" archive",
            '--mtime="@$SOURCE_EPOCH"',
            "--sort=name",
            "--owner=0",
            "--group=0",
            'rm -f -- "$temporary_archive"',
            "gpg --batch --yes --dearmor",
            "SHA256SUMS",
        ):
            self.assertIn(invariant, contents)
        self.assertIn("clean committed working tree", contents)

    def test_spec_sources_match_generator_outputs(self) -> None:
        spec = (PACKAGING / "lyra-upgrade.spec").read_text(encoding="utf-8")
        self.assertIn("Source0:        %{name}-%{version}.tar.zst", spec)
        self.assertIn("Source1:        vendor.tar.zst", spec)
        self.assertIn("Source2:        build-source.txt", spec)
        self.assertIn("Source3:        release-signing-key.gpg", spec)
        self.assertIn("cargo test --offline --workspace", spec)

    def test_tauri_and_desktop_icons_are_packaged(self) -> None:
        icons = ROOT / "upgrade" / "src-tauri" / "icons"
        for name in ("icon.png", "32x32.png", "128x128.png", "256x256.png", "512x512.png"):
            path = icons / name
            self.assertTrue(path.is_file(), name)
            self.assertGreater(path.stat().st_size, 100, name)
        desktop = (PACKAGING / "org.lyraos.LyraUpgrade.desktop").read_text(
            encoding="utf-8"
        )
        self.assertIn("Icon=org.lyraos.LyraUpgrade", desktop)
        spec = (PACKAGING / "lyra-upgrade.spec").read_text(encoding="utf-8")
        for size in (32, 128, 256, 512):
            self.assertIn(
                f"icons/hicolor/{size}x{size}/apps/org.lyraos.LyraUpgrade.png",
                spec,
            )

    def test_post_boot_verifier_and_offline_worker_are_enabled(self) -> None:
        spec = (PACKAGING / "lyra-upgrade.spec").read_text(encoding="utf-8")
        self.assertIn("system-update.target.wants/lyra-upgrade-offline.service", spec)
        self.assertIn("multi-user.target.wants/lyra-upgrade-verify.service", spec)
        self.assertNotIn("groupadd -r lyra-upgrade", spec)

    def test_runtime_dependencies_cover_fixed_command_allowlist(self) -> None:
        spec = (PACKAGING / "lyra-upgrade.spec").read_text(encoding="utf-8")
        required = {
            line.split(maxsplit=1)[1]
            for line in spec.splitlines()
            if line.startswith("Requires:")
        }
        self.assertTrue(
            {
                "coreutils",
                "curl",
                "dracut",
                "gnupg",
                "grub2",
                "mokutil",
                "polkit",
                "rpm",
                "snapper",
                "systemd",
                "util-linux",
                "zypper",
            }.issubset(required)
        )


if __name__ == "__main__":
    unittest.main()
