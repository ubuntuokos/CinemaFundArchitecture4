#!/usr/bin/env python3
"""Exercise the tarball installer with disposable binaries and installation directories."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import time
import unittest


HERE = Path(__file__).resolve().parent
APP_ID = "io.github.ubuntuokos.cfa4"


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="cfa4-install-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bundle = self.root / "extracted bundle"
        (self.bundle / "bin").mkdir(parents=True)
        self.share = self.bundle / "share"
        (self.share / "applications").mkdir(parents=True)
        shutil.copyfile(HERE / "install.sh", self.bundle / "install.sh")
        shutil.copyfile(
            HERE / f"{APP_ID}.desktop", self.share / "applications" / f"{APP_ID}.desktop"
        )
        for name in ("cfa4", "cfa4-cli"):
            (self.bundle / "bin" / name).write_text(
                '#!/bin/sh\nprintf \'%s\\n\' "$0" "$@" > "$(dirname "$0")/launched"\n'
            )
        for path in ("doc/cfa4/LICENSE-MIT", "icons/hicolor/test", "mime/packages/test.xml"):
            target = self.share / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text("fixture\n")

    def install(self, *args):
        # Run outside the extracted directory to check that the script finds its own files.
        return subprocess.run(
            ["bash", str(self.bundle / "install.sh"), *map(str, args)],
            cwd=self.root, capture_output=True, text=True, timeout=30,
        )

    def test_install_and_update_preserve_other_applications(self):
        prefix = self.root / "user programs"
        other = prefix / "share/applications/other.desktop"
        other.parent.mkdir(parents=True)
        other.write_text("unrelated\n")
        for version in ("first", "updated"):
            (self.bundle / "share/doc/cfa4/LICENSE-MIT").write_text(version)
            result = self.install("--prefix", prefix)
            self.assertEqual(result.returncode, 0, result.stderr)
            for name in ("cfa4", "cfa4-cli"):
                self.assertTrue(os.access(prefix / "bin" / name, os.X_OK))
                self.assertEqual(
                    (prefix / "bin" / name).read_bytes(), (self.bundle / "bin" / name).read_bytes()
                )
            self.assertEqual((prefix / "share/doc/cfa4/LICENSE-MIT").read_text(), version)
            self.assertTrue((prefix / "share/icons/hicolor/test").is_file())
            self.assertTrue((prefix / "share/mime/packages/test.xml").is_file())
            self.assertEqual(other.read_text(), "unrelated\n")
        desktop = prefix / "share/applications" / f"{APP_ID}.desktop"
        self.assertIn(f'Exec="{prefix}/bin/cfa4" %F\n', desktop.read_text())
        if shutil.which("desktop-file-validate"):
            subprocess.run(["desktop-file-validate", str(desktop)], check=True)
        # no Qt GUI in this bundle: no credits entry either
        self.assertFalse((prefix / "share/applications" / f"{APP_ID}.credits.desktop").exists())

    def test_menu_entry_starts_the_qt_gui_when_the_tarball_has_it(self):
        """Owner decision 2026-11-12: the menu entry (and files opened with it) start the Qt 6 GUI;
        a tarball without it keeps the egui editor, so the entry never points at a missing program."""
        prefix = self.root / "menu"
        result = self.install("--prefix", prefix)
        self.assertEqual(result.returncode, 0, result.stderr)
        desktop = prefix / "share/applications" / f"{APP_ID}.desktop"
        self.assertIn(f'Exec="{prefix}/bin/cfa4" %F\n', desktop.read_text())
        self.add_qt_gui("def main():\n    return 0\n")
        result = self.install("--prefix", prefix)
        self.assertEqual(result.returncode, 0, result.stderr)
        entry = desktop.read_text()
        self.assertIn(f'Exec="{prefix}/bin/cfa4-qt" %F\n', entry)
        self.assertIn(f"TryExec={prefix}/bin/cfa4-qt\n", entry)
        if shutil.which("desktop-file-validate"):
            subprocess.run(["desktop-file-validate", str(desktop)], check=True)

    def test_credits_entry_points_at_the_installed_qt_gui(self):
        shutil.copyfile(
            HERE / f"{APP_ID}.credits.desktop", self.share / "applications" / f"{APP_ID}.credits.desktop"
        )
        prefix = self.root / "with qt"
        result = self.install("--prefix", prefix)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((prefix / "share/applications" / f"{APP_ID}.credits.desktop").exists())
        self.add_qt_gui("def main():\n    return 0\n")
        result = self.install("--prefix", prefix)
        self.assertEqual(result.returncode, 0, result.stderr)
        entry = (prefix / "share/applications" / f"{APP_ID}.credits.desktop").read_text()
        self.assertIn(f'Exec="{prefix}/bin/cfa4-qt" %f\n', entry)
        self.assertIn(f"TryExec={prefix}/bin/cfa4-qt\n", entry)
        self.assertIn("MimeType=application/x-cfa4-credits;application/x-cfa4-credits-template;", entry)
        if shutil.which("desktop-file-validate"):
            subprocess.run(["desktop-file-validate", str(prefix / "share/applications" / f"{APP_ID}.credits.desktop")], check=True)

    @unittest.skipUnless(shutil.which("gio"), "gio is needed to exercise the desktop launcher")
    def test_desktop_launch_escapes_paths_and_preserves_file_arguments(self):
        prefix = self.root / 'programs with "quotes" $dollar `tick` \\slash & more'
        result = self.install("--prefix", prefix)
        self.assertEqual(result.returncode, 0, result.stderr)
        document = self.root / "my film.cfa4"
        document.touch()
        desktop = prefix / "share/applications" / f"{APP_ID}.desktop"
        subprocess.run(["gio", "launch", str(desktop), str(document)], check=True, timeout=10)
        launched = prefix / "bin/launched"
        for _ in range(100):
            if launched.exists() and launched.read_text().splitlines() == [
                str(prefix / "bin/cfa4"), str(document)
            ]:
                break
            time.sleep(0.01)
        self.assertEqual(launched.read_text().splitlines(), [str(prefix / "bin/cfa4"), str(document)])

    def add_qt_gui(self, main_body):
        """The real cfa4-qt launcher and a stand-in cfa4_qt package whose main() runs main_body."""
        shutil.copyfile(HERE / "cfa4-qt", self.bundle / "bin/cfa4-qt")
        package = self.bundle / "lib/cfa4/python/cfa4_qt"
        package.mkdir(parents=True)
        (package / "__init__.py").write_text("")
        (package / "__main__.py").write_text(main_body)

    def fake_pyside6(self, prefix, modules=("QtCore", "QtGui", "QtWidgets", "QtNetwork", "QtTest")):
        """A stand-in PySide6 with these modules, first on the installed launcher's path, so the
        host's PySide6 (installed or not) does not matter."""
        fake = prefix / "lib/cfa4/python/PySide6"
        fake.mkdir()
        (fake / "__init__.py").write_text("")
        for module in modules:
            (fake / f"{module}.py").write_text("")

    def run_qt(self, prefix, **env):
        clean = {k: v for k, v in os.environ.items() if k not in ("CFA4_ENGINE", "PYTHONPATH")}
        return subprocess.run(
            ["python3", str(prefix / "bin/cfa4-qt"), "--demo"], cwd=self.root,
            env={**clean, **env}, capture_output=True, text=True, timeout=30, check=False,
        )

    def test_qt_gui_installs_and_starts_with_the_engine_beside_it(self):
        self.add_qt_gui(
            "import json, os, sys\n"
            "def main():\n"
            "    print(json.dumps({'path': sys.path[0], 'engine': os.environ.get('CFA4_ENGINE'),"
            " 'argv': sys.argv[1:], 'module': __file__}))\n"
            "    return 0\n"
        )
        prefix = self.root / "qt prefix"
        for _ in range(2):  # an update replaces the GUI package
            result = self.install("--prefix", prefix)
            self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(os.access(prefix / "bin/cfa4-qt", os.X_OK))
        self.fake_pyside6(prefix)
        started = self.run_qt(prefix)
        self.assertEqual(started.returncode, 0, started.stderr)
        seen = json.loads(started.stdout)
        python = prefix / "lib/cfa4/python"
        self.assertEqual(seen["path"], str(python))
        self.assertEqual(seen["module"], str(python / "cfa4_qt/__main__.py"))
        self.assertEqual(seen["engine"], str(prefix / "bin/cfa4-cli"))
        self.assertEqual(seen["argv"], ["--demo"])
        # an engine named by the user wins
        seen = json.loads(self.run_qt(prefix, CFA4_ENGINE="/opt/other/cfa4-cli").stdout)
        self.assertEqual(seen["engine"], "/opt/other/cfa4-cli")
        # an empty CFA4_ENGINE names no engine: the one beside the launcher still serves
        seen = json.loads(self.run_qt(prefix, CFA4_ENGINE="").stdout)
        self.assertEqual(seen["engine"], str(prefix / "bin/cfa4-cli"))

    def test_qt_gui_without_pyside6_names_the_packages(self):
        # the GUI imports Qt only inside main(): the launcher checks the modules before it
        self.add_qt_gui("def main():\n    raise SystemExit('main() must not run')\n")
        prefix = self.root / "destination"
        self.assertEqual(self.install("--prefix", prefix).returncode, 0)
        self.fake_pyside6(prefix, ("QtCore",))
        started = self.run_qt(prefix)
        self.assertEqual(started.returncode, 1, started.stderr)
        self.assertIn("PySide6.QtGui", started.stderr)
        self.assertIn("python3-pyside6.qtwidgets", started.stderr)
        self.assertNotIn("main() must not run", started.stderr)

    def test_qt_gui_prefers_the_bundled_pyside6(self):
        """The AppImage and the Flatpak carry PySide6 in lib/cfa4/pyside6 (stage-pyside6.py): it
        comes right after the GUI's package, before the host's; a damaged bundle says so instead of
        naming apt packages (docs/qt-parity.md 1.10)."""
        self.add_qt_gui(
            "import json, sys\n"
            "import PySide6\n"
            "def main():\n"
            "    print(json.dumps({'path': sys.path[:2], 'pyside6': PySide6.__file__}))\n"
            "    return 0\n"
        )
        prefix = self.root / "bundle prefix"
        self.assertEqual(self.install("--prefix", prefix).returncode, 0)
        bundled = prefix / "lib/cfa4/pyside6/PySide6"
        bundled.mkdir(parents=True)
        (bundled / "__init__.py").write_text("")
        for module in ("QtCore", "QtGui", "QtWidgets", "QtNetwork", "QtTest"):
            (bundled / f"{module}.py").write_text("")
        started = self.run_qt(prefix)
        self.assertEqual(started.returncode, 0, started.stderr)
        seen = json.loads(started.stdout)
        self.assertEqual(seen["path"], [str(prefix / "lib/cfa4/python"), str(prefix / "lib/cfa4/pyside6")])
        self.assertEqual(seen["pyside6"], str(bundled / "__init__.py"))
        (bundled / "QtWidgets.py").unlink()
        started = self.run_qt(prefix)
        self.assertEqual(started.returncode, 1)
        self.assertIn("bundled in", started.stderr)
        self.assertNotIn("apt install", started.stderr)

    def test_qt_launcher_without_its_package_does_not_install(self):
        shutil.copyfile(HERE / "cfa4-qt", self.bundle / "bin/cfa4-qt")
        prefix = self.root / "destination"
        result = self.install("--prefix", prefix)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("extract the complete tarball", result.stderr)
        self.assertFalse(prefix.exists(), "refused before anything is written")

    def test_ffmpeg_libraries_install_beside_the_binaries_and_replace_older_ones(self):
        ffmpeg = self.bundle / "lib/cfa4/ffmpeg-9.0.2"
        (ffmpeg / "lib").mkdir(parents=True)
        (ffmpeg / "share/doc/ffmpeg").mkdir(parents=True)
        (ffmpeg / "lib/libavcodec.so.62").write_text("new\n")
        (ffmpeg / "share/doc/ffmpeg/BUILDINFO").write_text("FFmpeg 9.0.2\n")
        prefix = self.root / "destination"
        stale = prefix / "lib/cfa4/ffmpeg-9.0.2/lib/libavcodec.so.61"
        stale.parent.mkdir(parents=True)
        stale.write_text("old\n")
        result = self.install("--prefix", prefix)
        self.assertEqual(result.returncode, 0, result.stderr)
        installed = prefix / "lib/cfa4/ffmpeg-9.0.2"
        self.assertEqual((installed / "lib/libavcodec.so.62").read_text(), "new\n")
        self.assertTrue((installed / "share/doc/ffmpeg/BUILDINFO").is_file())
        self.assertFalse(stale.exists(), "a library of an older release stays behind")

    def test_release_package_refuses_without_ffmpeg_for_other_formats(self):
        result = subprocess.run(
            ["bash", str(HERE / "package.sh"), "--skip-build", "--formats", "deb", "--without-ffmpeg"],
            env={**os.environ, "CFA4_ALLOW_LOCAL_PACKAGING": "1"},
            capture_output=True, text=True, timeout=30,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("--without-ffmpeg is for local tarball tests only", result.stderr)

    def test_invalid_arguments_fail_before_writing(self):
        for args in (("--prefix",), ("--prefix", "relative"), ("--prefix", "/"), ("--unknown",),
                     ("--prefix", "///"), ("--prefix", "/tmp/../"), ("--prefix", str(self.bundle)),
                     ("--prefix", str(self.bundle / "nested")),
                     ("--prefix", str(self.root / "bad=path")),
                     ("--prefix", str(self.root / "bad%path")),
                     ("--prefix", str(self.root / "bad\npath"))):
            with self.subTest(args=args):
                self.assertNotEqual(self.install(*args).returncode, 0)

    def test_incomplete_bundle_does_not_install(self):
        (self.bundle / "bin/cfa4-cli").unlink()
        prefix = self.root / "destination"
        result = self.install("--prefix", prefix)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("extract the complete tarball", result.stderr)
        self.assertFalse(prefix.exists())

    def test_help(self):
        self.assertEqual(self.install("--help").returncode, 0)

    def test_release_tarball_includes_executable_installer(self):
        target = self.root / "target"
        binaries = target / "release"
        binaries.mkdir(parents=True)
        for name in ("cfa4", "cfa4-cli"):
            binary = binaries / name
            binary.write_text("#!/bin/sh\necho 'CFA4 fixture'\n")
            binary.chmod(0o755)
        dist = self.root / "dist"
        # stand-in binaries have no crate sources to list, so ship a stand-in notice file
        notices = self.root / "THIRD-PARTY-LICENSES.txt"
        notices.write_text("CFA4 – third-party licence notices (fixture)\n")
        sbom = self.root / "cfa4.cdx.json"
        sbom.write_text('{"bomFormat": "CycloneDX", "specVersion": "1.6"}\n')
        result = subprocess.run(
            # shell-script stand-ins cannot link FFmpeg: the local-test tarball without it
            ["bash", str(HERE / "package.sh"), "--skip-build", "--formats", "tar", "--without-ffmpeg"],
            env={**os.environ, "CARGO_TARGET_DIR": str(target), "DIST": str(dist),
                 "CFA4_ALLOW_LOCAL_PACKAGING": "1", "CFA4_THIRD_PARTY_LICENSES_FILE": str(notices),
                 "CFA4_SBOM_FILE": str(sbom)},
            capture_output=True, text=True, timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        archives = list(dist.glob("*.tar.gz"))
        self.assertEqual(len(archives), 1)
        with tarfile.open(archives[0]) as archive:
            installer = next(m for m in archive.getmembers() if m.name.endswith("/install.sh"))
            self.assertEqual(installer.mode & 0o777, 0o755)
            self.assertEqual(archive.extractfile(installer).read(), (HERE / "install.sh").read_bytes())
            names = archive.getnames()
            qt = next(m for m in archive.getmembers() if m.name.endswith("/bin/cfa4-qt"))
            self.assertEqual(qt.mode & 0o777, 0o755)
            self.assertEqual(archive.extractfile(qt).read(), (HERE / "cfa4-qt").read_bytes())
            for path in ("lib/cfa4/python/cfa4_qt/__main__.py", "lib/cfa4/python/cfa4_qt/i18n/hu.tsv",
                         "lib/cfa4/python/cfa4_qt/i18n/de.tsv",
                         "lib/cfa4/python/cfa4_qt/i18n/fr.tsv.attribution"):
                self.assertTrue(any(n.endswith("/" + path) for n in names), path)
            self.assertFalse([n for n in names if "__pycache__" in n])
            self.assertTrue(any(n.endswith("/share/doc/cfa4/THIRD-PARTY-LICENSES.txt") for n in names), names)
            self.assertTrue(any(n.endswith("/share/doc/cfa4/cfa4.cdx.json") for n in names), names)


if __name__ == "__main__":
    unittest.main()
