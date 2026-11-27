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
        self.values: dict[str, Any] = {"playback": {"prerollSeconds": 2.0, "draftDecode": True, "hardwareDecoding": "auto"}}

    def call(self, method: str, params: dict[str, Any] | None = None, on_reply: Any = None, on_error: Any = None) -> int:
        self.sent.append((method, params or {}))
        answers = {"hardware.inventory": HARDWARE, "codecs.capabilities": CODECS}
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
        replies = {"prefs.schema": SCHEMA, "prefs.get": {"values": self.values}, "prefs.set": {}}
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
        ("hardware", ["accelerators", "memory"]),
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
    assert listed(d) == ["#general", "general", "orphan", "#media", "media", "decoding", "playback", "#hardware", "accelerators", "memory"]
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
    d = PreferencesDialog(None, StubEngine(), "general")
    d.patch({"page": "decoding"})
    assert d.page == "decoding" and d.findChild(QtWidgets.QWidget, "settings.page.decoding") is not None
    d.close()


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
