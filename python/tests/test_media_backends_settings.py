"""Settings ▸ Media & Playback ▸ Media Backends (ADR 0005) and the restricted codecs of the Ubuntu
model (docs/licensing.md §7): the report model, the CLI contract, the page, and the menu items
that open it. The Qt parts run when PySide6 is installed (offscreen), the end-to-end part when a
built `cfa4-cli` exists (`CFA4_CLI` or target/debug).
"""
import os
import subprocess
import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from cfa4_devtools.settings_backend import (
    MEDIA_BACKENDS_PAGE,
    MediaBackends,
    Schema,
    SettingsBackendError,
    run_load,
    run_media_backends,
)

REPORT = {
    "backends": [
        {"id": "builtin", "name": "CFA4 engine", "role": "own", "compiledIn": True, "available": True,
         "enabled": True, "canDisable": False},
        {"id": "ffmpeg", "name": "FFmpeg", "compiledIn": True, "available": True, "enabled": True,
         "canDisable": True, "version": "9.0.2", "license": "GPL version 3 or later",
         "restrictedPackage": "cfa4-restricted-extras",
         "restricted": [{"name": "libx264", "purpose": "H.264 software encoding (x264)", "available": False},
                        {"name": "libx265", "purpose": "HEVC software encoding (x265)", "available": True},
                        {"purpose": "no name"}, "junk"]},
        {"id": "gstreamer", "name": "GStreamer", "compiledIn": False, "available": False, "enabled": True,
         "canDisable": True, "problem": "not compiled in"},
        "junk", {"name": "no id"},
    ],
}


class Model(unittest.TestCase):
    def test_order_status_and_restricted(self):
        m = MediaBackends.parse(REPORT)
        self.assertEqual([b.id for b in m.backends], ["builtin", "ffmpeg", "gstreamer"])
        self.assertEqual([b.status for b in m.backends], ["In use", "In use", "Not part of this build"])
        self.assertEqual([r.name for _b, r in m.restricted], ["libx264", "libx265"])
        self.assertEqual(m.install_command(), "sudo apt install cfa4-restricted-extras")

    def test_switched_off_and_nothing_missing(self):
        raw = {"backends": [{"id": "ffmpeg", "compiledIn": True, "available": True, "enabled": False,
                             "restrictedPackage": "cfa4-restricted-extras",
                             "restricted": [{"name": "libx264", "available": True}]}]}
        m = MediaBackends.parse(raw)
        self.assertEqual(m.backends[0].status, "Available, switched off")
        self.assertIsNone(m.install_command())  # all restricted codecs installed: no command

    def test_hostile_reports(self):
        for raw in (None, [], {"backends": "x"}, {}):
            with self.assertRaises(SettingsBackendError):
                MediaBackends.parse(raw)
        m = MediaBackends.parse({"backends": [{"id": "x", "compiledIn": "yes", "available": 1, "version": 9}]})
        self.assertEqual(m.backends[0].status, "Not part of this build")  # only real booleans count
        self.assertEqual(m.backends[0].version, "")


class Cli(unittest.TestCase):
    def test_argv(self):
        with TemporaryDirectory() as d:
            cli = Path(d) / "cfa4-cli"
            cli.write_text("#!/bin/sh\nexit 0\n")
            cli.chmod(0o755)
            done = subprocess.CompletedProcess([], 0, b'{"backends": []}', b"")
            with patch("cfa4_devtools.settings_backend.subprocess.run", return_value=done) as p:
                self.assertEqual(run_media_backends(cli, Path("/d")).backends, ())
            self.assertEqual(p.call_args[0][0], [str(cli), "--compact", "--data-dir", "/d", "media-backends"])


def built_cli():
    env = os.environ.get("CFA4_CLI")
    root = Path(__file__).resolve().parents[2]
    for candidate in ([Path(env)] if env else []) + [root / "target/debug/cfa4-cli", root / "target/release/cfa4-cli"]:
        if candidate.is_file():
            return candidate
    return None


@unittest.skipUnless(built_cli(), "needs a built cfa4-cli (CFA4_CLI or target/debug)")
class RealEngine(unittest.TestCase):
    def test_page_and_report(self):
        cli = built_cli()
        with TemporaryDirectory() as d:
            schema, _prefs = run_load(cli, Path(d))
            media = next(g for g in schema.groups if g.id == "media")
            self.assertIn(MEDIA_BACKENDS_PAGE, [p.id for p in media.pages])
            self.assertEqual(schema.page_keys(MEDIA_BACKENDS_PAGE), {"media.ffmpegBackend", "media.gstreamerBackend"})
            m = run_media_backends(cli, Path(d))
            self.assertEqual(m.backends[0].id, "builtin")
            self.assertEqual({b.id for b in m.backends}, {"builtin", "ffmpeg", "gstreamer"})


def qt_available():
    try:
        from PySide6 import QtWidgets  # noqa: F401
    except ImportError:
        return False
    return True


SCHEMA = {
    "groups": [{"id": "media", "title": "Media & Playback"}],
    "categories": [{"id": MEDIA_BACKENDS_PAGE, "title": "Media Backends", "group": "media", "fields": [
        {"key": "media.ffmpegBackend", "label": "Use FFmpeg", "wired": True, "value": True, "kind": {"type": "bool"}},
        {"key": "media.gstreamerBackend", "label": "Use GStreamer", "wired": True, "value": True, "kind": {"type": "bool"}},
    ]}],
}


@unittest.skipUnless(qt_available(), "needs PySide6")
class QtPage(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
        from PySide6.QtWidgets import QApplication
        cls.app = QApplication.instance() or QApplication([])

    def test_switches_status_and_restricted_codecs(self):
        from cfa4_devtools.qt_settings import SettingsDialog
        from PySide6.QtWidgets import QCheckBox, QLabel, QTableWidget

        dialog = SettingsDialog(None, page=MEDIA_BACKENDS_PAGE)
        dialog._loaded((Schema.parse(SCHEMA), {"media": {"ffmpegBackend": True, "gstreamerBackend": True}}))
        self.assertEqual(dialog.current_page(), MEDIA_BACKENDS_PAGE)
        dialog.findChild(QCheckBox, "settings.media.ffmpegBackend").setChecked(False)
        self.assertEqual(dialog.changes(), {"media.ffmpegBackend": False})
        dialog._backends_loaded(MediaBackends.parse(REPORT))
        listing = dialog.findChild(QTableWidget, "settings.mediaBackends.list")
        self.assertEqual(listing.rowCount(), 3)
        self.assertEqual(listing.item(1, 1).text(), "FFmpeg")
        restricted = dialog.findChild(QTableWidget, "settings.mediaBackends.restricted.list")
        self.assertEqual(restricted.rowCount(), 2)
        install = dialog.findChild(QLabel, "settings.mediaBackends.restricted.install")
        self.assertIn("sudo apt install cfa4-restricted-extras", install.text())
        dialog._backends_failed("boom")
        from PySide6.QtCore import QCoreApplication, QEvent
        QCoreApplication.sendPostedEvents(None, QEvent.Type.DeferredDelete)  # the old table is gone
        self.assertIsNone(dialog.findChild(QTableWidget, "settings.mediaBackends.list"))
        texts = [w.text() for w in dialog.findChildren(QLabel)]
        self.assertTrue(any("Could not read the backends: boom" in t for t in texts), texts)
        dialog.close()

    def test_menu_items_open_the_page(self):
        """Edit ▸ Settings…, Edit ▸ Media Backends… and Help ▸ Restricted Codecs… exist in the shell."""
        root = Path(__file__).resolve().parents[1]
        env = dict(os.environ, QT_QPA_PLATFORM="offscreen", PYTHONPATH=str(root), CFA4_QT_LIST_MENUS="1")
        r = subprocess.run([sys.executable, "-m", "cfa4_devtools.qt_app", "--smoke"], env=env,
                           capture_output=True, text=True, timeout=60, check=False)
        self.assertEqual(r.returncode, 0, r.stderr)
        for name in ("menu.edit.settings", "menu.edit.mediaBackends", "menu.help.restrictedCodecs"):
            self.assertIn(name, r.stdout)


if __name__ == "__main__":
    unittest.main()
