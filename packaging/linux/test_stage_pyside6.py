#!/usr/bin/env python3
"""stage-pyside6.py and AppRun with synthetic PySide6 wheels (small shared libraries built with
cc), so no Qt is downloaded: the libraries only carry the NEEDED entries the script follows.
docs/qt-parity.md 1.10."""

import hashlib
import os
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
STAGE = HERE / "stage-pyside6.py"
MODULES = ("QtCore", "QtGui", "QtWidgets", "QtNetwork", "QtTest", "QtMultimedia", "QtDBus")
LICENCES = Path("/usr/share/common-licenses")


@unittest.skipUnless(shutil.which("cc") and (LICENCES / "LGPL-3").is_file() and (LICENCES / "LGPL-2.1").is_file(),
                     "needs cc and base-files' LGPL texts")
class StagePyside6Tests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="cfa4-stage-pyside6-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.libs = self.root / "libs"
        self.libs.mkdir()
        self.source = self.root / "empty.c"
        self.source.write_text("int cfa4_fixture;\n")

    def library(self, name: str, *needed: str) -> Path:
        out = self.libs / name
        subprocess.run(["cc", "-shared", "-fPIC", "-o", str(out), str(self.source), f"-Wl,-soname,{name}",
                        "-Wl,--no-as-needed", *(str(self.libs / n) for n in needed)], check=True)
        return out

    def wheels(self) -> tuple[Path, Path]:
        """A wheelhouse with the three wheels and a requirements file pinning their hashes."""
        core = self.library("libQt6Core.so.6")
        self.library("libicuuc.so.73")
        gui = self.library("libQt6Gui.so.6", "libQt6Core.so.6")
        unused = self.library("libQt6Quick3D.so.6", "libQt6Gui.so.6")
        module = self.library("QtCore.abi3.so", "libQt6Core.so.6", "libicuuc.so.73")
        xcb = self.library("libqxcb.so", "libQt6Gui.so.6")
        eglfs = self.library("libqeglfs.so", "libQt6Quick3D.so.6")
        house = self.root / "wheels"
        house.mkdir()
        members = {
            "pyside6_essentials": {
                "PySide6/__init__.py": b"",
                "PySide6/support/__init__.py": b"",
                "PySide6/designer": b"#!/bin/sh\n",
                "PySide6/QtQuick.abi3.so": module.read_bytes(),
                **{f"PySide6/{m}.abi3.so": module.read_bytes() for m in MODULES if m != "QtMultimedia"},
                "PySide6/Qt/lib/libQt6Core.so.6": core.read_bytes(),
                "PySide6/Qt/lib/libQt6Core.so": core.read_bytes(),  # a development name: a copy in wheels
                "PySide6/Qt/lib/libQt6Gui.so.6": gui.read_bytes(),
                "PySide6/Qt/lib/libicuuc.so.73": (self.libs / "libicuuc.so.73").read_bytes(),
                "PySide6/Qt/lib/libQt6Quick3D.so.6": unused.read_bytes(),
                "PySide6/Qt/plugins/platforms/libqxcb.so": xcb.read_bytes(),
                "PySide6/Qt/plugins/platforms/libqeglfs.so": eglfs.read_bytes(),
                "PySide6/Qt/plugins/imageformats/libqpdf.so": eglfs.read_bytes(),
                "PySide6/Qt/plugins/designer/libqquickwidget.so": eglfs.read_bytes(),
                "PySide6/Qt/qml/QtQuick/qmldir": b"module QtQuick\n",
                "PySide6/Qt/translations/qtbase_hu.qm": b"qm",
                "PySide6/Qt/translations/qtdeclarative_hu.qm": b"qm",
                "PySide6/include/QtCore/qglobal.h": b"",
            },
            "pyside6_addons": {
                "PySide6/__init__.py": b"",
                "PySide6/QtMultimedia.abi3.so": module.read_bytes(),
                "PySide6/Qt3DCore.abi3.so": module.read_bytes(),
            },
            "shiboken6": {"shiboken6/__init__.py": b"", "shiboken6/Shiboken.abi3.so": module.read_bytes()},
        }
        pins = []
        for stem, files in members.items():
            wheel = house / f"{stem}-6.12.0-cp310-abi3-manylinux_2_34_x86_64.whl"
            with zipfile.ZipFile(wheel, "w") as z:
                for name, data in files.items():
                    z.writestr(name, data)
            digest = hashlib.sha256(wheel.read_bytes()).hexdigest()
            pins.append(f"{stem.replace('_', '-')}==6.12.0 \\\n    --hash=sha256:{'0' * 64} \\\n    --hash=sha256:{digest}")
        requirements = self.root / "requirements.txt"
        requirements.write_text("other==1.0 \\\n    --hash=sha256:" + "1" * 64 + "\n" + "\n".join(pins) + "\n")
        return house, requirements

    def stage(self, house: Path, requirements: Path, *extra: str):
        return subprocess.run(
            ["python3", str(STAGE), "--wheels", str(house), "--dest", str(self.root / "dest"),
             "--doc", str(self.root / "doc"), "--requirements", str(requirements), *extra],
            capture_output=True, text=True, timeout=60, check=False,
        )

    def test_stages_what_the_gui_loads_and_the_libraries_it_needs(self):
        house, requirements = self.wheels()
        result = self.stage(house, requirements)
        self.assertEqual(result.returncode, 0, result.stderr)
        dest = self.root / "dest"
        pyside = dest / "PySide6"
        self.assertEqual(sorted(p.name for p in pyside.glob("*.abi3.so")), sorted(f"{m}.abi3.so" for m in MODULES))
        self.assertTrue((pyside / "__init__.py").is_file() and (pyside / "support/__init__.py").is_file())
        self.assertTrue((dest / "shiboken6/Shiboken.abi3.so").is_file())
        # only the libraries a kept module or plugin needs, each once, under its SONAME
        self.assertEqual(sorted(p.name for p in (pyside / "Qt/lib").iterdir()),
                         ["libQt6Core.so.6", "libQt6Gui.so.6", "libicuuc.so.73"])
        self.assertEqual([p.name for p in (pyside / "Qt/plugins").rglob("*.so")], ["libqxcb.so"])
        self.assertEqual([p.name for p in (pyside / "Qt/translations").iterdir()], ["qtbase_hu.qm"])
        for gone in ("designer", "Qt/qml", "include", "Qt3DCore.abi3.so"):
            self.assertFalse((pyside / gone).exists(), gone)
        doc = self.root / "doc"
        self.assertEqual((doc / "LICENSE.LGPL-3.0").read_bytes(), (LICENCES / "LGPL-3").read_bytes())
        self.assertTrue((doc / "LICENSE.LGPL-2.1").is_file())
        notice = (doc / "PYSIDE6-QT-NOTICE.txt").read_text()
        self.assertIn("pyside6_essentials-6.12.0-cp310-abi3-manylinux_2_34_x86_64.whl  sha256:", notice)
        self.assertIn("download.qt.io", notice)

    def test_licence_texts_from_another_directory(self):
        """The from-source Flatpak's SDK has no /usr/share/common-licenses: `--licences` names the
        directory with the pinned GNU texts; without them nothing is staged."""
        house, requirements = self.wheels()
        texts = self.root / "gnu"
        texts.mkdir()
        (texts / "LGPL-3").write_text("lgpl 3 text\n")
        (texts / "LGPL-2.1").write_text("lgpl 2.1 text\n")
        result = self.stage(house, requirements, "--licences", str(texts))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.root / "doc/LICENSE.LGPL-3.0").read_text(), "lgpl 3 text\n")
        self.assertEqual((self.root / "doc/LICENSE.LGPL-2.1").read_text(), "lgpl 2.1 text\n")
        shutil.rmtree(self.root / "dest")
        (texts / "LGPL-2.1").unlink()
        result = self.stage(house, requirements, "--licences", str(texts))
        self.assertEqual(result.returncode, 1)
        self.assertIn("LGPL-2.1 is missing", result.stderr)
        self.assertFalse((self.root / "dest").exists(), "nothing staged")

    def test_refuses_a_wheel_whose_hash_is_not_pinned(self):
        house, requirements = self.wheels()
        with zipfile.ZipFile(next(house.glob("shiboken6-*.whl")), "a") as z:
            z.writestr("shiboken6/extra.py", b"")
        result = self.stage(house, requirements)
        self.assertEqual(result.returncode, 1)
        self.assertIn("refusing it", result.stderr)
        self.assertFalse((self.root / "dest").exists(), "nothing staged")

    def test_needs_a_local_wheelhouse(self):
        _house, requirements = self.wheels()
        result = self.stage(self.root / "no such wheelhouse", requirements)
        self.assertEqual(result.returncode, 1)
        self.assertIn("never downloaded here", result.stderr)


class AppRunTests(unittest.TestCase):
    """AppRun starts the Qt GUI by default (owner decision 2026-11-12) and egui with --egui or CFA4_GUI=egui."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="cfa4-apprun-")
        self.addCleanup(self.temp.cleanup)
        self.appdir = Path(self.temp.name) / "CFA4.AppDir"
        (self.appdir / "usr/bin").mkdir(parents=True)
        shutil.copyfile(HERE / "AppRun", self.appdir / "AppRun")
        (self.appdir / "AppRun").chmod(0o755)
        (self.appdir / "usr/bin/cfa4").write_text('#!/bin/sh\necho egui "$@"\n')
        (self.appdir / "usr/bin/cfa4").chmod(0o755)
        (self.appdir / "usr/bin/cfa4-qt").write_text('import sys\nprint("qt", *sys.argv[1:])\n')

    def run_app(self, *args: str, **env: str) -> str:
        clean = {k: v for k, v in os.environ.items() if k != "CFA4_GUI"}
        result = subprocess.run([str(self.appdir / "AppRun"), *args], env={**clean, **env},
                                capture_output=True, text=True, timeout=30, check=True)
        return result.stdout.strip()

    def test_qt_by_default_and_egui_on_request(self):
        self.assertEqual(self.run_app("film.fcproj"), "qt film.fcproj")
        self.assertEqual(self.run_app("--qt", "--demo"), "qt --demo")
        self.assertEqual(self.run_app("--demo", CFA4_GUI="qt"), "qt --demo")
        self.assertEqual(self.run_app("--egui", "x"), "egui x")
        self.assertEqual(self.run_app("x", CFA4_GUI="egui"), "egui x")
        self.assertEqual(self.run_app("--egui", "x", CFA4_GUI="qt"), "egui x")
        self.assertEqual(self.run_app("--qt", "x", CFA4_GUI="egui"), "qt x")

    def test_without_python3_the_default_falls_back_to_egui(self):
        """Only sh and the egui binary on PATH: the default still starts; an explicit --qt fails."""
        bindir = Path(self.temp.name) / "bin"
        bindir.mkdir()
        for tool in ("sh", "dirname", "readlink"):
            found = shutil.which(tool)
            self.assertIsNotNone(found, tool)
            (bindir / tool).symlink_to(found)
        env = {"PATH": str(bindir)}
        app = str(self.appdir / "AppRun")
        result = subprocess.run([app, "x"], env=env, capture_output=True, text=True, timeout=30, check=True)
        self.assertEqual(result.stdout.strip(), "egui x")
        self.assertIn("python3 is missing", result.stderr)
        result = subprocess.run([app, "--qt", "x"], env=env, capture_output=True, text=True, timeout=30, check=False)
        self.assertEqual(result.returncode, 1)
        self.assertIn("needs python3", result.stderr)


if __name__ == "__main__":
    unittest.main()
