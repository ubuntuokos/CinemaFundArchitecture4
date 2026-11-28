"""The Qt editor's Settings groups and its Qt-only hardware pages (CFA4-SETTINGS-HW-002,
docs/plans/CFA4-SETTINGS-HW-002.md): the category list under the groups of ``prefs.schema``,
Hardware & Devices ▸ Accelerators (``hardware.inventory``) and Media & Playback ▸ Decoding &
Encoding (``codecs.capabilities`` plus the Playback category's two decoding fields). Runs when
PySide6 is installed (offscreen) with a stub engine, and against a built ``cfa4-cli`` when there
is one.
"""

from __future__ import annotations

import os
import time
from pathlib import Path
from typing import TYPE_CHECKING, Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
if TYPE_CHECKING:
    from PySide6 import QtWidgets
else:
    QtWidgets = pytest.importorskip("PySide6.QtWidgets", exc_type=ImportError)

from cfa4_qt.hardware_report import Codecs, Hardware, ReportError, pci_name  # noqa: E402
from cfa4_qt.panels.settings import PreferencesDialog  # noqa: E402
from cfa4_qt.panels.settings_hardware import arrange  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def field(key: str, label: str, kind: dict[str, Any], value: Any) -> dict[str, Any]:
    return {"type": "field", "field": {"key": key, "label": label, "kind": kind, "wired": True, "value": value}}


SYSTEM: dict[str, Any] = {
    "user": "anna",
    "available": {"helper": "/usr/libexec/cfa4/cfa4-system-helper", "installed": True, "pkexec": True},
    "components": [
        {"component": "gpu-access", "status": "needs_action", "details": ["/dev/dri/renderD128: needs group render"], "can_apply": True, "relogin_required": True},
        {"component": "vaapi-drivers", "status": "not_needed", "details": ["NVIDIA's driver decodes through NVDEC; VA-API is not needed."], "can_apply": False},
        {"component": "npu-check", "status": "not_needed", "details": ["No NPU found."], "can_apply": False},
    ],
    "state": {"format": 1, "components": {}},
}
PLAN: dict[str, Any] = {
    "digest": "ab" * 32,
    "plan": {"format": 1, "component": "gpu-access", "user": "anna", "relogin_required": True,
             "actions": [{"kind": "add_to_group", "user": "anna", "group": "render"}]},
}
HW_NOTE = "Hardware decoding: Auto uses the system's video decoder."
SCHEMA: dict[str, Any] = {
    "groups": [
        {"id": "general", "title": "General"},
        {"id": "media", "title": "Media & Playback"},
        {"id": "hardware", "title": "Hardware & Devices"},
    ],
    "categories": [
        {"id": "general", "title": "General", "group": "general", "rows": [], "defaults": {}},
        {"id": "media", "title": "Media", "group": "media", "rows": [], "defaults": {}},
        {
            "id": "playback",
            "title": "Playback",
            "group": "media",
            "rows": [
                field("playback.prerollSeconds", "Preroll", {"type": "float", "min": 0, "max": 60, "decimals": 0}, 2.0),
                field("playback.draftDecode", "Draft decoding at reduced playback resolution", {"type": "bool"}, True),
                field("playback.hardwareDecoding", "Hardware decoding",
                      {"type": "choice", "choices": [{"value": "auto", "label": "Auto"}, {"value": "off", "label": "Off"}]}, "auto"),
                {"type": "note", "text": HW_NOTE},
            ],
            "defaults": {"prerollSeconds": 2.0, "draftDecode": True, "hardwareDecoding": "auto"},
        },
        {"id": "memory", "title": "Memory", "group": "hardware", "rows": [], "defaults": {}},
        {"id": "orphan", "title": "Orphan", "group": "nowhere", "rows": [], "defaults": {}},
    ],
}
HARDWARE = {
    "inventory": {
        "devices": [
            {"kind": "gpu", "vendor": "amd", "pci_vendor": 0x1002, "pci_device": 0x73BF, "name": "AMD GPU", "driver": "amdgpu", "node": "drm:renderD128"},
            {"kind": "npu", "vendor": "intel", "pci_vendor": 0x8086, "pci_device": 0x7D1D, "name": "", "driver": "intel_vpu", "node": "accel:accel0"},
        ],
        "notes": ["routing table: bad line; using the shipped table"],
    },
    "plans": [
        {"workload": "codec_decode", "chain": ["vaapi", "cpu"], "skipped": [{"backend": "nvdec", "reason": "no_device"}]},
        {"workload": "ai", "chain": [], "skipped": []},
    ],
}
CODECS = {
    "hardwareDecoder": {"backend": None, "reason": "no VA-API driver"},
    "gstreamer": {"compiledIn": True, "available": False, "version": None, "reason": "libgstreamer not found"},
    "codecs": [
        {"codec": "H.264", "kind": "video", "decode": "software", "decodeNote": "10-bit 4:2:2 in hardware only",
         "hardwareDecode": None, "encode": "software", "exportFormat": "MP4 (H.264)"},
        {"codec": "HEVC", "kind": "video", "decode": "software", "hardwareDecode": None, "encode": "none"},
    ],
}


class StubEngine:
    """Answers at once, as the engine would; remembers every request. ``fail`` names methods that
    answer with an error; ``hold`` methods that do not answer until ``release``."""

    def __init__(self, fail: frozenset[str] = frozenset(), hold: frozenset[str] = frozenset()) -> None:
        self.sent: list[tuple[str, dict[str, Any]]] = []
        self.fail = fail
        self.hold = hold
        self.held: list[tuple[Any, Any]] = []
        self.system: dict[str, Any] = SYSTEM
        self.values: dict[str, Any] = {"playback": {"prerollSeconds": 2.0, "draftDecode": True, "hardwareDecoding": "auto"}}

    def call(self, method: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
        self.sent.append((method, params or {}))
        answers: dict[str, Any] = {"hardware.inventory": HARDWARE, "codecs.capabilities": CODECS, "system.components": self.system,
                                   "system.plan": PLAN, "system.apply": {"component": "gpu-access", "relogin_required": True},
                                   "system.disable": {"component": "gpu-access", "undone": []}}
        if method in self.fail:
            on_error(f"{method} failed")
        elif method in self.hold:
            self.held.append((on_reply, answers[method]))
        elif method in answers:
            on_reply(answers[method], None)
        return len(self.sent)

    def release(self) -> None:
        for reply, answer in self.held:
            reply(answer, None)
        self.held.clear()

    def execute(self, command: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
        self.sent.append((command, params or {}))
        replies = {"prefs.schema": SCHEMA, "prefs.get": {"values": self.values}, "prefs.set": {},
                   "mediaCache.info": {"location": "/media/work/cache", "files": 3, "bytes": 1024},
                   "file.autoSaveStatus": {"autoSaveDir": "/home/user/.local/share/cfa4/autosave", "sessionDir": "/home/user/.local/share/cfa4/session"}}
        if command in replies and on_reply:
            on_reply(replies[command], None)
        return len(self.sent)


@pytest.fixture(scope="module")
def app() -> Any:
    return QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


def find(d: Any, kind: Any, name: str) -> Any:
    w = d.findChild(kind, name)
    assert w is not None, name
    return w


def listed(d: Any) -> list[str]:
    """The category list top to bottom: group headings as ``#id``, pages by id."""
    out = []
    lay = d.list_layout
    for i in range(lay.count()):
        w = lay.itemAt(i).widget()
        if w is None:
            continue
        name = w.objectName()
        if name.startswith("settings.group."):
            out.append("#" + name.removeprefix("settings.group."))
        elif name.startswith("settings.category."):
            out.append(name.removeprefix("settings.category."))
    return out


def test_arrange_orders_the_groups_and_adds_the_qt_only_pages() -> None:
    sections = arrange(SCHEMA)
    assert [(g["id"] if g else None, [c["id"] for c in pages]) for g, pages in sections] == [
        ("general", ["general", "orphan"]),  # a page of an unknown group goes under General
        ("media", ["media", "decoding", "playback"]),
        ("hardware", ["hardwareOverview", "accelerators", "displays", "storage", "memory", "systemComponents"]),
    ]
    decoding = sections[1][1][1]
    keys = [r["field"]["key"] for r in decoding["rows"] if r["type"] == "field"]
    assert keys == ["playback.hardwareDecoding", "playback.draftDecode"]
    assert any(r.get("text") == HW_NOTE for r in decoding["rows"])
    # an engine without groups keeps its flat list, without the Qt-only pages
    flat = arrange({"categories": SCHEMA["categories"]})
    assert flat == [(None, SCHEMA["categories"])]
    assert arrange({}) == [] and arrange(None) == []


def test_reports_parse_odd_answers() -> None:
    hw = Hardware.parse(HARDWARE)
    assert [d.pci_id for d in hw.devices] == ["1002:73bf", "8086:7d1d"]
    assert hw.plans[1].chain == ("cpu",)  # an empty chain still runs on the CPU
    assert hw.plans[0].skipped == (("nvdec", "no suitable device"),)
    assert Hardware.parse({"inventory": {"devices": [{"pci_vendor": True, "pci_device": -1}]}}).devices[0].pci_id == "—"
    c = Codecs.parse(CODECS)
    assert c.hardware_decoder.name is None and c.gstreamer.name is None and c.gstreamer_compiled_in
    assert c.rows[1].decode_note == "" and c.rows[1].export_format is None
    bad: Any
    for bad in (None, [], {"inventory": []}):
        with pytest.raises(ReportError):
            Hardware.parse(bad)
    with pytest.raises(ReportError):
        Codecs.parse({"codecs": {}})


def test_pci_names_come_from_the_system_list(tmp_path: Path) -> None:
    ids = tmp_path / "pci.ids"
    ids.write_text("# comment\n1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\t73bf  Navi 21\n\t\t1002 0e3a  sub\n10de  NVIDIA Corporation\n")
    assert pci_name(0x1002, 0x73BF, (ids,)) == "Advanced Micro Devices, Inc. [AMD/ATI] Navi 21"
    assert pci_name(0x1002, 0x9999, (ids,)) == "Advanced Micro Devices, Inc. [AMD/ATI]"
    assert pci_name(0x8086, 0x1, (ids,)) is None
    assert pci_name(0, 0, (ids,)) is None
    assert pci_name(0x1002, 0x73BF, (tmp_path / "missing",)) is None


def test_the_list_shows_group_headings_with_their_pages(app: Any) -> None:
    d = PreferencesDialog(None, StubEngine(), "general")
    assert listed(d) == ["#general", "general", "orphan", "#media", "media", "decoding", "playback", "#hardware", "hardwareOverview", "accelerators", "displays", "storage", "memory", "systemComponents"]
    assert find(d, QtWidgets.QLabel, "settings.group.hardware").text() == "Hardware & Devices"
    # the reports are asked for only when their page opens
    assert not [m for m, _p in d.engine.sent if m in ("hardware.inventory", "codecs.capabilities")]
    assert find(d, QtWidgets.QPushButton, "settings.reset").isEnabled()
    d.close()


def test_accelerators_page_lists_devices_and_plans(app: Any) -> None:
    engine = StubEngine()
    d = PreferencesDialog(None, engine, "accelerators")
    assert d.page == "accelerators"
    assert not find(d, QtWidgets.QPushButton, "settings.reset").isEnabled()
    devices = find(d, QtWidgets.QTableWidget, "settings.accelerators.devices")
    assert devices.rowCount() == 2
    assert [devices.item(0, c).text() for c in (0, 2, 3, 4, 5)] == ["GPU", "1002:73bf", "amd", "amdgpu", "drm:renderD128"]
    assert devices.item(1, 1).text()  # the pci.ids name, or Unknown device
    plans = find(d, QtWidgets.QTableWidget, "settings.accelerators.plans")
    assert plans.item(0, 0).text() == "Video decoding" and plans.item(0, 1).text() == "vaapi → cpu"
    assert plans.item(0, 2).text() == "nvdec" and "no suitable device" in plans.item(0, 2).toolTip()
    assert plans.item(1, 1).text() == "cpu"
    # Detect Again asks again, with refresh; switching pages does not
    assert engine.sent.count(("hardware.inventory", {})) == 1
    d.select_page("general")
    d.select_page("accelerators")
    assert engine.sent.count(("hardware.inventory", {})) == 1
    find(d, QtWidgets.QPushButton, "settings.accelerators.refresh").click()
    assert ("hardware.inventory", {"refresh": True}) in engine.sent
    assert find(d, QtWidgets.QTableWidget, "settings.accelerators.devices").rowCount() == 2
    d.close()


def test_accelerators_page_without_devices_and_on_errors(app: Any) -> None:
    engine = StubEngine(hold=frozenset({"hardware.inventory"}))
    d = PreferencesDialog(None, engine, "accelerators")
    assert find(d, QtWidgets.QLabel, "settings.accelerators.status").text() == "Detecting…"
    HARDWARE_NONE = {"inventory": {"devices": []}, "plans": []}
    engine.held[0] = (engine.held[0][0], HARDWARE_NONE)
    engine.release()
    assert find(d, QtWidgets.QLabel, "settings.accelerators.status").text().startswith("No GPU or NPU found")
    d.close()
    d = PreferencesDialog(None, StubEngine(fail=frozenset({"hardware.inventory"})), "accelerators")
    assert "hardware.inventory failed" in find(d, QtWidgets.QLabel, "settings.accelerators.status").text()
    d.close()


def test_decoding_page_edits_the_playback_keys_and_shows_this_machine(app: Any) -> None:
    engine = StubEngine()
    d = PreferencesDialog(None, engine, "decoding")
    combo = find(d, QtWidgets.QComboBox, "settings.playback.hardwareDecoding")
    assert combo.currentData() == "auto"
    assert find(d, QtWidgets.QCheckBox, "settings.playback.draftDecode").isChecked()
    assert "VA-API" in find(d, QtWidgets.QLabel, "settings.codecs.hardwareDecoder").text()
    assert "not available" in find(d, QtWidgets.QLabel, "settings.codecs.gstreamer").text()
    table = find(d, QtWidgets.QTableWidget, "settings.codecs.table")
    assert table.rowCount() == 2 and table.item(0, 0).text() == "H.264" and table.item(0, 1).text() == "Software"
    assert table.item(0, 3).toolTip() == "Export ▸ MP4 (H.264)" and table.item(1, 3).text() == "—"
    combo.setCurrentIndex(combo.findData("off"))
    find(d, QtWidgets.QPushButton, "settings.ok").click()
    assert ("prefs.set", {"values": {"playback.hardwareDecoding": "off"}}) in engine.sent
    d.close()


def test_menu_and_patch_reach_the_qt_only_pages(app: Any) -> None:
    from cfa4_qt.panels.settings import MENU

    assert ("accelerators", "Accelerators (GPU / NPU)…") in MENU and ("decoding", "Decoding & Encoding…") in MENU
    assert ("systemComponents", "System Settings (root)…") in MENU
    assert ("displays", "Displays…") in MENU and ("storage", "Storage Devices…") in MENU
    d = PreferencesDialog(None, StubEngine(), "general")
    d.patch({"page": "decoding"})
    assert d.page == "decoding" and d.findChild(QtWidgets.QWidget, "settings.page.decoding") is not None
    d.close()


def test_system_page_checks_plans_applies_and_undoes(app: Any) -> None:
    engine = StubEngine()
    d = PreferencesDialog(None, engine, "systemComponents")
    asked: list[str] = []

    def confirm(_title: str, text: str) -> bool:
        asked.append(text)
        return True

    d.system.confirm = confirm
    assert not find(d, QtWidgets.QPushButton, "settings.reset").isEnabled()
    assert ("system.components", {}) in engine.sent
    assert find(d, QtWidgets.QLabel, "settings.system.gpu-access.status").text() == "Missing"
    assert find(d, QtWidgets.QLabel, "settings.system.vaapi-drivers.status").text() == "Not needed"
    # Apply needs a plan the user looked at; Undo needs something the helper did
    assert not find(d, QtWidgets.QPushButton, "settings.system.gpu-access.apply").isEnabled()
    assert not find(d, QtWidgets.QPushButton, "settings.system.gpu-access.undo").isEnabled()
    assert not find(d, QtWidgets.QPushButton, "settings.system.npu-check.plan").isEnabled()
    d.system.plan("gpu-access")  # without a parent: no message box
    assert ("system.plan", {"component": "gpu-access"}) in engine.sent
    apply_button = find(d, QtWidgets.QPushButton, "settings.system.gpu-access.apply")
    assert apply_button.isEnabled()
    apply_button.click()
    assert "Add user anna to the group render." in asked[0] and "administrator password" in asked[0]
    assert ("system.apply", {"component": "gpu-access", "digest": "ab" * 32, "plan": PLAN["plan"]}) in engine.sent
    # the helper recorded the change: the status waits for a new login, and Undo is offered
    engine.system = {**SYSTEM, "state": {"format": 1, "components": {"gpu-access": {"status": "applied", "applied": [{"action": {}, "at": "x"}]}}}}
    find(d, QtWidgets.QPushButton, "settings.system.gpu-access.check").click()
    assert find(d, QtWidgets.QLabel, "settings.system.gpu-access.status").text() == "Waiting for log out"
    find(d, QtWidgets.QPushButton, "settings.system.gpu-access.undo").click()
    assert ("system.disable", {"component": "gpu-access"}) in engine.sent
    log = find(d, QtWidgets.QPlainTextEdit, "settings.system.log").toPlainText()
    assert "Log out and back in" in log
    d.close()


def test_system_page_without_the_helper_and_a_cancelled_confirmation(app: Any) -> None:
    engine = StubEngine()
    engine.system = {**SYSTEM, "available": {"helper": None, "installed": False, "pkexec": False}}
    d = PreferencesDialog(None, engine, "systemComponents")
    assert ".deb" in find(d, QtWidgets.QLabel, "settings.system.availability").text()
    assert not find(d, QtWidgets.QPushButton, "settings.system.gpu-access.plan").isEnabled()
    d.close()
    engine = StubEngine()
    d = PreferencesDialog(None, engine, "systemComponents")
    d.system.confirm = lambda title, text: False
    d.system.plan("gpu-access")
    find(d, QtWidgets.QPushButton, "settings.system.gpu-access.apply").click()
    assert not [m for m, _p in engine.sent if m == "system.apply"]
    d.close()


def test_report_pages_link_to_the_system_page_when_something_is_missing(app: Any) -> None:
    d = PreferencesDialog(None, StubEngine(), "accelerators")
    link = find(d, QtWidgets.QPushButton, "settings.system.open")
    link.click()
    assert d.page == "systemComponents"
    d.close()
    engine = StubEngine()
    comps: list[dict[str, Any]] = SYSTEM["components"]
    engine.system = {**SYSTEM, "components": [{**c, "status": "ok", "can_apply": False} for c in comps]}
    d = PreferencesDialog(None, engine, "accelerators")
    assert d.findChild(QtWidgets.QPushButton, "settings.system.open") is None
    d.close()
    # no hardware decoder: the decoding page links there too
    d = PreferencesDialog(None, engine, "decoding")
    assert d.findChild(QtWidgets.QPushButton, "settings.system.open") is not None
    d.close()


def test_accelerators_device_page_shows_one_device_per_entry(app: Any) -> None:
    engine = StubEngine()
    d = PreferencesDialog(None, engine, "accelerators")
    devices = find(d, QtWidgets.QListWidget, "settings.accelerators.page.list")
    assert devices.count() == 2
    # the system helper's check decides the status: renderD128 needs the render group
    assert devices.item(0).text().endswith("No access")
    tabs = find(d, QtWidgets.QTabWidget, "settings.accelerators.page.tabs")
    assert [tabs.tabText(i) for i in range(tabs.count())] == ["Details", "Capabilities", "Drivers", "Diagnostics"]  # no Settings tab
    diagnostics = find(d, QtWidgets.QWidget, "settings.accelerators.page.tab.diagnostics")
    assert any("needs group render" in w.text() for w in diagnostics.findChildren(QtWidgets.QLabel))
    # the selection and the tab survive a rebuild (a report arriving)
    devices.setCurrentRow(1)
    tabs.setCurrentIndex(2)
    find(d, QtWidgets.QPushButton, "settings.accelerators.refresh").click()
    assert find(d, QtWidgets.QListWidget, "settings.accelerators.page.list").currentRow() == 1
    assert find(d, QtWidgets.QTabWidget, "settings.accelerators.page.tabs").currentIndex() == 2
    drivers = find(d, QtWidgets.QWidget, "settings.accelerators.page.tab.drivers")
    assert any("intel_vpu" in w.text() for w in drivers.findChildren(QtWidgets.QLabel))
    d.close()


def test_overview_sums_up_every_hardware_page(app: Any) -> None:
    engine = StubEngine()
    d = PreferencesDialog(None, engine, "hardwareOverview", devices=lambda: {"outputs": ["Speakers"], "inputs": []},
                          memory=lambda: 300 << 20)
    assert not find(d, QtWidgets.QPushButton, "settings.reset").isEnabled()
    for method in ("hardware.inventory", "codecs.capabilities", "system.components", "surface.devices"):
        assert method in [m for m, _p in engine.sent], method
    assert find(d, QtWidgets.QLabel, "settings.overview.accelerators.summary").text() == "1 GPU, 1 NPU"
    row = find(d, QtWidgets.QFrame, "settings.overview.accelerators")
    assert any("group membership" in w.text() for w in row.findChildren(QtWidgets.QLabel))
    assert find(d, QtWidgets.QLabel, "settings.overview.decoding.summary").text() == "Software decoding only"
    assert "GPU access: Missing" in find(d, QtWidgets.QLabel, "settings.overview.systemComponents.summary").text()
    # every row, also those of pages this schema lacks
    rows = {r[0]: r for r in type(d.overview)(engine, d.hardware, d.system, lambda: None, lambda: {"outputs": []}, lambda: 0,
                                                lambda: 0, lambda _p: True, lambda _p: None).rows()}
    assert rows["audioHardware"][2] == "0 outputs, 0 inputs" and "silent" in rows["audioHardware"][3][0]
    assert rows["controlSurfaces"][2] == "Checking…"
    assert find(d, QtWidgets.QLabel, "settings.overview.memory.summary").text().startswith("Decoded pictures: 300 MB")
    # the schema has no Control Surfaces page here, so the dialog shows no row for it
    assert d.findChild(QtWidgets.QFrame, "settings.overview.controlSurfaces") is None
    find(d, QtWidgets.QPushButton, "settings.overview.systemComponents.open").click()
    assert d.page == "systemComponents"
    d.close()


def test_plans_read_as_plain_lines() -> None:
    from cfa4_qt.panels.settings_system import SystemReport, describe_plan, shown_status

    text = describe_plan({"actions": [{"kind": "install_packages", "packages": ["mesa-va-drivers"]}, "junk"]})
    assert text.splitlines() == ["Install packages: mesa-va-drivers", "  Undo: removes the installed packages again."]
    r = SystemReport.parse({"components": [{"component": "gpu-access", "status": "weird"}, {"nope": 1}], "state": []})
    assert [shown_status(c) for c in r.components] == ["unchecked"]
    assert SystemReport.parse(None).components == []


def _real_engine() -> Path | None:
    from cfa4_qt.engine import locate_engine

    return locate_engine()


@pytest.mark.skipif(_real_engine() is None, reason="Rust engine not built (cargo build -p cfa4-cli)")
def test_real_engine_reports_the_hardware_pages(app: Any, tmp_path: Path) -> None:
    """The built engine announces the feature and draws both pages from its own reports."""
    from cfa4_qt.engine import EngineClient

    path = _real_engine()
    assert path is not None
    engine = EngineClient(path, ["--data-dir", str(tmp_path / "data")])
    got: dict[str, Any] = {}
    engine.start()

    def wait(done: Any, seconds: float) -> bool:
        deadline = time.monotonic() + seconds
        while not done() and time.monotonic() < deadline:
            app.processEvents()
            time.sleep(0.01)
        return bool(done())

    try:
        engine.call("hello", {}, on_reply=lambda r, _b: got.update(hello=r))
        assert wait(lambda: "hello" in got, 30)
        assert "hardware" in got["hello"]["features"]
        d = PreferencesDialog(None, engine, "accelerators")
        assert wait(lambda: d.findChild(QtWidgets.QTableWidget, "settings.accelerators.plans") is not None, 30)
        assert d.findChild(QtWidgets.QLabel, "settings.group.hardware") is not None
        d.select_page("decoding")
        assert wait(lambda: d.findChild(QtWidgets.QTableWidget, "settings.codecs.table") is not None, 60)
        table = d.findChild(QtWidgets.QTableWidget, "settings.codecs.table")
        assert table is not None and table.rowCount() > 10
        assert d.findChild(QtWidgets.QComboBox, "settings.playback.hardwareDecoding") is not None
        d.close()
    finally:
        engine.stop()


GB = 1 << 30


def _displays() -> list[Any]:
    from cfa4_qt.panels.settings_devices import Display

    return [Display("1:HDMI-1", "HDMI-1", "Dell", "U2720Q", "", 3840, 2160, 60.0, 2.0, (597, 336), 163.0, 1920, 0, False),
            Display("0:DP-1", "DP-1", "Eizo", "CS2740", "ABC123", 2560, 1440, 59.95, 1.0, (597, 336), 109.0, 0, 0, True)]


def _volumes() -> list[Any]:
    from cfa4_qt.panels.settings_devices import Volume

    return [Volume("/", "/dev/nvme0n1p2", "ext4", 500 * GB, 120 * GB, False, "System"),
            Volume("/media/work", "/dev/sda1", "ext4", 2000 * GB, 50 * GB, False, "Work"),
            Volume("/media/cd", "/dev/sr0", "iso9660", 4 * GB, 0, True, "CD")]


def test_storage_helpers_pick_the_volume_and_the_limit() -> None:
    from cfa4_qt.panels.settings_devices import low_space_limit, volume_of

    vols = _volumes()
    assert volume_of("/media/work/cache/x", vols) is vols[1]
    assert volume_of("/media/workshop", vols) is vols[0]  # a prefix of the name is not the mount point
    assert volume_of("/home/user", vols) is vols[0] and volume_of("", vols) is None
    assert low_space_limit(100 * GB) == 10 * GB and low_space_limit(2000 * GB) == 100 * GB  # 10 GB or 5%
    assert low_space_limit(2000 * GB, 25) == 25 * GB


def test_displays_page_lists_the_screens_primary_first(app: Any) -> None:
    from cfa4_qt.panels.settings_devices import DisplaysPage

    d = PreferencesDialog(None, StubEngine(), "general")
    d.displays = DisplaysPage(d.select_page, lambda: False, lambda: sorted(_displays(), key=lambda x: not x.primary))
    d.select_page("displays")
    items = find(d, QtWidgets.QListWidget, "settings.displays.page.list")
    assert items.count() == 2 and items.item(0).text() == "Eizo CS2740 (DP-1)\nPrimary"
    tabs = find(d, QtWidgets.QTabWidget, "settings.displays.page.tabs")
    assert [tabs.tabText(i) for i in range(tabs.count())] == ["Details", "Capabilities", "Settings", "Diagnostics"]
    details = find(d, QtWidgets.QWidget, "settings.displays.page.tab.details")
    assert "Resolution: 2560 × 1440" in [w.text() for w in details.findChildren(QtWidgets.QLabel)]
    items.setCurrentRow(1)
    caps = find(d, QtWidgets.QWidget, "settings.displays.page.tab.capabilities")
    assert any("covers 2 device pixel" in w.text() for w in caps.findChildren(QtWidgets.QLabel))
    assert any("Detected 2 display(s)" in w.text() for w in find(d, QtWidgets.QWidget, "settings.displays.page.tab.diagnostics").findChildren(QtWidgets.QLabel))
    tabs.setCurrentIndex(2)
    find(d, QtWidgets.QPushButton, "settings.displays.open.color").click()
    assert d.page == "color" or "color" not in [c.get("id") for c in d.categories]
    d.close()


def test_storage_page_marks_the_cfa4_disks_and_low_space(app: Any) -> None:
    from cfa4_qt.panels.settings_devices import StoragePage

    engine = StubEngine()
    d = PreferencesDialog(None, engine, "general")
    d.storage = StoragePage(engine, lambda: d._hardware_changed("storage"), d.select_page, source=_volumes)
    d.select_page("storage")
    assert "mediaCache.info" in [m for m, _p in engine.sent] and "file.autoSaveStatus" in [m for m, _p in engine.sent]
    items = find(d, QtWidgets.QListWidget, "settings.storage.page.list")
    texts = [items.item(i).text() for i in range(items.count())]
    assert texts[1].startswith("Work (/media/work)\n⚠ 50.0 GB free") and "CFA4" in texts[1]  # 50 GB < 5% of 2 TB
    assert "⚠" not in texts[0] and "CFA4" in texts[0]  # auto-saves in the home folder, enough space
    assert "read-only" in texts[2] and "⚠" not in texts[2]  # read-only but no CFA4 location
    items.setCurrentRow(1)
    caps = find(d, QtWidgets.QWidget, "settings.storage.page.tab.capabilities")
    lines = [w.text() for w in caps.findChildren(QtWidgets.QLabel)]
    assert "Media Cache: /media/work/cache" in lines and any(x.startswith("⚠ Low on space") for x in lines)
    text, warnings = d.storage.summary()
    assert text == "50.0 GB free on the media cache's disk (/media/work)" and len(warnings) == 1
    d.close()


def test_overview_has_display_and_storage_rows(app: Any) -> None:
    from cfa4_qt.panels.settings_devices import DisplaysPage, StoragePage

    engine = StubEngine()
    d = PreferencesDialog(None, engine, "general")
    d.displays = DisplaysPage(d.select_page, None, _displays)
    d.storage = StoragePage(engine, lambda: d._hardware_changed("hardwareOverview"), d.select_page, source=_volumes)
    d.overview.extra = [("displays", "Displays", d.displays.summary), ("storage", "Storage Devices", d.storage.summary)]
    d.select_page("hardwareOverview")
    assert find(d, QtWidgets.QLabel, "settings.overview.displays.summary").text() == "2 display(s), primary: Eizo CS2740 (DP-1)"
    row = find(d, QtWidgets.QFrame, "settings.overview.storage")
    assert any(w.text().startswith("⚠ Low on space") for w in row.findChildren(QtWidgets.QLabel))
    find(d, QtWidgets.QPushButton, "settings.overview.storage.open").click()
    assert d.page == "storage"
    d.close()


def test_real_screen_and_volume_lists_do_not_crash(app: Any) -> None:
    from cfa4_qt.panels.settings_devices import PSEUDO_FS, screens, volumes

    assert isinstance(screens(), list)
    assert all(v.fs not in PSEUDO_FS and v.total > 0 for v in volumes())
