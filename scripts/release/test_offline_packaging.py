"""Validate CFA4 offline packaging policy, never install dependencies."""
import hashlib
import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PREFLIGHT = ROOT / "scripts/release/check-cfa4-package-prereqs.sh"
FLATPAK = ROOT / "packaging/linux/flatpak-bundle.sh"
BUILDER = ROOT / "scripts/release/build-cfa4-four-formats.sh"
SOURCES = ROOT / "packaging/linux/flatpak-sources.py"
FETCH = ROOT / "scripts/release/fetch-flatpak-sources.sh"
BUNDLE_MANIFEST = ROOT / "packaging/linux/flatpak/io.github.ubuntuokos.cfa4.bundle.yml"
HOSTED = ROOT / ".github/workflows/cfa4-four-formats-hosted.yml"
CANDIDATE = ROOT / ".github/workflows/cfa4-release-candidate.yml"


def _sources_module():
    spec = importlib.util.spec_from_file_location("flatpak_sources", SOURCES)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module  # dataclasses look their module up while it loads
    spec.loader.exec_module(module)
    return module


class OfflinePackagingTests(unittest.TestCase):
    def test_scripts_have_valid_bash_syntax(self):
        for path in (PREFLIGHT, FLATPAK, BUILDER, FETCH):
            with self.subTest(path=path):
                subprocess.run(["bash", "-n", str(path)], check=True,
                               capture_output=True, text=True)

    def test_reject_unsupported_rpm_without_touching_host(self):
        result = subprocess.run(
            ["bash", str(PREFLIGHT), "--formats", "rpm"],
            capture_output=True, text=True, check=False,
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


    def test_every_downloaded_bundle_source_is_found_and_pinned(self):
        # flatpak-builder --disable-download only reads its cache (the 11-12 krb5 failure): the
        # parser must see every url of the bundle manifest, each with a 64-hex SHA-256
        sources = _sources_module().parse(BUNDLE_MANIFEST)
        urls = [line for line in BUNDLE_MANIFEST.read_text().splitlines() if line.strip().startswith("url:")]
        self.assertEqual(len(sources), len(urls))
        names = {source.name for source in sources}
        self.assertIn("krb5-1.22.3.tar.gz", names)
        for source in sources:
            self.assertTrue(source.url.startswith("https://"), source.url)
            self.assertRegex(source.sha256, r"^[0-9a-f]{64}$")

    def test_seed_puts_checked_sources_where_flatpak_builder_looks(self):
        module = _sources_module()
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            payload = b"krb5 stand-in"
            sha = hashlib.sha256(payload).hexdigest()
            manifest = tmp / "m.yml"
            manifest.write_text(
                "modules:\n  - name: krb5\n    sources:\n      - type: archive\n"
                "        url: https://example.invalid/dist/krb5-9.tar.gz\n"
                f"        sha256: {sha}\n  - name: app\n    sources:\n      - type: dir\n"
                "        path: stage\n")
            src = tmp / "src"
            src.mkdir()
            run = lambda *args: subprocess.run(  # noqa: E731
                ["python3", str(SOURCES), *map(str, args)], capture_output=True, text=True, check=False)
            missing = run("seed", manifest, src, tmp / "state")
            self.assertEqual(missing.returncode, 1)
            self.assertIn("krb5-9.tar.gz missing", missing.stderr)
            (src / "krb5-9.tar.gz").write_bytes(b"tampered")
            wrong = run("check", manifest, src)
            self.assertEqual(wrong.returncode, 1)
            self.assertIn("the manifest pins", wrong.stderr)
            self.assertFalse((tmp / "state").exists())
            (src / "krb5-9.tar.gz").write_bytes(payload)
            self.assertEqual(run("seed", manifest, src, tmp / "state").returncode, 0)
            seeded = tmp / "state" / "downloads" / sha / "krb5-9.tar.gz"
            self.assertEqual(seeded.read_bytes(), payload)
            self.assertEqual(len(module.parse(manifest)), 1)

    def test_bundle_seeds_the_builder_cache_before_the_offline_build(self):
        content = FLATPAK.read_text()
        self.assertIn("CFA4_FLATPAK_SOURCES", content)
        self.assertLess(content.index("flatpak-sources.py\" seed"),
                        content.index("flatpak-builder --user --disable-download"))
        self.assertIn('--state-dir="$WORK/state"', content)
        self.assertIn('"$WORK/state"', content[:content.index("flatpak-builder --user")])
        self.assertIn("CFA4_FLATPAK_SOURCES", PREFLIGHT.read_text())

    def test_workflows_provide_the_flatpak_sources(self):
        hosted = HOSTED.read_text()
        self.assertIn("CFA4_FLATPAK_SOURCES:", hosted)
        self.assertIn("scripts/release/fetch-flatpak-sources.sh", hosted)
        self.assertLess(hosted.index("fetch-flatpak-sources.sh \""),
                        hosted.index("check-cfa4-package-prereqs.sh"))
        self.assertEqual(CANDIDATE.read_text().count("vars.CFA4_FLATPAK_SOURCES"), 2)
        self.assertNotIn("curl ", FLATPAK.read_text())


if __name__ == "__main__":
    unittest.main()
