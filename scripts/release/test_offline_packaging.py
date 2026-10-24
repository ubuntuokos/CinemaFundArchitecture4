"""Validate CFA4 offline packaging policy, never install dependencies."""
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
PREFLIGHT = ROOT / "scripts/release/check-cfa4-package-prereqs.sh"
FLATPAK = ROOT / "packaging/linux/flatpak-bundle.sh"
BUILDER = ROOT / "scripts/release/build-cfa4-four-formats.sh"


class OfflinePackagingTests(unittest.TestCase):
    def test_scripts_have_valid_bash_syntax(self):
        for path in (PREFLIGHT, FLATPAK, BUILDER):
            with self.subTest(path=path):
                subprocess.run(["bash", "-n", str(path)], check=True,
                               capture_output=True, text=True)

    def test_reject_unsupported_rpm_without_touching_host(self):
        result = subprocess.run(
            ["bash", str(PREFLIGHT), "--formats", "rpm"],
            capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("unsupported CFA4 package format: rpm", result.stderr)

    def test_flatpak_does_not_add_remotes_or_download_deps(self):
        content = FLATPAK.read_text()
        self.assertIn("flatpak-builder --user --disable-download", content)
        self.assertNotIn("flatpak remote-add ", content)
        self.assertNotIn("--install-deps-from=", content)
        self.assertIn("check-cfa4-package-prereqs.sh", content)

    def test_flatpak_app_install_requires_explicit_opt_in(self):
        content = FLATPAK.read_text()
        self.assertIn("TEST=0", content)
        self.assertIn("--test) TEST=1", content)
        self.assertIn("CFA4_ALLOW_FLATPAK_INSTALL_TEST", content)
        self.assertIn("--no-pull --no-related", content)

    def test_appimagetool_and_sdk_are_preflighted(self):
        content = PREFLIGHT.read_text()
        for fragment in ("CFA4_APPIMAGETOOL_SHA256", "sha256sum --",
                         "org.freedesktop.Platform", "org.freedesktop.Sdk", "25.08"):
            self.assertIn(fragment, content)
        for forbidden in ("apt-get", "curl ", "wget "):
            self.assertNotIn(forbidden, content)

    def test_preflight_occurs_before_builder(self):
        content = BUILDER.read_text()
        self.assertLess(content.index('check-cfa4-package-prereqs.sh'),
                        content.index('packaging/linux/package.sh'))


if __name__ == "__main__":
    unittest.main()
