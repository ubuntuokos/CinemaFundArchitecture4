"""Qt Settings window backend: CLI contract, schema model and hardware report; no display needed.

The optional Qt test runs only when PySide6 is installed (offscreen), the end-to-end test only when
a built `cfa4-cli` is present (`CFA4_CLI` or target/debug).
"""
import json
import os
import subprocess
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from cfa4_devtools.settings_backend import (
    ACCELERATORS_PAGE,
    Hardware,
    Schema,
    SettingsBackendError,
    changed_values,
    duration_limits,
    lookup,
    pci_name,
    run_cli,
    run_load,
    run_save,
)

SCHEMA = {
    "groups": [
        {"id": "general", "title": "General"},
        {"id": "hardware", "title": "Hardware & Devices"},
        {"id": "ai", "title": "Local AI"},  # no page: not shown
    ],
    "categories": [
        {"id": "general", "title": "General", "group": "general", "fields": [
            {"key": "general.showWelcome", "label": "Show welcome", "wired": True, "value": True,
             "kind": {"type": "bool"}},
        ]},
        {"id": "memory", "title": "Memory", "group": "hardware", "fields": [
            {"key": "memory.reserved", "label": "Reserved", "wired": True, "value": 4,
             "kind": {"type": "int", "min": 1, "max": 64, "unit": "GB"}},
        ]},
        {"id": "timeline", "title": "Timeline", "group": "nope", "fields": [
            {"key": "timeline.still", "label": "Still", "wired": False, "value": 5.0,
             "kind": {"type": "duration", "unitKey": "timeline.stillUnit"}},
            {"key": "timeline.scroll", "label": "Scroll", "wired": True, "value": "page",
             "kind": {"type": "choice", "choices": [{"value": "page", "label": "Page"}, "smooth"]}},
            "not a field", {"label": "no key"},
        ]},
        "junk", {"title": "no id"},
    ],
}
PREFS = {"general": {"showWelcome": True}, "timeline": {"stillUnit": "seconds"}}


def completed(payload, code=0, err=b""):
    out = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
    return subprocess.CompletedProcess([], code, out, err)


class SchemaModel(unittest.TestCase):
    def test_groups_pages_and_the_accelerators_page(self):
        schema = Schema.parse(SCHEMA)
        self.assertEqual([g.id for g in schema.groups], ["general", "hardware"])
        general, hardware = schema.groups
        # a page of an unknown group falls back to General
        self.assertEqual([p.id for p in general.pages], ["general", "timeline"])
        self.assertEqual([p.id for p in hardware.pages], [ACCELERATORS_PAGE, "memory"])
        memory = schema.page("memory").fields[0]
        self.assertEqual((memory.kind, memory.minimum, memory.maximum, memory.unit), ("int", 1.0, 64.0, "GB"))
        timeline = schema.page("timeline")
        self.assertEqual(len(timeline.fields), 2)
        self.assertFalse(timeline.fields[0].wired)
        self.assertEqual([c.value for c in timeline.fields[1].choices], ["page", "smooth"])
        self.assertIsNone(schema.page("missing"))

    def test_values_include_duration_units(self):
        values = Schema.parse(SCHEMA).values(PREFS)
        self.assertEqual(values["timeline.stillUnit"], "seconds")
        self.assertEqual(values["memory.reserved"], 4)

    def test_page_keys_and_duration_limits(self):
        schema = Schema.parse(SCHEMA)
        self.assertEqual(schema.page_keys("timeline"), {"timeline.still", "timeline.stillUnit", "timeline.scroll"})
        self.assertEqual(schema.page_keys("missing"), set())
        self.assertEqual(duration_limits("frames"), (1.0, 100_000.0, 0))
        self.assertEqual(duration_limits("seconds"), (0.01, 3600.0, 2))
        self.assertEqual(duration_limits(None), (0.01, 3600.0, 2))

    def test_hostile_schema(self):
        for bad in (None, [], "x", {"categories": "x"}, {}):
            with self.subTest(bad=bad), self.assertRaises(SettingsBackendError):
                Schema.parse(bad)
        weird = Schema.parse({"categories": [{"id": "x", "fields": [
            {"key": "x.y", "kind": {"type": "float", "min": True, "max": "9", "decimals": 99}}]}]})
        f = weird.page("x").fields[0]
        self.assertEqual((f.minimum, f.maximum, f.decimals, f.label), (None, None, 0, "x.y"))

    def test_changed_values_and_lookup(self):
        self.assertEqual(changed_values({"a": 1, "b": 2}, {"a": 1, "b": 3, "c": 4}), {"b": 3})
        self.assertEqual(lookup(PREFS, "timeline.stillUnit"), "seconds")
        self.assertIsNone(lookup(PREFS, "timeline.stillUnit.deeper"))
        self.assertIsNone(lookup(PREFS, "nope"))


class HardwareModel(unittest.TestCase):
    def test_parse_report(self):
        hw = Hardware.parse({
            "inventory": {"devices": [
                {"kind": "gpu", "vendor": "nvidia", "pci_vendor": 0x10DE, "pci_device": 0x25B0,
                 "driver": "nvidia", "node": "/dev/dri/renderD128"},
                {"kind": "npu", "pci_vendor": True, "pci_device": -1},
                "junk",
            ], "notes": ["n", 3]},
            "plans": [{"workload": "composite", "chain": ["vulkan", "cpu"],
                       "skipped": [{"backend": "nvenc", "reason": "noDevice", "needs": "an NVIDIA GPU"},
                                   {"backend": "npu", "reason": "notImplemented"}, 1]},
                      {"workload": "videoDecode", "chain": []}, {"chain": ["x"]}],
        })
        self.assertEqual([d.pci_id for d in hw.devices], ["10de:25b0", "—"])
        self.assertEqual(hw.plans[0].skipped, (("nvenc", "an NVIDIA GPU"), ("npu", "not implemented yet")))
        self.assertEqual(hw.plans[1].chain, ("cpu",))
        self.assertEqual(len(hw.plans), 2)
        self.assertEqual(hw.notes, ("n",))
        with self.assertRaises(SettingsBackendError):
            Hardware.parse({"inventory": []})

    def test_pci_name(self):
        with TemporaryDirectory() as d:
            ids = Path(d) / "pci.ids"
            ids.write_text("# c\n10de  NVIDIA Corporation\n\t25b0  GA107GL [RTX A1000]\n\t\t1028 0001  sub\n"
                           "1002  AMD\n\t25b0  Not this one\n")
            self.assertEqual(pci_name(0x10DE, 0x25B0, (ids,)), "NVIDIA Corporation GA107GL [RTX A1000]")
            self.assertEqual(pci_name(0x10DE, 0x9999, (ids,)), "NVIDIA Corporation")
            self.assertIsNone(pci_name(0x8086, 0x1, (ids,)))
            self.assertIsNone(pci_name(0, 0, (ids,)))
            self.assertIsNone(pci_name(0x10DE, 0x25B0, (Path(d) / "missing",)))


class CliContract(unittest.TestCase):
    def setUp(self):
        self.directory = TemporaryDirectory(prefix="cfa4-settings-")
        self.addCleanup(self.directory.cleanup)
        self.cli = Path(self.directory.name) / "cfa4-cli"
        self.cli.write_text("#!/bin/sh\nexit 0\n")
        self.cli.chmod(0o755)

    def test_argv_has_no_shell_and_bounded_timeout(self):
        with patch("cfa4_devtools.settings_backend.subprocess.run", return_value=completed({"ok": 1})) as p:
            self.assertEqual(run_cli(self.cli, ["settings", "get"], data_dir=Path("/d")), {"ok": 1})
        args, kwargs = p.call_args
        self.assertEqual(args[0], [str(self.cli), "--compact", "--data-dir", "/d", "settings", "get"])
        self.assertNotIn("shell", kwargs)
        self.assertLessEqual(kwargs["timeout"], 30)
        for timeout in (0, 31):
            with self.assertRaises(SettingsBackendError):
                run_cli(self.cli, [], timeout=timeout)

    def test_failures(self):
        with self.assertRaisesRegex(SettingsBackendError, "missing"):
            run_cli(Path(self.directory.name) / "nope", [])
        cases = [
            (completed(b"", 3, b"bad value"), "bad value"),
            (completed(b"", 4), "exit 4"),
            (completed(b""), "empty"),
            (completed(b"{"), "invalid JSON"),
            (completed(b" " * (4 * 1024 * 1024 + 1)), "too large"),
        ]
        for answer, message in cases:
            with (self.subTest(message=message),
                  patch("cfa4_devtools.settings_backend.subprocess.run", return_value=answer),
                  self.assertRaisesRegex(SettingsBackendError, message)):
                run_cli(self.cli, [])
        with (patch("cfa4_devtools.settings_backend.subprocess.run",
                    side_effect=subprocess.TimeoutExpired("x", 1)),
              self.assertRaisesRegex(SettingsBackendError, "timed out")):
            run_cli(self.cli, [])

    def test_load_and_save(self):
        answers = [completed(SCHEMA), completed({"values": PREFS})]
        with patch("cfa4_devtools.settings_backend.subprocess.run", side_effect=answers):
            schema, prefs = run_load(self.cli)
        self.assertEqual(prefs, PREFS)
        self.assertEqual(schema.groups[0].id, "general")
        with (patch("cfa4_devtools.settings_backend.subprocess.run", side_effect=[completed(SCHEMA), completed([])]),
              self.assertRaises(SettingsBackendError)):
            run_load(self.cli)
        with patch("cfa4_devtools.settings_backend.subprocess.run", return_value=completed(PREFS)) as p:
            run_save(self.cli, {"memory.reserved": 8})
        self.assertEqual(json.loads(p.call_args[0][0][-1]), {"memory.reserved": 8})
        with self.assertRaises(SettingsBackendError):
            run_save(self.cli, {})
        with self.assertRaises(ValueError):
            run_save(self.cli, {"x": float("nan")})


def built_cli():
    explicit = os.environ.get("CFA4_CLI")
    candidate = Path(explicit) if explicit else Path(__file__).resolve().parents[2] / "target/debug/cfa4-cli"
    return candidate if candidate.is_file() else None


@unittest.skipUnless(built_cli(), "needs a built cfa4-cli (CFA4_CLI or target/debug)")
class RealEngine(unittest.TestCase):
    def test_round_trip_through_the_rust_engine(self):
        cli = built_cli()
        with TemporaryDirectory() as d:
            schema, _prefs = run_load(cli, Path(d))
            self.assertIn("hardware", [g.id for g in schema.groups])
            self.assertEqual(schema.groups[[g.id for g in schema.groups].index("hardware")].pages[0].id,
                             ACCELERATORS_PAGE)
            saved = run_save(cli, {"timeline.autoScroll": "smoothScroll"}, Path(d))
            self.assertEqual(saved["timeline"]["autoScroll"], "smoothScroll")
            self.assertTrue((Path(d) / "preferences.json").is_file())
            with self.assertRaises(SettingsBackendError):
                run_save(cli, {"timeline.autoScroll": "sideways"}, Path(d))
            Hardware.parse(run_cli(cli, ["hardware"]))


def qt_available():
    try:
        from PySide6 import QtWidgets  # noqa: F401
    except ImportError:  # PySide6 missing, or its system libraries (libEGL, libGL)
        return False
    return True


@unittest.skipUnless(qt_available(), "needs PySide6")
class QtWindow(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
        from PySide6.QtWidgets import QApplication
        cls.app = QApplication.instance() or QApplication([])

    def test_builds_pages_and_tracks_changes(self):
        from cfa4_devtools.qt_settings import SettingsDialog
        from PySide6.QtWidgets import QCheckBox, QSpinBox

        dialog = SettingsDialog(None)
        dialog._loaded((Schema.parse(SCHEMA), PREFS))
        self.assertEqual(set(dialog.page_widgets), {"general", "timeline", ACCELERATORS_PAGE, "memory"})
        box = dialog.findChild(QCheckBox, "settings.general.showWelcome")
        box.setChecked(False)
        dialog.findChild(QSpinBox, "settings.memory.reserved").setValue(999)  # clamped to max 64
        self.assertEqual(dialog.changes(), {"general.showWelcome": False, "memory.reserved": 64})
        # a reload keeps the edits of other pages and reopens the page
        dialog._loaded((Schema.parse(SCHEMA), PREFS), {"memory.reserved": 8, "unknown": 1}, "memory")
        self.assertEqual(dialog.changes(), {"memory.reserved": 8})
        self.assertEqual(dialog.current_page(), "memory")
        dialog._hardware_loaded(Hardware.parse({"inventory": {"devices": []}, "plans": [
            {"workload": "composite", "chain": ["cpu"]}]}))
        self.assertIsNotNone(dialog.findChild(object, "settings.accelerators.plans"))
        dialog._hardware_failed("boom")
        dialog.close()

    def test_duration_follows_the_unit_limits(self):
        from cfa4_devtools.qt_settings import SettingsDialog
        from PySide6.QtWidgets import QComboBox, QDoubleSpinBox

        dialog = SettingsDialog(None)
        dialog._loaded((Schema.parse(SCHEMA), PREFS))
        spin = dialog.findChild(QDoubleSpinBox)
        unit = dialog.findChild(QComboBox, "settings.timeline.stillUnit")
        spin.setValue(7200)  # seconds: the engine caps at 3600
        self.assertEqual(dialog.edited["timeline.still"], 3600.0)
        unit.setCurrentIndex(unit.findData("frames"))
        self.assertEqual(spin.decimals(), 0)
        spin.setValue(2.6)
        self.assertEqual(dialog.edited["timeline.still"], 3)
        self.assertEqual(dialog.edited["timeline.stillUnit"], "frames")
        dialog.close()

    def test_busy_blocks_editing_and_closing(self):
        from cfa4_devtools.qt_settings import SettingsDialog

        dialog = SettingsDialog(None)
        dialog._loaded((Schema.parse(SCHEMA), PREFS))
        dialog._busy(True, "Saving…")
        self.assertFalse(dialog.stack.isEnabled() or dialog.ok_button.isEnabled() or dialog.reset_button.isEnabled())
        dialog.reject()
        self.assertEqual(dialog.result(), 0)
        dialog._busy(False)
        self.assertTrue(dialog.stack.isEnabled() and dialog.ok_button.isEnabled())
        dialog.close()


if __name__ == "__main__":
    unittest.main()
