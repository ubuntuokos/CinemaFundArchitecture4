"""CFA4 Qt 6 main window: docks, menus from the engine's command registry, and the glue
between panels and the engine. The window holds views of engine state only; every edit is an
engine command and every redraw follows the engine's ``projectChanged``."""

from __future__ import annotations

import math
import time
from collections.abc import Callable
from dataclasses import replace
from itertools import pairwise
from pathlib import Path
from typing import Any

from PySide6.QtCore import QByteArray, QElapsedTimer, QEvent, QPoint, QSettings, Qt, QTimer, QUrl
from PySide6.QtGui import QAction, QActionGroup, QCloseEvent, QCursor, QDesktopServices, QKeyEvent, QKeySequence
from PySide6.QtWidgets import (
    QAbstractSpinBox,
    QApplication,
    QDialog,
    QDockWidget,
    QFileDialog,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QMainWindow,
    QMenu,
    QMessageBox,
    QPlainTextEdit,
    QTabBar,
    QTabWidget,
    QTextEdit,
    QToolButton,
    QWidget,
)

from . import __version__, crash, dock_commands, theme
from .branding import logo_pixmap
from .commands import Command, CommandRegistry, qt_shortcut
from .dialogs import ErrorDialog, InterpretFootageDialog, ParamDialog, path_params
from .engine import EngineClient, EngineError
from .i18n import LANGUAGES, N_, TR, fmt, tr
from .i18n import system_language as i18n_system_language
from .panels.clip_dialogs import DIALOGS as CLIP_DIALOGS
from .panels.clip_dialogs import ClipDialog
from .panels.effect_controls import EffectControlsPanel
from .panels.effects import EffectsPanel
from .panels.essential_graphics import EssentialGraphicsPanel
from .panels.essential_sound import EssentialSoundPanel
from .panels.export import ExportDialog
from .panels.export_dialogs import EXPORT_DIALOGS, ExportSettingsDialog
from .panels.fx_editor import FxEditor, target_key
from .panels.info import InfoPanel, MarkersPanel, MetadataPanel, NotesPanel
from .panels.lumetri import LumetriColorPanel
from .panels.media_browser import MediaBrowserPanel
from .panels.media_dialogs import (
    CreateProxiesDialog,
    LinkMediaDialog,
    MakeOfflineDialog,
    ProjectManagerDialog,
    sequences_of,
)
from .panels.menu_dialogs import MENU_COMMANDS, MenuDialog
from .panels.meters import AudioMetersPanel
from .panels.mixer import ClipMixerPanel, TrackMixerPanel
from .panels.monitor import Monitor, ProgramMonitor, SourceMonitor
from .panels.monitor_view import build_view_menu
from .panels.multicam import EditCamerasDialog, MulticamController, SyncDialog
from .panels.project import ProjectPanel
from .panels.properties import PropertiesPanel
from .panels.quick_export import QuickExport
from .panels.readouts import ProgressPanel, TimecodePanel
from .panels.reference import ReferenceMonitor
from .panels.remix import RemixPropertiesDialog
from .panels.scopes import ScopesPanel
from .panels.sequence_settings import Draft as SeqDraft
from .panels.sequence_settings import SequenceSettingsDialog, count_sequences
from .panels.sequence_settings import draft_from as seq_draft_from
from .panels.settings import MENU as SETTINGS_MENU
from .panels.settings import PreferencesDialog
from .panels.shortcuts import ShortcutsDialog
from .panels.side import EventsPanel, HistoryPanel, LibrariesPanel
from .panels.text import TextPanel
from .panels.timeline import TimelinePanel
from .panels.tools import ToolsPanel, tool_commands
from .panels.tts import TextToSpeechPanel
from .panels.voiceover import VoiceOverRecorder, VoiceOverSettingsDialog
from .timecode import TICKS_PER_SECOND
from .window_menu import Workspaces, bind_free, build_window_menu, window_commands

# egui's EXPORT_SAVE_DIALOGS: exports that ask where to save (filter label, extension)
EXPORT_SAVES = {
    "markers.exportCsv": (N_("Marker report (CSV)"), "csv"),
    "file.exportEdl": ("EDL", "edl"),
    "file.exportFcp7Xml": ("Final Cut Pro XML", "xml"),
    "file.exportFcpxml": ("FCPXML", "fcpxml"),
    "file.exportOtio": ("OpenTimelineIO", "otio"),
    "file.exportAle": (N_("Avid Log Exchange"), "ale"),
    "file.exportSelectionProject": ("CFA4 Project", "fcproj"),
}

# egui's rulers and guides UI commands (menus.rs), run on the focused monitor
# egui's ROLL_PX: a right-click this close to a cut picks Roll (timeline_hit.rs)
ROLL_PX = 2.5
GUIDE_COMMANDS = {
    "view.showRulers": N_("Show Rulers"),
    "view.showGuides": N_("Show Guides"),
    "view.lockGuides": N_("Lock Guides"),
    "view.addGuide": N_("Add Guide…"),
    "view.clearGuides": N_("Clear Guides"),
    "view.snapInProgramMonitor": N_("Snap in Program Monitor"),
    "view.guideTemplates.save": N_("Save Guides as Template…"),
    "view.guideTemplates.manage": N_("Manage Guides…"),
}
# an engine that stopped is restarted, unless it stopped this often within this many seconds
# egui's timeline height UI commands (keyboard.rs): id → (label, default key)
TRACK_HEIGHT_COMMANDS = {
    "timeline.expandAllTracks": (N_("Expand All Tracks"), "Shift+="),
    "timeline.minimizeAllTracks": (N_("Minimize All Tracks"), "Shift+-"),
    "timeline.increaseVideoHeight": (N_("Increase Video Tracks Height"), ""),
    "timeline.decreaseVideoHeight": (N_("Decrease Video Tracks Height"), ""),
    "timeline.increaseAudioHeight": (N_("Increase Audio Tracks Height"), ""),
    "timeline.decreaseAudioHeight": (N_("Decrease Audio Tracks Height"), ""),
}
MAX_RESTARTS = 3
RESTART_WINDOW_S = 60.0
LAYOUT_VERSION = 20  # 2: Source monitor; 3: Effect Controls; 4: Properties (egui name); 5: proportional sizes; 6: Tools panel; 7: Audio Meters; 8: Info, Markers, Metadata, Project Notes, egui's groups; 9: Timecode, Progress; 10: Audio Track Mixer; 11: Audio Clip Mixer; 12: Reference Monitor; 13: Lumetri Scopes; 14: Lumetri Color; 15: Media Browser; 16: Text; 17: Essential Sound; 18: Essential Graphics; 19: Text to Speech; 20: Libraries
# commands that can change which media an item shows, done or undone
SOURCE_COMMANDS = frozenset({
    "media.relink", "media.replaceFootage", "media.makeOffline", "media.offlineAll", "media.autoRelink", "media.linkMedia",
    "media.findMissing", "media.refreshChanged", "media.refreshGrowingFiles", "media.attachProxies", "media.detachProxies",
    "media.toggleProxies", "media.enableProxies", "media.reconnectFullRes", "clip.interpretFootage", "edit.undo", "edit.redo",
})

# panels docked around the Program monitor, in the order they are created
DOCK_PLAN = (
    ("Source", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Effect Controls", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Audio Clip Mixer", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Metadata", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Project", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Media Browser", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Libraries", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Info", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Effects", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Markers", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Project Notes", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Timecode", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Audio Track Mixer", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Reference Monitor", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Lumetri Scopes", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Progress", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Properties", Qt.DockWidgetArea.RightDockWidgetArea),
    ("Essential Graphics", Qt.DockWidgetArea.RightDockWidgetArea),
    ("Text", Qt.DockWidgetArea.RightDockWidgetArea),
    ("Lumetri Color", Qt.DockWidgetArea.RightDockWidgetArea),
    ("Essential Sound", Qt.DockWidgetArea.RightDockWidgetArea),
    ("Text to Speech", Qt.DockWidgetArea.RightDockWidgetArea),
    ("History", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Tools", Qt.DockWidgetArea.BottomDockWidgetArea),
    ("Timeline", Qt.DockWidgetArea.BottomDockWidgetArea),
    ("Events", Qt.DockWidgetArea.BottomDockWidgetArea),
    ("Audio Meters", Qt.DockWidgetArea.BottomDockWidgetArea),
)
# egui's other workspaces (crates/ui-egui/src/dock.rs `workspace`): the tab groups left of the
# Program monitor (with the active tab), the column right of it; Tools, Timeline and Audio Meters
# always run along the bottom
PROJECT_GROUP = ["Project", "Media Browser", "Libraries", "Info", "Effects", "Markers", "History"]
WORKSPACE_LAYOUTS: dict[str, dict[str, Any]] = {
    "Assembly": {
        "left": [(PROJECT_GROUP, 0), (["Source", "Effect Controls", "Audio Clip Mixer", "Metadata"], 0)],
        "leftSplit": Qt.Orientation.Horizontal,
        "right": [],
    },
    "Color": {
        "left": [(["Lumetri Scopes", "Source", "Effect Controls"], 0), (["Project", "Info", "Markers", "History"], 0)],
        "right": [(["Lumetri Color", "Effects"], 0)],
        "rightWidth": 340,
    },
    "Effects": {
        "left": [(["Source", "Effect Controls", "Audio Clip Mixer"], 1), (["Project", "Media Browser", "Info", "Markers", "History"], 0)],
        "right": [(["Effects"], 0), (["Essential Graphics"], 0)],
        "rightWidth": 320,
    },
    "Audio": {
        "left": [(["Audio Track Mixer", "Source", "Audio Clip Mixer", "Effect Controls"], 0), (["Project", "Media Browser", "Effects", "Markers", "History"], 0)],
        "right": [(["Essential Sound", "Text to Speech"], 0)],
        "rightWidth": 320,
    },
    "Captions and Graphics": {
        "left": [(["Text", "Source", "Effect Controls"], 0), (["Project", "Media Browser", "Libraries"], 0)],
        "right": [(["Essential Graphics", "Properties"], 0)],
        "rightWidth": 330,
    },
    "All Panels": {
        "left": [(["Source", "Effect Controls", "Audio Clip Mixer", "Metadata", "Lumetri Scopes", "Text"], 0), (PROJECT_GROUP, 0)],
        "right": [(["Lumetri Color", "Essential Graphics", "Essential Sound", "Text to Speech", "Properties", "Audio Track Mixer"], 0)],
    },
}
TOOLS_W = 46  # the Tools strip, as egui's
COMPACT_PANELS = ("Tools", "Audio Meters")  # egui's compact panels: no tab strip, a slim grip
METERS_W = 116  # the Audio Meters right of the Timeline, as egui's

# engine playhead commands that act on the Source monitor's playhead when it has the focus
SOURCE_STEPS = {
    "playhead.stepForward": 1,
    "playhead.stepBack": -1,
    "playhead.start": "start",
    "playhead.end": "end",
}

# commands that replace the open project (and its undo history) without asking the engine
# whether it is dirty: the frontend offers Save / Don't Save / Cancel first, as egui does
REPLACES_PROJECT = frozenset({"file.open", "file.newProject", "file.openDemoProject", "file.recover"})
STATE_TIMEOUT_MS = 3000


def recovery_text(cand: dict[str, Any]) -> str:
    """The recovery prompt for one of the engine's recovery candidates (camelCase on the wire:
    ``JournalMeta`` in ``crates/engine/src/autosave.rs``)."""
    import datetime

    try:
        when = (
            datetime.datetime.fromtimestamp(int(cand.get("savedUnix")), tz=datetime.UTC)
            .astimezone()
            .strftime("%Y-%m-%d %H:%M:%S")
        )
    except (TypeError, ValueError, OverflowError, OSError):
        when = tr("an unknown time")
    name = str(cand.get("projectName") or tr("Untitled"))
    text = tr("CFA4 quit unexpectedly while '{name}' had unsaved changes. Recover unsaved changes from {time}?")
    if cand.get("cleanExit"):
        text = tr("CFA4 was closed while '{name}' had unsaved changes. Recover unsaved changes from {time}?")
    return text.replace("{name}", name).replace("{time}", when)


# what the monitors run at the sequence playhead: placement edits from their buttons, and the
# Program monitor's eyedropper (it samples the frame there and keys an animated colour there)
# commands that open their dialog when run without parameters (egui's multicam::route)
SYNC_DIALOGS = {"clip.synchronize": "synchronize", "clip.mergeClips": "merge", "clip.createMulticam": "multicam"}
EDITS_AT_PLAYHEAD = frozenset({"source.insert", "source.overwrite", "sequence.lift", "sequence.extract", "effects.eyedropper"})

class MainWindow(QMainWindow):
    def __init__(self, engine: EngineClient, settings: QSettings | None = None) -> None:
        super().__init__()
        self.setObjectName("cfa4.main")
        self.engine = engine
        self.settings = settings or QSettings("CFA4", "CFA4-Qt")
        self.registry = CommandRegistry(self.run_command)
        # the engine's application-wide keys of the Qt-run commands (None: their defaults)
        self.local_keys: dict[str, list[str]] | None = None
        # their panel-scoped keys (keys, panel), dispatched by the focused panel as the engine's
        self.local_panel_keys: dict[str, list[tuple[str, str]]] = {}
        self.shortcuts_dialog: ShortcutsDialog | None = None
        self.project: dict[str, Any] | None = None
        self.sequence: dict[str, Any] | None = None
        self.state: dict[str, Any] = {}
        self.revision = -1
        self.source_state: dict[str, Any] | None = None
        self.export_dialog: ExportDialog | None = None
        # the monitor that keyboard commands with a `target` act on (the focused panel, as egui)
        self.active_monitor = "program"
        self._closing = False
        self._engine_restarts: list[float] = []
        self._quit_confirmed = False

        # egui's Darkest until the engine says which theme Settings ▸ Appearance selects
        theme.apply(theme.DARKEST)
        self.appearance: dict[str, Any] = {}
        self.monitor = ProgramMonitor(engine, self)
        self.source_monitor = SourceMonitor(engine, self)
        self.timeline = TimelinePanel(self)
        self.timeline.canvas.media.attach(engine)  # clip thumbnails and waveforms
        self.tools = ToolsPanel(self)
        self.tools.tool_selected.connect(self.timeline.set_tool)
        self.tools.tool_selected.connect(self.monitor.overlay.set_tool)
        for mon in (self.monitor, self.source_monitor):
            mon.attach_settings(self.settings)
            self.tools.tool_selected.connect(lambda tool, m=mon: m.view.set_hand(tool == "hand"))
            mon.run.connect(self.run_preset)
            mon.message.connect(lambda m: self.events.log(m, error=True))
            mon.status.connect(lambda m: self.statusBar().showMessage(m, 6000))
            mon.program_peer = self.monitor
        self.meters = AudioMetersPanel(self)
        self.meters.watch(self.monitor, self.source_monitor)
        self.project_panel = ProjectPanel(engine, self)
        self.effects = EffectsPanel(engine, self)
        self.effects.attach_settings(self.settings)
        self.inspector = PropertiesPanel(engine, self)
        self.inspector.attach_settings(self.settings)
        self.effect_controls = EffectControlsPanel(self)
        self.effect_controls.attach(engine)
        self.history = HistoryPanel(self)
        self.markers = MarkersPanel(self)
        self.info = InfoPanel(self)
        self.metadata = MetadataPanel(self)
        self.notes = NotesPanel(self)
        self.timecode_panel = TimecodePanel(self)
        self.timecode_panel.attach_settings(self.settings)
        self.timecode_panel.on_change = self.update_timecode
        self._timecode_busy = False
        self.progress = ProgressPanel(self)
        self.progress.attach_settings(self.settings)
        self.progress.poll.timeout.connect(self.update_jobs)
        self.mixer = TrackMixerPanel(self)
        self.mixer.transport.connect(self._mixer_transport)
        self.clip_mixer = ClipMixerPanel(self)
        self.reference = ReferenceMonitor(engine, self)
        self.reference.attach_settings(self.settings)
        self.scopes = ScopesPanel(engine, self)
        self.scopes.attach_settings(self.settings)
        self.reference.scope_settings = lambda: self.scopes.st
        self.media_browser = MediaBrowserPanel(engine, self)
        self.libraries = LibrariesPanel(self)
        self.text_panel = TextPanel(engine, self)
        self.essential_sound = EssentialSoundPanel(self)
        self.essential_sound.attach_settings(self.settings)
        self.essential_graphics = EssentialGraphicsPanel(engine, self)
        self.essential_graphics.attach_settings(self.settings)
        self.tts = TextToSpeechPanel(engine, self)
        self._project_generation = 0  # counts project opens: two untitled projects share a name
        self.last_dialog: QDialog | None = None  # the last frontend dialog opened (tests, agents)
        self.tts.context = lambda: (self._project_generation, (self.sequence or {}).get("id"))
        self._graphics_busy = False
        self._essential_busy = False
        self.lumetri = LumetriColorPanel(self)
        self.lumetri.program = self.monitor
        self.monitor.view_set.connect(self.lumetri.show_comparison)
        self.lumetri.attach_settings(self.settings)
        self._lumetri_busy = False
        self._clip_mixer_busy = False
        self._mixer_busy = False
        self._mixer_clock = QElapsedTimer()
        self._mixer_clock.start()
        self._mixer_heard: tuple[Any, int] | None = None  # (audio generation, sample heard up to)
        self._mixer_timer = QTimer(self)
        self._mixer_timer.setInterval(33)
        self._mixer_timer.timeout.connect(self._mixer_meters)
        self._mixer_timer.start()
        self.inspector_source = "project"  # which selection Info and Metadata follow (egui's)
        self.hidden_markers: list[Any] = []
        self.target_item: int | None = None
        self.events = EventsPanel(self)

        self.setCentralWidget(self.monitor)
        self.setDockOptions(
            QMainWindow.DockOption.AllowTabbedDocks
            | QMainWindow.DockOption.AllowNestedDocks
            | QMainWindow.DockOption.AnimatedDocks
        )
        # panel tabs along the top, as egui's (and Premiere's)
        self.setTabPosition(Qt.DockWidgetArea.AllDockWidgetAreas, QTabWidget.TabPosition.North)
        self.docks: dict[str, QDockWidget] = {}
        widgets = {
            "Source": self.source_monitor,
            "Effect Controls": self.effect_controls,
            "Tools": self.tools,
            "Project": self.project_panel,
            "Effects": self.effects,
            "Properties": self.inspector,
            "History": self.history,
            "Timeline": self.timeline,
            "Events": self.events,
            "Audio Meters": self.meters,
            "Info": self.info,
            "Markers": self.markers,
            "Metadata": self.metadata,
            "Project Notes": self.notes,
            "Timecode": self.timecode_panel,
            "Progress": self.progress,
            "Audio Track Mixer": self.mixer,
            "Audio Clip Mixer": self.clip_mixer,
            "Reference Monitor": self.reference,
            "Lumetri Scopes": self.scopes,
            "Lumetri Color": self.lumetri,
            "Media Browser": self.media_browser,
            "Libraries": self.libraries,
            "Text": self.text_panel,
            "Essential Sound": self.essential_sound,
            "Essential Graphics": self.essential_graphics,
            "Text to Speech": self.tts,
        }
        for name, area in DOCK_PLAN:
            self._dock(name, widgets[name], area)
        # The window itself only ever gets its tab groups through restoreState() (see
        # _default_layout_state and docs/qt-gui.md, "Known Qt issues").
        self._default_state = self._default_layout_state()
        self._builtin_states: dict[str, QByteArray] = {}
        self.workspaces = Workspaces(self, self.settings, self._default_state, LAYOUT_VERSION, self.builtin_layout)

        self.status_engine = QLabel(tr("Starting engine…"))
        self.status_engine.setObjectName("status.engine")
        self.statusBar().addPermanentWidget(self.status_engine)

        for panel in (
            self.monitor,
            self.source_monitor,
            self.timeline.canvas,
            self.history,
            self.project_panel,
            self.effects,
            self.inspector,
            self.effect_controls,
            self.metadata,
            self.notes,
            self.progress,
            self.mixer,
            self.clip_mixer,
            self.lumetri,
            self.media_browser,
            self.text_panel,
            self.essential_sound,
            self.essential_graphics,
            self.tts,
        ):
            panel.command.connect(self.execute)
        self.media_browser.attach_settings(self.settings)
        self.media_browser.invoke.connect(self.invoke)
        self.timeline.canvas.files_dropped.connect(self.place_files)
        self.timeline.canvas.menu_host = self
        # voice-over recording: the audio track headers' microphones and the mixer's record button
        self.voiceover = VoiceOverRecorder(engine, self.monitor, self)
        self.voiceover_dialog: VoiceOverSettingsDialog | None = None
        self.voiceover.changed.connect(self._voiceover_changed)
        self.voiceover.message.connect(self._voiceover_message)
        self.voiceover.refresh.connect(self.schedule_refresh)
        self.timeline.canvas.voiceover.connect(self.voiceover.toggle)
        self.timeline.canvas.voiceover_settings.connect(self.open_voiceover_settings)
        self.timeline.canvas.caption_opened.connect(self.open_captions_tab)
        # multi-camera: the Program monitor's Multi-Camera view, live switching and its dialogs
        self.multicam = MulticamController(engine, self.monitor)
        self.multicam.message.connect(self._voiceover_message)
        self.multicam.refresh.connect(self.schedule_refresh)
        self.monitor.multicam_command.connect(self.multicam.view_command)
        self.monitor.multicam_record.connect(self.multicam.toggle_record)
        self.markers.command.connect(lambda c, p: self._seek_program(p["time"]) if c == "playhead.set" else self.execute(c, p))
        self.effect_controls.seek.connect(self._seek_program)
        self.monitor.overlay.pen_finished.connect(self.effect_controls.pen_done)
        self.effect_controls.eyedropper_armed.connect(self.monitor.overlay.arm_eyedropper)
        self.fx_editors: dict[tuple[Any, ...], FxEditor] = {}
        # an exception that escaped a slot or event handler (crash.py): shown, the session runs on
        self.error_dialog: ErrorDialog | None = None
        crash.reporter().error.connect(self.show_error)
        self.effect_controls.fx_editor.connect(self.open_fx_editor)
        self.mixer.fx_editor.connect(self.open_fx_editor)
        self.monitor.overlay.eyedropper_done.connect(lambda: self.effect_controls.arm_eyedropper(None))
        self.tools.tool_selected.connect(lambda tool: setattr(self.monitor.overlay, "enabled", tool == "selection"))
        self.inspector.seek.connect(self._seek_program)
        self.inspector.run.connect(self.run_preset)
        self.effects.run.connect(self.run_preset)
        self.inspector.invoke.connect(self.invoke)
        self.text_panel.seek.connect(self._seek_program)
        self.text_panel.run.connect(self.run_preset)
        self.essential_graphics.run.connect(self.run_preset)
        self.tts.status.connect(lambda text: self.statusBar().showMessage(text, 6000))
        self.tts.changed.connect(self.schedule_refresh)
        self.tts.bring_forward.connect(lambda: (self.docks["Text to Speech"].show(), self.docks["Text to Speech"].raise_()))
        app = QApplication.instance()
        if isinstance(app, QApplication):
            app.focusChanged.connect(self._focus_changed)
        self.project_panel.invoke.connect(self.invoke)
        self.project_panel.run.connect(self.run_preset)
        self.project_panel.message.connect(lambda text: (self.events.log(text, error=True), self.statusBar().showMessage(text, 6000)))
        self.timeline.canvas.seek.connect(self._scrub)
        self.monitor.playhead_moved.connect(self._playback_moved)
        self.source_monitor.playhead_moved.connect(lambda _t: self.update_timecode())
        self.docks["Timecode"].visibilityChanged.connect(lambda shown: shown and self.update_timecode())
        self.docks["Progress"].visibilityChanged.connect(lambda shown: shown and self.update_jobs())
        self.docks["Audio Track Mixer"].visibilityChanged.connect(lambda shown: shown and self.update_mixer())
        self.docks["Audio Clip Mixer"].visibilityChanged.connect(lambda shown: shown and self.update_clip_mixer())
        self.docks["Lumetri Color"].visibilityChanged.connect(lambda shown: shown and self.update_lumetri())
        self.docks["Essential Sound"].visibilityChanged.connect(lambda shown: shown and self.update_essential_sound())
        self.docks["Essential Graphics"].visibilityChanged.connect(lambda shown: shown and self.update_essential_graphics())
        self.docks["Text"].visibilityChanged.connect(lambda shown: shown and QTimer.singleShot(0, self.text_panel.refresh))

        self._refresh_timer = QTimer(self, singleShot=True, interval=40)
        self._refresh_timer.timeout.connect(self.refresh)
        self._enabled_timer = QTimer(self, singleShot=True, interval=250)
        self._enabled_timer.timeout.connect(self._refresh_enabled)

        engine.started.connect(self._on_started)
        engine.event.connect(self._on_event)
        engine.failure.connect(lambda m: self.events.log(m, error=True))
        engine.stopped.connect(self._on_engine_stopped)

        self._build_local_actions()
        self.menuBar().addMenu(tr("File"))  # placeholder until the registry arrives
        self.resize(1440, 900)
        geometry = self.settings.value("geometry")
        if isinstance(geometry, QByteArray):
            self.restoreGeometry(geometry)
        state = self.settings.value("windowState")
        # the default layout's sizes follow the window (fitted when it is first shown)
        self._fit_pending = False
        if not (isinstance(state, QByteArray) and self.restoreState(state, LAYOUT_VERSION)):
            self.restoreState(self._default_state, LAYOUT_VERSION)
            self._fit_pending = True
        self._update_title()
        self._titles_later()

    @staticmethod
    def arrange_default(win: QMainWindow, d: dict[str, QDockWidget]) -> None:
        """Source over Project/Effects on the left, Program in the middle, Properties/History on
        the right, Timeline/Events at the bottom."""
        win.splitDockWidget(d["Source"], d["Project"], Qt.Orientation.Vertical)
        # egui's Editing groups: Source, Effect Controls, Metadata; Project, Info, Effects,
        # Markers, History (Project Notes there too, closed until Window ▸ Project Notes)
        win.tabifyDockWidget(d["Source"], d["Effect Controls"])
        win.tabifyDockWidget(d["Effect Controls"], d["Audio Clip Mixer"])
        win.tabifyDockWidget(d["Audio Clip Mixer"], d["Metadata"])
        win.tabifyDockWidget(d["Metadata"], d["Audio Track Mixer"])
        win.tabifyDockWidget(d["Audio Track Mixer"], d["Reference Monitor"])
        win.tabifyDockWidget(d["Reference Monitor"], d["Lumetri Scopes"])
        for closed in ("Audio Track Mixer", "Reference Monitor", "Lumetri Scopes"):  # other workspaces; Window menu
            d[closed].hide()
        win.tabifyDockWidget(d["Project"], d["Media Browser"])
        win.tabifyDockWidget(d["Media Browser"], d["Libraries"])
        win.tabifyDockWidget(d["Libraries"], d["Info"])
        win.tabifyDockWidget(d["Info"], d["Effects"])
        win.tabifyDockWidget(d["Effects"], d["Markers"])
        win.tabifyDockWidget(d["Markers"], d["History"])
        win.tabifyDockWidget(d["History"], d["Project Notes"])
        win.tabifyDockWidget(d["Project Notes"], d["Timecode"])
        win.tabifyDockWidget(d["Timecode"], d["Progress"])
        for closed in ("Project Notes", "Timecode", "Progress"):  # opened from the Window menu
            d[closed].hide()
        win.splitDockWidget(d["Tools"], d["Timeline"], Qt.Orientation.Horizontal)  # Tools left of it
        win.splitDockWidget(d["Timeline"], d["Audio Meters"], Qt.Orientation.Horizontal)
        win.tabifyDockWidget(d["Timeline"], d["Events"])
        # egui's Editing group: Properties, Essential Graphics, Text; its Color workspace puts
        # Lumetri Color there too (opened from the Window menu)
        win.tabifyDockWidget(d["Properties"], d["Essential Graphics"])
        win.tabifyDockWidget(d["Essential Graphics"], d["Text"])
        win.tabifyDockWidget(d["Text"], d["Lumetri Color"])
        win.tabifyDockWidget(d["Lumetri Color"], d["Essential Sound"])  # egui's Audio workspace
        win.tabifyDockWidget(d["Essential Sound"], d["Text to Speech"])
        d["Lumetri Color"].hide()
        d["Essential Sound"].hide()
        d["Text to Speech"].hide()
        for name in ("Source", "Project", "Properties", "Timeline"):
            d[name].raise_()
        win.resizeDocks([d["Timeline"]], [300], Qt.Orientation.Vertical)
        win.resizeDocks([d["Tools"], d["Audio Meters"]], [TOOLS_W, METERS_W], Qt.Orientation.Horizontal)
        win.resizeDocks([d["Source"], d["Properties"]], [460, 280], Qt.Orientation.Horizontal)
        win.resizeDocks([d["Source"], d["Project"]], [320, 300], Qt.Orientation.Vertical)

    @staticmethod
    def arrange_workspace(win: QMainWindow, d: dict[str, QDockWidget], name: str) -> None:
        """egui's layout of a built-in workspace (``dock.rs`` ``workspace``) around the Program
        monitor: its left groups stacked (or side by side), its right column, and Tools, Timeline
        and Audio Meters along the bottom; panels it leaves out are closed."""
        spec = WORKSPACE_LAYOUTS[name]
        placed: set[str] = set()

        def column(groups: list[tuple[list[str], int]], area: Qt.DockWidgetArea, split: Qt.Orientation) -> None:
            # the groups' first docks split first: splitting a dock that already has tabs would
            # add a tab to its group instead
            prev: QDockWidget | None = None
            for names, _active in groups:
                first = d[names[0]]
                if prev is None:
                    win.addDockWidget(area, first)
                else:
                    win.splitDockWidget(prev, first, split)
                first.show()
                prev = first
            for names, active in groups:
                for a, b in pairwise(names):
                    win.tabifyDockWidget(d[a], d[b])
                    d[b].show()
                placed.update(names)
                d[names[min(active, len(names) - 1)]].raise_()

        column(spec["left"], Qt.DockWidgetArea.LeftDockWidgetArea, spec.get("leftSplit", Qt.Orientation.Vertical))
        column(spec["right"], Qt.DockWidgetArea.RightDockWidgetArea, Qt.Orientation.Vertical)
        win.addDockWidget(Qt.DockWidgetArea.BottomDockWidgetArea, d["Tools"])
        win.splitDockWidget(d["Tools"], d["Timeline"], Qt.Orientation.Horizontal)
        win.splitDockWidget(d["Timeline"], d["Audio Meters"], Qt.Orientation.Horizontal)
        placed.update(("Tools", "Timeline", "Audio Meters"))
        for n in ("Tools", "Timeline", "Audio Meters"):
            d[n].show()
        for n, dock in d.items():
            if n not in placed:
                dock.hide()
        win.resizeDocks([d["Timeline"]], [300], Qt.Orientation.Vertical)
        win.resizeDocks([d["Tools"], d["Audio Meters"]], [TOOLS_W, METERS_W], Qt.Orientation.Horizontal)
        if spec["right"]:
            firsts = [d[names[0]] for names, _a in spec["right"]]
            win.resizeDocks(firsts[:1], [spec.get("rightWidth", 330)], Qt.Orientation.Horizontal)
            if len(firsts) > 1:  # egui splits the column evenly
                win.resizeDocks(firsts, [400] * len(firsts), Qt.Orientation.Vertical)

    def builtin_layout(self, name: str) -> QByteArray:
        """The saved state of egui's layout of a built-in workspace (made once, then kept)."""
        if name not in WORKSPACE_LAYOUTS:
            return self._default_state
        state = self._builtin_states.get(name)
        if state is None:
            state = self._default_layout_state(name)
            self._builtin_states[name] = state
        return state

    def stand_in_window(self, size: tuple[int, int] = (1440, 900)) -> tuple[QMainWindow, dict[str, QDockWidget]]:
        """A hidden window with empty docks of the same names, to arrange a layout in: the
        caller saves its state, restores that here and deletes the helper."""
        helper = QMainWindow()
        helper.setDockOptions(self.dockOptions())
        helper.setTabPosition(Qt.DockWidgetArea.AllDockWidgetAreas, QTabWidget.TabPosition.North)
        helper.setCentralWidget(QWidget(helper))
        helper.resize(*size)
        stand_ins: dict[str, QDockWidget] = {}
        for name, area in DOCK_PLAN:
            dock = QDockWidget(name, helper)
            dock.setObjectName(f"dock.{name}")
            dock.setWidget(QWidget(dock))
            helper.addDockWidget(area, dock)
            stand_ins[name] = dock
        return helper, stand_ins

    def _default_layout_state(self, workspace: str | None = None) -> QByteArray:
        """The default layout, arranged in a hidden stand-in window with empty docks of the
        same names: tabifying the real window's docks directly leaves orphaned tab bars on screen
        (Qt 6.10.3+) or can crash once it is shown (Qt 6.10.2); restoring a state is clean.
        ``workspace``: one of egui's other built-in workspaces instead of Editing."""
        helper, stand_ins = self.stand_in_window()
        if workspace is None:
            self.arrange_default(helper, stand_ins)
        else:
            self.arrange_workspace(helper, stand_ins, workspace)
        state = helper.saveState(LAYOUT_VERSION)
        helper.deleteLater()
        return state

    def reset_panel_layout(self) -> None:
        """Window ▸ Workspaces ▸ Reset to Saved Layout: the current workspace's saved layout."""
        self.restore_layout(self.workspaces.saved_state(self.workspaces.current))

    # share of the window the default layout gives the timeline (height), the left column with
    # the Source monitor and the right column with Properties (width), as the egui editor does
    TIMELINE_SHARE = 0.42
    LEFT_SHARE = 0.30
    RIGHT_SHARE = 0.16

    def fit_default_sizes(self) -> None:
        """Size the default layout's docks in proportion to the window: fixed pixel sizes left
        the timeline at 13 % of a 4K window."""
        w, h = max(1, self.width()), max(1, self.height())
        self.resizeDocks([self.docks["Timeline"]], [int(h * self.TIMELINE_SHARE)], Qt.Orientation.Vertical)
        self.resizeDocks(
            [self.docks["Source"], self.docks["Properties"]],
            [int(w * self.LEFT_SHARE), int(w * self.RIGHT_SHARE)],
            Qt.Orientation.Horizontal,
        )
        self.resizeDocks([self.docks["Source"], self.docks["Project"]], [int(h * 0.30), int(h * 0.28)], Qt.Orientation.Vertical)
        self.resizeDocks([self.docks["Tools"], self.docks["Audio Meters"]], [TOOLS_W, METERS_W], Qt.Orientation.Horizontal)

    def fit_workspace_sizes(self) -> None:
        """A built-in workspace's Timeline height and bottom strips in proportion to the window."""
        h = max(1, self.height())
        self.resizeDocks([self.docks["Timeline"]], [int(h * self.TIMELINE_SHARE)], Qt.Orientation.Vertical)
        self.resizeDocks([self.docks["Tools"], self.docks["Audio Meters"]], [TOOLS_W, METERS_W], Qt.Orientation.Horizontal)

    def showEvent(self, e) -> None:
        super().showEvent(e)
        if self._fit_pending:
            self._fit_pending = False
            QTimer.singleShot(0, self, self.fit_default_sizes)

    def restore_layout(self, state: QByteArray) -> None:
        before = set(self.findChildren(QTabBar))
        self.restoreState(state, LAYOUT_VERSION)
        if state is self._default_state or state == self._default_state:
            QTimer.singleShot(0, self, self.fit_default_sizes)
        elif any(state is b or state == b for b in self._builtin_states.values()):
            QTimer.singleShot(0, self, self.fit_workspace_sizes)
        self._retire_tab_bars(before)
        QTimer.singleShot(0, self, lambda: self._retire_tab_bars(before))
        self._titles_later()

    def _retire_tab_bars(self, old: set[QTabBar]) -> None:
        """restoreState() on a window that is on screen makes new tab bars and leaves the old
        ones visible under or over them (Qt 6.10.3, 6.11, 6.12). Hide an old bar when a new
        bar shows the same tabs; a stale bar on top would swallow the clicks on the tabs."""
        def tabs(bar: QTabBar) -> tuple[str, ...]:
            return tuple(bar.tabText(i) for i in range(bar.count()))

        by_title = {dock.windowTitle(): dock for dock in self.docks.values()}

        def stale(bar: QTabBar) -> bool:
            # a bar whose tabs are no longer one group of open docks (another layout's group)
            docks = [by_title.get(t) for t in tabs(bar)]
            if not docks or any(dk is None or dk.isHidden() for dk in docks):
                return True
            group = {docks[0], *(x for x in self.tabifiedDockWidgets(docks[0]) if not x.isHidden())}
            return group != set(docks)

        visible = [b for b in self.findChildren(QTabBar) if b.parent() is self and b.isVisible()]
        fresh = {tabs(b) for b in visible if b not in old}
        for bar in visible:
            if bar in old and (bar.count() == 0 or tabs(bar) in fresh or stale(bar)):
                bar.hide()

    def _dock(self, name: str, widget: QWidget, area: Qt.DockWidgetArea) -> QDockWidget:
        dock = QDockWidget(tr(name), self)
        dock.setObjectName(f"dock.{name}")
        dock.setWidget(widget)
        self.addDockWidget(area, dock)
        self.docks[name] = dock
        dock.dockLocationChanged.connect(lambda _a: self._titles_later())
        dock.topLevelChanged.connect(lambda _f: self._titles_later())
        dock.visibilityChanged.connect(lambda _v: self._titles_later())
        return dock

    def createPopupMenu(self) -> QMenu | None:
        """Right-click on a panel's tab or title bar: egui's panel menu (Close Panel, Undock
        Panel, Restore Workspace) instead of Qt's list of docks."""
        title = dock_commands.dock_at(self, QCursor.pos())
        if title is None:
            return super().createPopupMenu()
        menu = dock_commands.panel_menu(self, title, self)
        self.last_panel_menu = menu
        return menu

    def _titles_later(self) -> None:
        if getattr(self, "_titles_pending", False):
            return
        try:  # docks also report while the window is being destroyed
            QTimer.singleShot(0, self, self.update_dock_titles)
        except RuntimeError:
            return
        self._titles_pending = True

    def update_dock_titles(self) -> None:
        """egui shows a panel's name once, on its tab: a panel in a tab group has no title bar of
        its own (its tab names it and drags it), the narrow Tools and Audio Meters panels a slim
        grip instead (egui's compact panels); a panel alone or floating keeps its title bar."""
        self._titles_pending = False
        for name, dock in self.docks.items():
            if dock.isFloating():
                mode = "title"
            elif name in COMPACT_PANELS:
                mode = "grip"
            else:
                mode = "tab" if self.tabifiedDockWidgets(dock) else "title"
            if dock.property("titleMode") == mode:
                continue
            dock.setProperty("titleMode", mode)
            if mode == "title":
                dock.setTitleBarWidget(None)
            else:
                bar = QWidget(dock)
                bar.setObjectName(f"dock.{name}.{mode}")
                bar.setFixedHeight(6 if mode == "grip" else 0)
                bar.setToolTip(tr(name))
                dock.setTitleBarWidget(bar)

    # ----- actions owned by the Qt GUI (view, playback, help) -----
    def _build_local_actions(self) -> None:
        self.play_action = QAction(tr("Play/Stop"), self)
        self.play_action.setObjectName("qt.playback.toggle")
        self.play_action.triggered.connect(lambda: self.focused_monitor().toggle_playback())
        self.export_action = QAction(tr("Export Media…"), self)
        self.export_action.setObjectName("qt.export")
        self.export_action.triggered.connect(self.open_export)
        self.fit_action = QAction(tr("Zoom to Sequence"), self)
        self.fit_action.triggered.connect(self.timeline.canvas.zoom_to_fit)
        self.reset_layout = QAction(tr("Reset Panel Layout"), self)
        self.reset_layout.triggered.connect(self.reset_panel_layout)
        self.quit_action = QAction(tr("Quit"), self)
        self.quit_action.setObjectName("qt.quit")
        self.quit_action.setMenuRole(QAction.MenuRole.QuitRole)
        self.quit_action.triggered.connect(self.close)  # asks to save unsaved changes first
        self.about_action = QAction(tr("About CFA4"), self)
        self.about_action.triggered.connect(self._about)
        # J / K / L shuttle and Play In to Out on the focused monitor (egui's playback.*)
        self.shuttle_actions: list[tuple[QAction, str]] = []
        for name, label, key, slot in (
            ("reverse", N_("Shuttle Left"), "J", lambda: self.focused_monitor().shuttle(-1)),
            ("stop", N_("Shuttle Stop"), "K", lambda: self.focused_monitor().stop()),
            ("forward", N_("Shuttle Right"), "L", lambda: self.focused_monitor().shuttle(1)),
            ("inToOut", N_("Play In to Out"), "Alt+K", lambda: self.focused_monitor().play_in_to_out()),
        ):
            a = QAction(tr(label), self)
            a.setObjectName(f"qt.playback.{name}")
            a.triggered.connect(slot)
            self.shuttle_actions.append((a, key))
        # egui's multi-camera UI commands (menus.rs): Multi-Camera View, Record On/Off, Edit Cameras
        self.multicam_actions: list[tuple[QAction, str]] = []
        for cid, label, key in (
            ("multicam.toggleView", N_("Multi-Camera View"), "Shift+0"),
            ("multicam.recordToggle", N_("Multi-Camera Record On/Off Toggle"), "0"),
            ("multicam.editCamerasDialog", N_("Edit Cameras…"), ""),
        ):
            a = QAction(tr(label), self)
            a.setObjectName(cid)
            a.triggered.connect(lambda _c=False, c=cid: self.invoke(c))
            self.multicam_actions.append((a, key))
        # egui's menu long tail UI commands (menus.rs): View ▸ Dynamic Audio Waveforms (on by
        # default), Help ▸ Reveal Log Files…, System Compatibility Report…
        self.dynamic_waveforms_action = QAction(tr("Dynamic Audio Waveforms"), self)
        self.dynamic_waveforms_action.setObjectName("view.dynamicAudioWaveforms")
        self.dynamic_waveforms_action.setCheckable(True)
        self.dynamic_waveforms_action.setChecked(True)
        self.dynamic_waveforms_action.toggled.connect(self._set_dynamic_waveforms)
        self.help_actions: list[QAction] = []
        for cid, label in (("help.revealLogFiles", N_("Reveal Log Files…")), ("help.systemCompatibilityReport", N_("System Compatibility Report…"))):
            a = QAction(tr(label), self)
            a.setObjectName(cid)
            a.triggered.connect(lambda _c=False, c=cid: self.invoke(c))
            self.help_actions.append(a)
        self.opened: list[str] = []  # paths handed to the file manager (egui's extras.opened)
        self.frame_folder = ""  # Export Frame's last folder (egui's frame-export-folder)
        self.open_path_hook: Callable[[str, bool], None] | None = None  # tests: instead of the OS
        self.shortcuts_action = QAction(tr("Keyboard Shortcuts…"), self)
        self.shortcuts_action.setObjectName("app.keyboardShortcuts")
        self.shortcuts_action.triggered.connect(lambda _c=False: self.open_shortcuts())
        self.theme_actions = QActionGroup(self)
        for name, label in theme.THEMES:
            a = QAction(tr(label), self, checkable=True)
            a.setObjectName(f"view.theme.{name}")
            a.setData(name)
            a.setChecked(name == theme.current().kind)
            a.triggered.connect(lambda _c=False, n=name: self.set_theme(n))
            self.theme_actions.addAction(a)
        self.next_mode_action = QAction(tr("Next Appearance Mode"), self)
        self.next_mode_action.setObjectName("view.appearanceMode.next")
        self.next_mode_action.triggered.connect(self.next_appearance_mode)
        self.lang_actions = QActionGroup(self)
        for code, name in LANGUAGES.items():
            a = QAction(name, self, checkable=True)
            a.setData(code)
            a.setChecked(code == TR.language)
            a.triggered.connect(lambda _c=False, c=code: self.set_language(c))
            self.lang_actions.addAction(a)

    def local_commands(self) -> list[tuple[str, str, list[str], str]]:
        """(id, label, menu, default key) of the commands the Qt GUI runs itself, with egui's ids
        (``menus.rs`` UI_COMMANDS): registered with the engine's shortcut set
        (``shortcuts.registerExternal``) so Keyboard Shortcuts lists and rebinds them as egui's."""
        out: list[tuple[str, str, list[str], str]] = [
            ("playback.toggle", N_("Play/Stop"), [], "Space"),
            ("playback.reverse", N_("Shuttle Left"), [], "J"),
            ("playback.stop", N_("Shuttle Stop"), [], "K"),
            ("playback.forward", N_("Shuttle Right"), [], "L"),
            ("playback.inToOut", N_("Play In to Out"), [], "Alt+K"),
            ("mode.export", N_("Export"), ["File", "Export"], "Cmd+M"),
            ("app.quit", N_("Quit"), [], "Cmd+Q"),
            ("multicam.toggleView", N_("Multi-Camera View"), ["View"], "Shift+0"),
            ("multicam.recordToggle", N_("Multi-Camera Record On/Off Toggle"), [], "0"),
            ("multicam.editCamerasDialog", N_("Edit Cameras…"), [], ""),
            ("app.keyboardShortcuts", N_("Keyboard Shortcuts…"), ["Edit"], "Cmd+Alt+K"),
            ("view.dynamicAudioWaveforms", N_("Dynamic Audio Waveforms"), ["View"], ""),
            ("help.revealLogFiles", N_("Reveal Log Files…"), ["Help"], ""),
            ("help.systemCompatibilityReport", N_("System Compatibility Report…"), ["Help"], ""),
            *((cid, dock_commands.LABELS[cid], [], "") for cid in dock_commands.COMMANDS),
            *((cid, label, ["View"], "") for cid, label in GUIDE_COMMANDS.items()),
            ("view.compare.setReference", N_("Set Comparison Reference"), [], ""),
            *((cid, label, [], key) for cid, (label, key) in TRACK_HEIGHT_COMMANDS.items()),
        ]
        out += [(f"app.settings.{page}", label, ["Edit", "Preferences"], "Cmd+," if page == "general" else "") for page, label in SETTINGS_MENU]
        return out + tool_commands() + window_commands()

    def _local_keys(self, cid: str, default: str) -> list[str]:
        """Qt key strings of a Qt-run command: its engine bindings (application-wide), or its
        default before the engine has answered (or when it could not take the commands)."""
        keys = self.local_keys.get(cid, []) if self.local_keys is not None else ([default] if default else [])
        return [q for k in keys if (q := qt_shortcut(k))]

    def local_action_pairs(self) -> list[tuple[QAction, str, str]]:
        """(action, command id, default key) of the Qt-run commands with an action of their own."""
        return [
            (self.play_action, "playback.toggle", "Space"),
            (self.export_action, "mode.export", "Cmd+M"),
            (self.quit_action, "app.quit", "Cmd+Q"),
            (self.shortcuts_action, "app.keyboardShortcuts", "Cmd+Alt+K"),
            *((a, f"playback.{a.objectName().rsplit('.', 1)[-1]}", k) for a, k in self.shuttle_actions),
            *((a, a.objectName(), k) for a, k in self.multicam_actions),
            (self.dynamic_waveforms_action, "view.dynamicAudioWaveforms", ""),
            *((a, a.objectName(), "") for a in self.help_actions),
        ]

    def _bind_local_shortcuts(self, bound_keys: set[str]) -> list[QAction]:
        """Playback, multi-camera and other Qt-run keys, unless an engine command claims them."""
        pairs = self.local_action_pairs()
        added = bind_free(self, [(a, self._local_keys(cid, k)) for a, cid, k in pairs], bound_keys)
        for a, _cid, _k in pairs:
            if a not in added:
                self.addAction(a)
        return [a for a, _c, _k in pairs]

    def _build_menus(self) -> None:
        self.registry.build_menus(
            self.menuBar(), self, {
                "File": [self.quit_action],
                "Sequence": [self.play_action, *(a for a, _k in self.shuttle_actions)],
                "Edit": [self.shortcuts_action],
                "View": [self.fit_action, self.multicam_actions[0][0], self.dynamic_waveforms_action],
                "Help": [*self.help_actions, self.about_action],
            },
            self.local_panel_keys,
            self.local_keys or {},
        )
        local: dict[str, QAction] = {}
        menus = {a.menu().objectName(): a for a in self.menuBar().actions() if a.menu()}
        view = menus.get("menu.View")
        if view is not None:
            view.menu().addSeparator()
            build_view_menu(view.menu(), self.focused_monitor)
            view.menu().addSeparator()
            appearance = view.menu().addMenu(tr("Appearance"))
            appearance.setObjectName("menu.View.Appearance")
            for a in self.theme_actions.actions():
                appearance.addAction(a)
            appearance.addAction(self.next_mode_action)
            lang = view.menu().addMenu(tr("Language"))
            lang.setObjectName("menu.View.Language")
            for a in self.lang_actions.actions():
                lang.addAction(a)
        edit = menus.get("menu.Edit")
        if edit is not None:
            edit.menu().addSeparator()
            prefs = edit.menu().addMenu(tr("Preferences"))
            prefs.setObjectName("menu.Edit.Preferences")
            for page, label in SETTINGS_MENU:
                a = prefs.addAction(tr(label))
                a.setObjectName(f"app.settings.{page}")
                keys = [QKeySequence(q) for q in self._local_keys(a.objectName(), "Cmd+," if page == "general" else "")]
                a.setShortcuts([k for k in keys if k.toString() not in self.registry.claimed_keys])
                a.triggered.connect(lambda _c=False, pg=page: self.open_settings(pg))
                local[a.objectName()] = a
        self._build_appearance_button()
        # Window, before Help as in the egui editor; the previous build's actions go first
        for action in getattr(self, "_window_actions", []):
            self.removeAction(action)
            if action is not self.reset_layout:  # kept across rebuilds
                action.deleteLater()
        if getattr(self, "_window_menu", None) is not None:
            self._window_menu.deleteLater()
        window_menu = QMenu(tr("Window"), self.menuBar())
        self._window_menu = window_menu
        window_menu.setObjectName("menu.Window")
        help_action = menus.get("menu.Help")
        if help_action is not None:
            self.menuBar().insertMenu(help_action, window_menu)
        else:
            self.menuBar().addMenu(window_menu)
        window_shortcuts = build_window_menu(
            self, window_menu, self.docks, self.monitor, self.workspaces, self.restore_layout, self.reset_layout
        )
        keys = set(self.registry.claimed_keys)
        for a in self._bind_local_shortcuts(keys):
            keys |= {s.toString() for s in a.shortcuts()}
        # V, A, B, N, R, C, Y, U, H, Z ... as egui, then the Window menu's keys
        tools = bind_free(self, [(a, self._local_keys(cid, k)) for a, cid, k in self.tools.shortcut_pairs()], keys)
        for a in tools:
            keys |= {s.toString() for s in a.shortcuts()}
        self._window_actions = bind_free(self, [(a, self._local_keys(cid, k)) for a, cid, k in window_shortcuts], keys) + [
            a for a in window_menu.actions() if a not in {p[0] for p in window_shortcuts}
        ]
        for a in self._window_actions:
            keys |= {s.toString() for s in a.shortcuts()}
        for a, cid, _k in [*self.local_action_pairs(), *self.tools.shortcut_pairs(), *window_shortcuts]:
            local.setdefault(cid, a)
        # the other Qt-run commands (docking, guides, the comparison reference …): an action each,
        # so a key given to them in Keyboard Shortcuts runs them
        for old in getattr(self, "_command_actions", []):
            self.removeAction(old)
            old.deleteLater()
        extra = []
        for cid, label, _menu, default in self.local_commands():
            if cid in local:
                continue
            a = QAction(tr(label), self)
            a.setObjectName(f"shortcut.{cid}")
            a.triggered.connect(lambda _c=False, c=cid: self.ui_command(c))
            local[cid] = a
            extra.append((a, self._local_keys(cid, default)))
        self._command_actions = [a for a, _k in extra]
        bind_free(self, extra, keys)
        self.registry.local_actions = local

    def rebuild_window_menu(self) -> None:
        """A workspace was added: rebuild the menus (the engine's part comes from the registry)."""
        self._build_menus()

    def set_language(self, code: str) -> None:
        TR.set_language(code)
        self.settings.setValue("language", code)
        for name, dock in self.docks.items():
            dock.setWindowTitle(tr(name))
        for cid, action in self.registry.actions.items():
            action.setText(tr(self.registry.commands[cid].label))
        self._build_menus()
        self.events.log(tr("Some panel labels change language after a restart."))

    # ----- engine lifecycle -----
    def _on_started(self, hello: dict[str, Any]) -> None:
        self.status_engine.setText(str(hello.get("engine") or "engine"))
        self.register_local_commands()
        self.engine.call("engine.commands", on_reply=self._on_commands)
        self.engine.execute("effects.list", {}, on_reply=self._on_effects_list)
        self.load_appearance()
        self.load_audio_output()
        self.refresh()
        recovery = hello.get("recovery")
        if isinstance(recovery, list) and recovery and isinstance(recovery[0], dict):
            QTimer.singleShot(0, lambda c=recovery[0]: self._offer_recovery(c))

    # ----- appearance -----
    def load_appearance(self) -> None:
        """Take the theme from the engine's Settings ▸ Appearance (shared with egui)."""

        def got(result: Any, _data: bytes | None) -> None:
            if isinstance(result, dict):
                value = result.get("value", result)
                self.appearance = value if isinstance(value, dict) else {}
                theme.apply(theme.tokens_from_prefs(self.appearance))
                for a in self.theme_actions.actions():
                    a.setChecked(a.data() == theme.current().kind)
                self._update_appearance_button()

        self.engine.execute("prefs.get", {"key": "appearance"}, on_reply=got)

    def _build_appearance_button(self) -> None:
        """egui's header Appearance Mode button, at the menu bar's right: a click goes to the next
        mode (Auto, Light, Dark), its context menu has Appearance Settings… and the favourite
        themes."""
        button = getattr(self, "appearance_button", None)
        if button is None:
            # the header's right end: Quick Export, then Appearance Mode (as egui's header)
            corner = QWidget(self.menuBar())
            corner.setObjectName("header")
            row = QHBoxLayout(corner)
            row.setContentsMargins(0, 0, 4, 0)
            row.setSpacing(2)
            quick = QToolButton(corner)
            quick.setObjectName("header.quickExport")
            quick.setAutoRaise(True)
            quick.setText("⇪")
            quick.setToolTip(tr("Quick Export"))
            quick.clicked.connect(self.toggle_quick_export)
            row.addWidget(quick)
            button = QToolButton(corner)
            button.setObjectName("header.appearance")
            button.setAutoRaise(True)
            button.clicked.connect(self.next_appearance_mode)
            button.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
            button.customContextMenuRequested.connect(lambda pos: self.appearance_menu().exec(button.mapToGlobal(pos)))
            row.addWidget(button)
            self.appearance_button = button
            self.quick_export_button = quick
            self.header_corner = corner
        if self.menuBar().cornerWidget(Qt.Corner.TopRightCorner) is not self.header_corner:
            self.menuBar().setCornerWidget(self.header_corner, Qt.Corner.TopRightCorner)
        self.header_corner.show()
        self._update_appearance_button()

    def toggle_quick_export(self) -> None:
        """The header's Quick Export button: the popup below it, or closed when it is open."""
        popup = getattr(self, "quick_export", None)
        if popup is None:
            popup = self.quick_export = QuickExport(self.engine, self)
            popup.status.connect(lambda text: (self.events.log(text), self.statusBar().showMessage(text, 6000)))
        if popup.isVisible():
            popup.close()
            return
        b = self.quick_export_button
        popup.open_at(b.mapToGlobal(b.rect().bottomRight()) + QPoint(0, 4), self.sequence)

    def _update_appearance_button(self) -> None:
        button = getattr(self, "appearance_button", None)
        if button is None:
            return
        mode = str(self.appearance.get("appearanceMode") or "dark")
        icon, name = {"auto": ("🖥", tr("Sync with system")), "light": ("☀", tr("Light"))}.get(mode, ("☾", tr("Dark")))
        button.setText(icon)
        button.setToolTip(f"{tr('Appearance Mode')}: {name}")

    def appearance_menu(self) -> QMenu:
        """Appearance Settings… and the favourite themes, light ones first (egui's
        ``appearance_context_menu``)."""
        menu = QMenu(self)
        menu.setObjectName("header.appearance.menu")
        a = menu.addAction(tr("Appearance Settings…"))
        a.setObjectName("header.appearance.settings")
        a.triggered.connect(lambda: self.open_settings("appearance"))
        favourites = [str(x) for x in self.appearance.get("favoriteThemes") or [] if isinstance(x, str)]
        for group in (("light",), ("darkest", "dark")):
            items = [(k, label) for k, label in theme.THEMES if k in group and k in favourites]
            if items:
                menu.addSeparator()
            for k, label in items:
                act = menu.addAction(tr(label))
                act.setObjectName(f"header.appearance.theme.{k}")
                act.setCheckable(True)
                act.setChecked(theme.current().kind == k)
                act.triggered.connect(lambda _c=False, name=k: self.set_theme(name))
        self.last_menu = menu
        return menu

    def open_captions_tab(self) -> None:
        """egui: a caption block double-clicked in the Timeline shows the Text panel's Captions tab."""
        self.docks["Text"].show()
        self.docks["Text"].raise_()
        self.text_panel.set_tab("Captions")

    def open_voiceover_settings(self) -> VoiceOverSettingsDialog:
        """Voice-Over Record Settings (the microphone's menu, ``voiceover.settingsDialog``)."""
        dialog = VoiceOverSettingsDialog(self, self.engine, self.voiceover)
        dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        dialog.destroyed.connect(lambda *_a: setattr(self, "voiceover_dialog", None))
        self.voiceover_dialog = dialog
        self.last_dialog = dialog
        dialog.open()
        return dialog

    def open_sync_dialog(self, kind: str) -> SyncDialog:
        """Synchronize Clips / Merge Clips / Create Multi-Camera Source Sequence (egui's)."""

        def done(text: str) -> None:
            self.statusBar().showMessage(text, 6000)
            self.schedule_refresh()

        dialog = SyncDialog(self, self.engine, kind, done)
        dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        self.last_dialog = dialog
        dialog.open()
        return dialog

    def open_clip_dialog(self, cid: str) -> None:
        """An Edit / Clip / File menu command's dialog, from the engine's ``dialog.defaults``."""

        def run(command: str, params: dict[str, Any], then: Any) -> None:
            def ok(r: Any, _b: bytes | None) -> None:
                self.schedule_refresh()
                then(None, r)

            self.engine.execute(command, params, on_reply=ok, on_error=lambda m: then(str(m), None))

        def got(state: Any, _b: bytes | None) -> None:
            if not isinstance(state, dict):
                self._command_failed(cid, tr("no dialog"))
                return
            dialog = ClipDialog(self, state, run)
            dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
            self.last_dialog = dialog
            dialog.open()

        self.engine.execute("dialog.defaults", {"command": cid}, on_reply=got, on_error=lambda m: self._command_failed(cid, m))

    def open_menu_dialog(self, cid: str) -> None:
        """A File / Edit / Clip / Sequence / Markers menu dialog (egui's ``menu_dialogs``), from the
        engine's ``dialog.defaults``; Find starts from the project search and the timeline focus."""

        def run(command: str, params: dict[str, Any], then: Any) -> None:
            def ok(r: Any, _b: bytes | None) -> None:
                self.schedule_refresh()
                then(None, r)

            self.engine.execute(command, params, on_reply=ok, on_error=lambda m: then(str(m), None))

        def got(state: Any, _b: bytes | None) -> None:
            if not isinstance(state, dict):
                self._command_failed(cid, tr("no dialog"))
                return
            dialog = MenuDialog(self, state, run, status=lambda t: self.statusBar().showMessage(t, 6000))
            dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
            self.last_dialog = dialog
            dialog.open()

        query = {"command": cid, "timeline": self.inspector_source == "timeline", "searchText": self.project_panel.filter.text()}
        self.engine.execute("dialog.defaults", query, on_reply=got, on_error=lambda m: self._command_failed(cid, m))

    def open_export_dialog(self, cid: str, params: dict[str, Any]) -> None:
        """Export Frame (egui's ``frame_export``: the frame of the monitor that asked, else the
        focused one, captured now; the last export's folder) and AAF / OMF Export Settings."""
        query: dict[str, Any] = {"command": cid}
        if cid == "file.exportFrame":
            mon = self.source_monitor if params.get("target") == "source" else self.monitor if params.get("target") == "program" else self.focused_monitor()
            if mon.playing:
                mon.stop()
            query.update({"target": mon.target, "time": int(mon.playhead)})
            query.update({k: v for k, v in params.items() if k in ("target", "time", "item", "sequence")})

        def run(command: str, p: dict[str, Any], then: Any) -> None:
            def ok(r: Any, _b: bytes | None) -> None:
                self.schedule_refresh()
                then(None, r)

            self.engine.execute(command, p, on_reply=ok, on_error=lambda m: then(str(m), None))

        def remember(folder: str) -> None:
            self.frame_folder = folder

        def got(state: Any, _b: bytes | None) -> None:
            if not isinstance(state, dict):
                self._command_failed(cid, tr("no dialog"))
                return
            p = state.get("params")
            if cid == "file.exportFrame" and self.frame_folder and isinstance(p, dict):
                p["folder"] = self.frame_folder
            dialog = ExportSettingsDialog(self, state, run, status=lambda t: self.statusBar().showMessage(t, 6000), remember=remember)
            dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
            self.last_dialog = dialog
            dialog.open()

        self.engine.execute("dialog.defaults", query, on_reply=got, on_error=lambda m: self._command_failed(cid, m))

    # ----- the frontend side of the menu long tail (egui's menu_dialogs::route) -----
    def _set_dynamic_waveforms(self, on: bool) -> None:
        self.timeline.canvas.dynamic_waveforms = on
        self.timeline.canvas.update()

    def open_path(self, path: str, reveal: bool = False) -> None:
        """Hand a file to the OS: open it, or (``reveal``) show its folder in the file manager."""
        self.opened = [*self.opened, path][-50:]
        if self.open_path_hook is not None:
            self.open_path_hook(path, reveal)
            return
        target = str(Path(path).parent) if reveal else path
        QDesktopServices.openUrl(QUrl.fromLocalFile(target))

    def open_system_report(self) -> None:
        def got(report: Any, _b: bytes | None) -> None:
            state = {"command": "help.systemCompatibilityReport", "params": {}, "info": report if isinstance(report, dict) else {}}
            self._show_menu_dialog(state)

        self.engine.execute("help.systemReport", {}, on_reply=got, on_error=lambda m: self._command_failed("help.systemCompatibilityReport", m))

    def reveal_log_files(self) -> None:
        """This session's log (system report and command journal) written by the engine, then
        shown in the file manager."""

        def got(r: Any, _b: bytes | None) -> None:
            path = r.get("path") if isinstance(r, dict) else None
            if isinstance(path, str) and path:
                self.open_path(path, reveal=True)
                self.statusBar().showMessage(fmt("Session log: {path}", path=path), 6000)

        self.engine.execute("help.writeSessionLog", {}, on_reply=got, on_error=lambda m: self._command_failed("help.revealLogFiles", m))

    def open_originals(self, cid: str, params: dict[str, Any]) -> None:
        """Edit Original opens the media files, Reveal in Finder shows them in the file manager."""

        def got(r: Any, _b: bytes | None) -> None:
            paths = r.get("paths") if isinstance(r, dict) else None
            for p in paths if isinstance(paths, list) else []:
                if isinstance(p, str) and p:
                    self.open_path(p, reveal=cid == "file.reveal")

        self.engine.execute(cid, params, on_reply=got, on_error=lambda m: self._command_failed(cid, m))

    def generate_waveforms(self, params: dict[str, Any]) -> None:
        """Generate Audio Waveform: the engine re-reads the peaks; the Timeline forgets its own."""

        def got(r: Any, _b: bytes | None) -> None:
            items = r.get("items") if isinstance(r, dict) else None
            ids = {i for i in items if isinstance(i, int) and not isinstance(i, bool)} if isinstance(items, list) else set()
            if ids:
                self.timeline.canvas.media.forget_peaks(ids)
            self.schedule_refresh()

        self.engine.execute("clip.generateAudioWaveform", params, on_reply=got, on_error=lambda m: self._command_failed("clip.generateAudioWaveform", m))

    def pick_and_run(self, cid: str, params: dict[str, Any]) -> None:
        """egui's ``EXPORT_SAVE_DIALOGS`` (EDL, Final Cut Pro XML, FCPXML, OpenTimelineIO, the
        marker CSV, Avid Log Exchange, Export Selection as Project) ask where to save; Get Media
        File Properties for ▸ File… asks for the file and shows its Properties."""
        if cid == "file.mediaPropertiesFile":
            path, _f = QFileDialog.getOpenFileName(self, tr("Media"), "", tr("All files") + " (*)")
        else:
            cmd = self.registry.commands.get(cid)
            if cmd is not None and not cmd.enabled:
                self._command_failed(cid, tr("not available now"))
                return
            name = str((self.project or {}).get("name") or tr("Untitled"))
            label, suffix = EXPORT_SAVES.get(cid, ("CFA4 Project", "fcproj"))
            filt, ext = f"{tr(label)} (*.{suffix})", f".{suffix}"
            if cid == "file.exportAle":
                start = f"{name}.ale"
            elif cid == "file.exportSelectionProject":
                start = fmt("{name} Selection.fcproj", name=name)
            else:  # timelines are named after the sequence (egui's)
                seq = str((self.sequence or {}).get("name") or "").strip() or name
                start = "".join("-" if c in "/\\" or not c.isprintable() else c for c in seq) + ext
            path, _f = QFileDialog.getSaveFileName(self, tr(cmd.label if cmd is not None else cid), start, filt)
            if path and not path.endswith(ext):
                path += ext
        if not path:
            return

        def got(r: Any, _b: bytes | None) -> None:
            self.schedule_refresh()
            if cid == "file.mediaPropertiesFile":
                self._show_menu_dialog({"command": "file.mediaProperties", "params": {}, "info": [r]})

        self.engine.execute(cid, {**params, "path": path}, on_reply=got, on_error=lambda m: self._command_failed(cid, m))

    # ----- media management dialogs (egui's media_dialogs) -----
    def _link_media_open(self) -> bool:
        return getattr(self, "link_media", None) is not None

    def open_link_media(self, menu: bool = False) -> None:
        """Link Media: the clips whose media is missing (File ▸ Link Media…, or a project opened
        with missing media); from the menu with everything online it only says so."""
        if self._link_media_open():
            return

        def got(r: Any, _b: bytes | None) -> None:
            missing = r.get("missing") if isinstance(r, dict) else None
            if not missing:
                if menu:
                    self.statusBar().showMessage(tr("All media is online."), 6000)
                return
            if self._link_media_open():
                return
            d = LinkMediaDialog(self, self.engine, status=self._media_status, changed=self._media_changed)
            d.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
            d.finished.connect(lambda _r: setattr(self, "link_media", None))
            self.link_media = d
            self.last_dialog = d
            d.open()

        self.engine.execute("media.linkMedia", {}, on_reply=got, on_error=lambda m: self._command_failed("media.linkMedia", m))

    def _media_status(self, text: str) -> None:
        self.statusBar().showMessage(text, 6000)

    def _media_changed(self) -> None:
        self.clear_media_caches()  # relinked or offline items show other pictures now
        self.schedule_refresh()

    def open_media_dialog(self, cid: str) -> None:
        """Make Offline…, Create Proxies…, Project Manager…: the command's own refusal (nothing
        selected) comes first, as egui's."""
        cmd = self.registry.commands.get(cid)
        if cmd is not None and not cmd.enabled:
            self._command_failed(cid, tr("not available now"))
            return
        items = self.project_panel.selected_items()
        if cid == "media.makeOffline":
            d: QDialog = MakeOfflineDialog(self, self.engine, len(items), status=self._media_status, changed=self._media_changed)
        elif cid == "media.createProxies":
            d = CreateProxiesDialog(self, self.engine, items, status=self._media_status, changed=self._media_changed)
        else:
            project = self.project or {}
            path = project.get("path")
            name = str(project.get("name") or tr("Untitled"))
            dest = str(Path(path).parent / fmt("{name} (copy)", name=name)) if isinstance(path, str) and path else ""
            seq = (self.sequence or {}).get("id")
            d = ProjectManagerDialog(self, self.engine, sequences_of(project.get("root")), [seq] if isinstance(seq, int) else [], dest, status=self._media_status, changed=self._media_changed)
        d.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        self.last_dialog = d
        d.open()

    def pick_proxy_file(self, cid: str) -> None:
        """Attach Proxies… / Reconnect Full Resolution Media…: a file for the Project panel's
        first selected clip."""
        items = self.project_panel.selected_items()
        if not items:
            self._command_failed(cid, tr("select a clip in the Project panel"))
            return
        path, _f = QFileDialog.getOpenFileName(self, tr("Media"), "", tr("All files") + " (*)")
        if path:
            self.engine.execute(cid, {"item": items[0], "path": path}, on_reply=lambda _r, _b: self._media_changed(), on_error=lambda m: self._command_failed(cid, m))

    def _show_menu_dialog(self, state: dict[str, Any]) -> None:
        dialog = MenuDialog(self, state, lambda _c, _p, then: then(None, None))
        dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        self.last_dialog = dialog
        dialog.open()

    def open_sequence_settings(self, new: bool = False) -> None:
        """Sequence Settings… of the active sequence, or New Sequence (egui's
        ``panels/sequence_settings.rs``)."""
        if not new and self.sequence is None:
            self._command_failed("sequence.settings", tr("no sequence is open"))
            return

        def run(cid: str, params: dict[str, Any], then: Any) -> None:
            def failed(m: str) -> None:
                self._command_failed(cid, m)
                then(False)

            self.engine.execute(cid, params, on_reply=lambda _r, _b: (self.schedule_refresh(), then(True)), on_error=failed)

        def show(general: Any) -> None:
            samples = not (isinstance(general, dict) and general.get("audioDisplay") == "Milliseconds")
            if new:
                n = count_sequences((self.project or {}).get("root")) + 1
                draft = replace(SeqDraft(), new_sequence=True, name=f"Sequence {n:02}")
                cur_name = ""
            else:
                seq = self.sequence or {}
                cur_name = str(seq.get("name") or "")
                draft = seq_draft_from(seq.get("settings"), cur_name)
            dialog = SequenceSettingsDialog(self, draft, run, cur_name, samples)
            dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
            self.last_dialog = dialog
            dialog.open()

        self.engine.execute("file.projectSettings.general", {}, on_reply=lambda r, _b: show(r), on_error=lambda _m: show(None))

    def open_remix_properties(self) -> None:
        """Remix Properties… of the selected remixed clip (egui's: the menu entry opens the
        dialog; with parameters the command applies directly)."""

        def got(result: Any, _b: bytes | None) -> None:
            if not isinstance(result, dict) or not result.get("remixed"):
                self._command_failed("clip.remix.properties", tr("the selected clip is not remixed"))
                return

            def done(text: str) -> None:
                self.statusBar().showMessage(text, 6000)
                self.schedule_refresh()

            dialog = RemixPropertiesDialog(self, self.engine, result, done)
            dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
            self.last_dialog = dialog
            dialog.open()

        self.engine.execute("clip.remix.properties", {}, on_reply=got, on_error=lambda m: self._command_failed("clip.remix.properties", m))

    def open_edit_cameras(self) -> None:
        """Edit Cameras for the multi-camera clip at the shown playhead."""

        def got(result: Any, _b: bytes | None) -> None:
            if not isinstance(result, dict) or not isinstance(result.get("source"), int):
                self._command_failed("multicam.editCamerasDialog", tr("no multi-camera clip at the playhead"))
                return
            dialog = EditCamerasDialog(self, self.engine, result, self.multicam._changed)
            dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
            self.last_dialog = dialog
            dialog.open()

        self.engine.execute("multicam.inspect", {"time": int(self.monitor.playhead)}, on_reply=got, on_error=lambda m: self._command_failed("multicam.inspect", m))

    def _voiceover_changed(self, track: Any) -> None:
        self.timeline.canvas.voiceover_track = track
        self.timeline.canvas.update()
        self.mixer.set_voiceover(track is not None)

    def _voiceover_message(self, text: str, error: bool) -> None:
        self.statusBar().showMessage(text, 6000)
        if error:
            self.events.log(text, error=True)

    def open_settings(self, page: str = "general") -> PreferencesDialog:
        """Edit ▸ Preferences ▸ <category> (egui's ``app.settings.<category>``)."""
        dialog = PreferencesDialog(self, self.engine, page, devices=self.audio_devices, memory=self.memory_usage)
        dialog.applied.connect(self._settings_applied)
        dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        self.settings_dialog = dialog
        self.last_dialog = dialog
        dialog.open()
        return dialog

    def load_audio_output(self) -> None:
        """Play through Settings ▸ Audio Hardware ▸ Default Output (shared with egui)."""

        def got(result: Any, _data: bytes | None) -> None:
            value = result.get("value", result) if isinstance(result, dict) else None
            name = str(value.get("defaultOutput") or "") if isinstance(value, dict) else ""
            self.monitor.set_audio_output(name)
            self.source_monitor.set_audio_output(name)
            self.tts.audio_output = name

        self.engine.execute("prefs.get", {"key": "audioHardware"}, on_reply=got)

    def _settings_applied(self, changes: dict[str, Any]) -> None:
        self.load_appearance()
        self.load_audio_output()
        lang = changes.get("general.interfaceLanguage")
        if isinstance(lang, str):
            code = i18n_system_language() if lang == "system" else lang
            if code in LANGUAGES and code != TR.language:
                self.set_language(code)
        self.schedule_refresh()

    def audio_devices(self) -> dict[str, Any]:
        """The audio devices Qt reports, for Settings ▸ Audio Hardware."""
        try:
            from PySide6.QtMultimedia import QMediaDevices
        except ImportError:
            return {}
        outputs = QMediaDevices.audioOutputs()
        default = QMediaDevices.defaultAudioOutput()
        return {
            "hosts": [],
            "inputs": [d.description() for d in QMediaDevices.audioInputs()],
            "outputs": [d.description() for d in outputs],
            "outputChannels": default.maximumChannelCount() if not default.isNull() else 0,
        }

    def memory_usage(self) -> int:
        """Bytes of decoded pictures held for the Timeline, Project panel and Source monitor."""
        total = 0
        for media in (self.timeline.canvas.media, self.project_panel.media, self.source_monitor.media):
            for img in list(getattr(media, "thumbs", {}).values()):
                total += img.sizeInBytes() if img is not None else 0
        return total

    def set_theme(self, name: str) -> None:
        """View ▸ Appearance ▸ Darkest / Dark / Light (egui's view.theme.*): that theme, and the
        Appearance Mode of its family."""
        self.engine.execute(
            "prefs.set",
            {"key": "appearance.colorTheme", "value": name},
            on_reply=lambda _r, _b: self.load_appearance(),
            on_error=lambda m: self.events.log(m, error=True),
        )

    def next_appearance_mode(self) -> None:
        """View ▸ Appearance ▸ Next Appearance Mode: Auto, Light, Dark, then Auto again."""
        mode = str(self.appearance.get("appearanceMode") or "dark")
        nxt = {"auto": "light", "light": "dark"}.get(mode, "auto")
        self.engine.execute(
            "prefs.set",
            {"key": "appearance.appearanceMode", "value": nxt},
            on_reply=lambda _r, _b: self.load_appearance(),
            on_error=lambda m: self.events.log(m, error=True),
        )

    def _offer_recovery(self, cand: dict[str, Any]) -> None:
        """Unsaved changes of a session that died: Recover / Not Now / Discard (as egui)."""
        text = recovery_text(cand)
        box = QMessageBox(
            QMessageBox.Icon.Question,
            tr("Recover Unsaved Changes"),
            text,
            parent=self,
        )
        recover = box.addButton(tr("Recover"), QMessageBox.ButtonRole.AcceptRole)
        box.addButton(tr("Not Now"), QMessageBox.ButtonRole.RejectRole)
        discard = box.addButton(tr("Discard"), QMessageBox.ButtonRole.DestructiveRole)
        box.exec()
        cid = cand.get("id")
        if box.clickedButton() is recover:
            self.execute("file.recover", {"id": cid})
        elif box.clickedButton() is discard:
            self.execute("file.discardRecovery", {"id": cid})

    def _on_effects_list(self, result: Any, _b: bytes | None) -> None:
        self.effects.set_effects(result)
        self.mixer.set_effects(result)

    def _on_commands(self, result: Any, _binary: bytes | None) -> None:
        self.registry.load(result)
        self._build_menus()

    # ----- keyboard shortcuts -----
    def register_local_commands(self) -> None:
        """Give the engine's shortcut set the Qt-run commands (egui registers its UI commands the
        same way), then read their keys; the menus are built with them."""
        cmds = [{"id": cid, "label": label, "menu": menu, "shortcut": key or None} for cid, label, menu, key in self.local_commands()]
        self.local_keys = None
        ok = {"registered": False}

        def registered(_r: Any, _b: bytes | None) -> None:
            ok["registered"] = True

        self.engine.execute("shortcuts.registerExternal", {"commands": cmds}, on_reply=registered, on_error=lambda _m: None)
        self.engine.execute(
            "shortcuts.list", {}, on_reply=lambda r, _b: self._on_local_keys(r) if ok["registered"] else None, on_error=lambda _m: None
        )

    def _on_local_keys(self, rows: Any) -> None:
        ids = {c[0] for c in self.local_commands()}
        keys: dict[str, list[str]] = {}
        panel_keys: dict[str, list[tuple[str, str]]] = {}
        for row in rows if isinstance(rows, list) else []:
            if not isinstance(row, dict) or row.get("id") not in ids:
                continue
            binds = [b for b in row.get("shortcuts") or [] if isinstance(b, dict) and isinstance(b.get("keys"), str)]
            keys[row["id"]] = [b["keys"] for b in binds if not b.get("panel")]
            # a panel binding of the command's own application-wide key adds nothing
            scoped = [
                (b["keys"], b["panel"]) for b in binds if isinstance(b.get("panel"), str) and b["panel"] and b["keys"] not in keys[row["id"]]
            ]
            if scoped:
                panel_keys[row["id"]] = scoped
        self.local_keys = keys
        self.local_panel_keys = panel_keys
        if self.registry.commands:
            self._build_menus()

    def reload_shortcuts(self) -> None:
        """Keyboard Shortcuts changed a key: rebind the menus and the Qt-run commands."""
        if self.local_keys is not None:
            self.engine.execute("shortcuts.list", {}, on_reply=lambda r, _b: self._on_local_keys(r), on_error=lambda _m: None)
        self.engine.call("engine.commands", on_reply=self._on_commands)

    def open_shortcuts(self) -> ShortcutsDialog:
        """Edit ▸ Keyboard Shortcuts… (egui's ``shortcuts_dialog.rs``)."""
        if self.shortcuts_dialog is not None:
            self.shortcuts_dialog.raise_()
            self.shortcuts_dialog.activateWindow()
            return self.shortcuts_dialog
        dialog = ShortcutsDialog(self, self.engine, on_closed=lambda _ok: self.reload_shortcuts())
        dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        dialog.changed.connect(self.reload_shortcuts)

        def gone() -> None:
            self.shortcuts_dialog = None

        dialog.destroyed.connect(lambda *_a: gone())
        self.shortcuts_dialog = dialog
        self.last_dialog = dialog
        dialog.open()
        return dialog

    def _refresh_enabled(self) -> None:
        if self.engine.running:
            self.engine.call("engine.commands", on_reply=lambda r, _b: self.registry.update_enabled(r))

    def _on_engine_stopped(self, reason: str) -> None:
        self.status_engine.setText(tr("Engine stopped"))
        self.monitor.stop(notify=False)
        self.source_monitor.stop(notify=False)
        if self._closing:
            return
        self.events.log(f"{tr('The engine stopped')}: {reason}", error=True)
        # a fresh engine at once, on the project that was open; the dead one's unsaved changes are
        # in the recovery journal, which the new one offers (Recover / Not Now / Discard). An
        # engine that keeps dying is left stopped.
        now = time.monotonic()
        self._engine_restarts = [t for t in self._engine_restarts if now - t < RESTART_WINDOW_S]
        restarted = False
        if len(self._engine_restarts) < MAX_RESTARTS:
            self._engine_restarts.append(now)
            try:
                self.engine.restart(self.restart_args())
                restarted = True
            except EngineError as exc:
                reason = f"{reason}\n{exc}"
        if restarted:
            self.revision = -1
            self.clear_media_caches()
            self.status_engine.setText(tr("Engine restarted"))
            text = tr("The CFA4 engine stopped unexpectedly and was restarted. Unsaved changes are kept in the recovery journal; it offers to recover them.")
        else:
            text = tr("The CFA4 engine stopped unexpectedly. Unsaved changes are kept in the recovery journal; restart CFA4 to recover them.")
        # window-modal but non-blocking: a nested event loop here would freeze the GUI's own
        # handling of the stopped engine (and hang unattended runs); one message at a time
        old = getattr(self, "engine_stopped_box", None)
        if old is not None:
            try:
                old.close()
            except RuntimeError:
                pass  # already closed and deleted
        box = QMessageBox(QMessageBox.Icon.Critical, tr("CFA4"), text + f"\n\n{reason}", QMessageBox.StandardButton.Ok, self)
        box.setObjectName("dialog.engineStopped")
        box.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        box.open()
        self.engine_stopped_box = box

    def restart_args(self) -> list[str]:
        """The engine's start arguments for a restart: the project that was open (saved) instead
        of the one it started with; data directory and auto-save as before."""
        args: list[str] = []
        skip = False
        path = (self.project or {}).get("path")
        for a in self.engine.args:
            if skip:
                skip = False
                continue
            if isinstance(path, str) and path and a in ("--project", "--demo"):
                skip = a == "--project"
                continue
            args.append(a)
        if isinstance(path, str) and path:
            args += ["--project", path]
        return args

    def _on_event(self, ev: dict[str, Any]) -> None:
        kind = ev.get("event")
        if kind == "projectChanged":
            self.schedule_refresh()
        elif kind == "toast":
            msg = str(ev.get("message") or "")
            self.events.log(msg, error=bool(ev.get("error")))
            self.statusBar().showMessage(msg, 5000)
        elif kind == "openSequence":
            self.docks["Timeline"].raise_()
            self.schedule_refresh()
        elif kind == "openSource":
            self.docks["Source"].show()
            self.docks["Source"].raise_()
            self.active_monitor = "source"
            self.schedule_refresh()
        elif kind == "revealInProject":
            self.docks["Project"].raise_()

    # ----- state refresh -----
    def schedule_refresh(self) -> None:
        self._refresh_timer.start()
        self._enabled_timer.start()

    def refresh(self) -> None:
        if not self.engine.running:
            return
        self.engine.call("session.state", on_reply=self._on_state)
        self.engine.execute("project.inspect", {}, on_reply=self._on_project)
        self.engine.execute(
            "sequence.inspect", {}, on_reply=self._on_sequence, on_error=lambda _m: self._on_sequence(None, None)
        )
        self.engine.execute("history.list", {}, on_reply=lambda r, _b: self.history.set_history(r))
        self.engine.execute("source.inspect", {}, on_reply=self._on_source, on_error=lambda _m: self._on_source(None, None))

    def _on_state(self, result: Any, _b: bytes | None) -> None:
        self.state = result if isinstance(result, dict) else {}
        self.revision = int(self.state.get("revision") or 0)
        self._update_title()
        if self.state.get("linkMediaPrompt") is True and not self._link_media_open():
            # a project opened with missing media (egui's): Link Media asks what to do
            self.open_link_media()

    def _on_project(self, result: Any, _b: bytes | None) -> None:
        old = self.project or {}
        self.project = result if isinstance(result, dict) else None
        new = self.project or {}
        if (old.get("path"), old.get("name")) != (new.get("path"), new.get("name")):
            # another project: its item ids mean other media, so cached pictures are wrong
            self.clear_media_caches()
            self._project_generation += 1
        self.project_panel.set_project(self.project)
        self._update_title()

    def _on_sequence(self, result: Any, _b: bytes | None) -> None:
        self.sequence = result if isinstance(result, dict) else None
        if self.monitor.playing and self.sequence is not None:
            self.sequence["playhead"] = self.monitor.playhead
        self.timeline.set_sequence(self.sequence)
        self.monitor.set_sequence(self.sequence, self.revision)
        self.reference.set_sequence(self.sequence, self.revision)
        self.update_scopes()
        self.inspector.set_sequence(self.sequence)
        self.effect_controls.refresh((self.sequence or {}).get("selection"), (self.sequence or {}).get("transitionSelection"))
        self.monitor.overlay.refresh()
        self.update_lumetri()
        self.update_essential_sound()
        self.update_essential_graphics()
        self.essential_graphics.refresh_library()
        self.effects.refresh()
        self.tts.has_sequence = self.sequence is not None
        self.tts.follow_selection()  # also while hidden: a narration clip brings it forward
        self.media_browser.refresh()
        self.text_panel.set_sequence(self.sequence, self.monitor.playhead)
        self.update_inspectors()

    def _on_source(self, result: Any, _b: bytes | None) -> None:
        self.source_state = result if isinstance(result, dict) else None
        self.source_monitor.set_source(self.source_state, self.revision)
        if self.source_monitor.source is None and self.active_monitor == "source":
            self.active_monitor = "program"

    def _update_title(self) -> None:
        name = (self.project or {}).get("name") or tr("Untitled")
        dirty = "*" if self.state.get("dirty") else ""
        self.setWindowTitle(f"{name}{dirty} — CFA4")

    # ----- commands -----
    def execute(self, command: str, params: dict[str, Any] | None = None) -> None:
        """Run an engine command; errors go to the status bar and the Events panel."""
        if command == "playhead.set" and self.monitor.playing:
            return
        if command == "source.setPlayhead" and self.source_monitor.playing:
            return
        if command in EDITS_AT_PLAYHEAD and self.monitor.playing:
            # the engine's playhead is still where Program playback started: stopping sends the
            # shown frame first, so the edit lands where the timeline shows the playhead
            self.monitor.stop()
        self.engine.execute(
            command,
            params or {},
            on_reply=lambda _r, _b, c=command: self._command_done(c),
            on_error=lambda m, c=command: self._command_failed(c, m),
        )

    def _command_done(self, command: str) -> None:
        if command in SOURCE_COMMANDS:
            # the item keeps its id but shows other media (or comes back online): cached
            # thumbnails and waveforms keyed by that id are stale
            self.clear_media_caches()
        self.schedule_refresh()

    def clear_media_caches(self) -> None:
        """Forget every cached thumbnail and waveform (Timeline, Project panel, Source monitor):
        they are keyed by item id, which can now mean other media."""
        for media in (self.timeline.canvas.media, self.project_panel.media, self.source_monitor.media):
            media.clear()

    def _command_failed(self, command: str, message: str) -> None:
        text = f"{command}: {message}"
        self.events.log(text, error=True)
        self.statusBar().showMessage(text, 6000)

    def ui_command(self, cid: str, params: dict[str, Any] | None = None) -> bool:
        """egui's UI commands (voice-over, multi-camera, docking, …): True when ``cid`` was one."""
        if cid in dock_commands.COMMANDS:
            try:
                dock_commands.run(self, cid, dict(params or {}))
            except dock_commands.DockError as e:
                self._command_failed(cid, str(e))
            return True
        if cid == "view.compare.setReference":
            p = params or {}
            t = p.get("time")
            secs = p.get("seconds")
            if isinstance(t, int) and not isinstance(t, bool) and 0 <= t < 2**62:
                self.monitor.set_reference(t)
            elif isinstance(secs, (int, float)) and not isinstance(secs, bool) and 0 <= secs < 1e7 and math.isfinite(secs):
                self.monitor.set_reference(round(secs * TICKS_PER_SECOND))
            else:
                self.monitor.set_reference(self.monitor.playhead)
            return True
        if cid in TRACK_HEIGHT_COMMANDS:
            self.timeline.canvas.height_command(cid)
            return True
        if cid in GUIDE_COMMANDS:
            try:
                self.guide_command(cid, dict(params or {}))
            except ValueError as e:
                self._command_failed(cid, str(e))
            return True
        if cid == "voiceover.recordToggle":
            self.voiceover.toggle()
        elif cid == "voiceover.settingsDialog":
            self.open_voiceover_settings()
        elif cid == "multicam.toggleView":
            self.monitor.set_view(multicam=not self.monitor.mv.multicam)
        elif cid == "multicam.recordToggle":
            self.multicam.toggle_record()
        elif cid == "multicam.editCamerasDialog":
            self.open_edit_cameras()
        elif cid in ("app.keyboardShortcuts", "help.shortcuts"):
            self.open_shortcuts()
        elif cid in CLIP_DIALOGS and not params:
            # egui's clip_dialogs: from the menu the dialog; with parameters the command runs
            self.open_clip_dialog(cid)
        elif cid in EXPORT_DIALOGS and not (params or {}).get("path"):
            self.open_export_dialog(cid, dict(params or {}))
        elif cid in MENU_COMMANDS and not params:
            self.open_menu_dialog(cid)
        elif cid == "view.dynamicAudioWaveforms":
            on = (params or {}).get("on")
            self.dynamic_waveforms_action.setChecked(on if isinstance(on, bool) else not self.dynamic_waveforms_action.isChecked())
        elif cid == "help.systemCompatibilityReport":
            self.open_system_report()
        elif cid == "help.revealLogFiles":
            self.reveal_log_files()
        elif cid in ("edit.editOriginal", "file.reveal"):
            self.open_originals(cid, dict(params or {}))
        elif cid == "clip.generateAudioWaveform":
            self.generate_waveforms(dict(params or {}))
        elif (cid in EXPORT_SAVES or cid == "file.mediaPropertiesFile") and not (params or {}).get("path"):
            self.pick_and_run(cid, dict(params or {}))
        elif cid == "media.linkMedia" and not params:
            self.open_link_media(menu=True)
        elif cid in ("media.makeOffline", "media.createProxies", "file.projectManager") and not params:
            self.open_media_dialog(cid)
        elif cid in ("media.attachProxies", "media.reconnectFullRes") and not params:
            self.pick_proxy_file(cid)
        elif cid in ("sequence.settings", "file.newSequence") and not params:
            # egui's: from the menu the dialog; with parameters the engine command applies them
            self.open_sequence_settings(new=cid == "file.newSequence")
        else:
            return False
        return True

    def guide_command(self, cid: str, params: dict[str, Any]) -> dict[str, Any]:
        """egui's rulers and guides commands (``monitor_view.rs`` route) on ``params.monitor``
        ("program" / "source"), else the focused monitor; ``enabled`` sets a toggle."""
        name = params.get("monitor")
        mon: Monitor = self.source_monitor if name == "source" else self.monitor if name == "program" else self.focused_monitor()
        v = mon.mv

        def toggled(cur: bool) -> bool:
            on = params.get("enabled")
            return on if isinstance(on, bool) else not cur

        rest = cid.removeprefix("view.")
        if rest in ("showRulers", "showGuides", "lockGuides"):
            attr = {"showRulers": "show_rulers", "showGuides": "show_guides", "lockGuides": "lock_guides"}[rest]
            mon.set_view(**{attr: toggled(getattr(v, attr))})
            return {"monitor": mon.target, rest: getattr(v, attr)}
        if rest == "snapInProgramMonitor":
            self.monitor.set_view(snap=toggled(self.monitor.mv.snap))
            return {"snap": self.monitor.mv.snap}
        if rest == "clearGuides":
            return {"monitor": mon.target, "cleared": mon.clear_guides()}
        if rest == "addGuide":
            pos = params.get("position")
            if pos is None:
                mon.open_add_guide()
                return {"dialog": "addGuide"}
            if isinstance(pos, bool) or not isinstance(pos, (int, float)) or abs(pos) > 1e6 or not math.isfinite(pos):
                raise ValueError("`position` must be a number of frame pixels")
            mon.add_guide(params.get("orientation") != "horizontal", float(pos))
            return {"monitor": mon.target, "guides": len(v.guides)}
        if rest == "guideTemplates.save":
            if not v.guides:
                raise ValueError("there are no guides to save")
            d = mon.open_save_guides()
            if d is not None and isinstance(params.get("name"), str):
                d.name.setText(params["name"])
                d.save()
            return {"dialog": "saveGuides"}
        if rest == "guideTemplates.manage":
            mon.open_manage_guides()
            return {"dialog": "manageGuides"}
        raise ValueError(f"unknown guide command `{cid}`")

    def invoke(self, cid: str) -> None:
        if self.ui_command(cid):
            return
        cmd = self.registry.commands.get(cid)
        if cmd is not None:
            self.run_command(cmd)
        else:
            self.execute(cid, {})

    # ----- the timeline's context menus (panels/timeline_menus.py) -----
    def command_enabled(self, cid: str) -> bool:
        cmd = self.registry.commands.get(cid)
        return cmd is not None and cmd.enabled

    def run_menu_command(self, cid: str, params: dict[str, Any]) -> None:
        if params:
            self.run_preset(cid, params)
        else:
            self.invoke(cid)

    def menu_request(self, cid: str, params: dict[str, Any], then: Any) -> None:
        ok = [True]

        def failed(_m: str) -> None:
            ok[0] = False

        if cid:
            self.engine.execute(cid, params, on_reply=lambda _r, _b: None, on_error=failed)
        # replies come in request order: these enabled states follow the command above
        self.engine.call(
            "engine.commands",
            on_reply=lambda r, _b: (self.registry.update_enabled(r), then(ok[0])),
            on_error=lambda _m: then(ok[0]),
        )
        self.schedule_refresh()

    def edit_point_kind(self, dist_px: float, neighbour: bool, then: Any) -> None:
        def got(on: Any) -> None:
            # egui's selection_trim_kind with no modifier: Roll on the cut, else Ripple, when the
            # Trim preference lets the Selection tool choose; a plain trim otherwise
            then(("roll" if neighbour and dist_px <= ROLL_PX else "ripple") if on is True else "trim")

        self.engine.execute("prefs.get", {"key": "trim.selectionToolRollRipple"}, on_reply=lambda r, _b: got(r), on_error=lambda _m: got(False))

    def run_preset(self, cid: str, params: dict[str, Any]) -> None:
        """A panel runs ``cid`` with ``params`` already known (the monitor's Export Frame knows
        its monitor and time): ask only for what is still missing, such as the file."""
        if self.ui_command(cid, params):
            return
        cmd = self.registry.commands.get(cid)
        if cmd is None:
            self.execute(cid, params)
            return
        if self.interpret_footage(cid, params):
            return
        project_path = (self.project or {}).get("path")
        # a panel that already knows the file (an export dialog, a script) is not asked again
        asked = {} if params.get("path") else path_params(cmd, self, project_path if isinstance(project_path, str) else None) or {}
        if asked.get("__cancel__"):
            return
        self.engine.execute(
            cid,
            {**params, **asked},
            on_reply=lambda _r, _b: self.schedule_refresh(),
            on_error=lambda m: self._command_failed(cid, m),
        )

    def interpret_footage(self, cid: str, params: dict[str, Any]) -> bool:
        """Interpret Footage without settings opens its dialog (egui's), for the items given or
        the Project panel's selection. True when it did."""
        if cid != "clip.interpretFootage" or "colorSpace" in params or "pixelAspect" in params:
            return False
        items = [i for i in params.get("items") or self.project_panel.selected_items() or self.selected_clip_items() if isinstance(i, int)]
        dialog = InterpretFootageDialog(self, self.engine, items, self.execute)
        dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        self.last_dialog = dialog
        dialog.open()
        return True

    def selected_clip_items(self) -> list[int]:
        """The project items of the Timeline's selected clips, once each (egui's Interpret
        Footage without a Project panel selection)."""
        seq = self.sequence or {}
        chosen = {i for i in seq.get("selection") or [] if isinstance(i, int)}
        out: list[int] = []
        for kind in ("video", "audio"):
            for track in seq.get(kind) or [] if isinstance(seq.get(kind), list) else []:
                for it in (track.get("items") or []) if isinstance(track, dict) else []:
                    item = it.get("item") if isinstance(it, dict) else None
                    if isinstance(it, dict) and it.get("clip") in chosen and isinstance(item, int) and item not in out:
                        out.append(item)
        return out

    def flush_editors(self) -> None:
        """Send every value still in a panel's send delay (before a command, Save or a check of
        whether the project needs saving)."""
        for panel in (self.effect_controls, self.inspector, self.lumetri, self.essential_sound, self.notes):
            panel.flush()

    def run_command(self, cmd: Command, unsaved_ok: bool = False) -> None:
        """A menu, button or shortcut ran ``cmd``: ask for what it needs, then execute."""
        # an Effect Controls value still in its send delay goes first (Save must write it)
        self.flush_editors()
        if cmd.id in ("markers.markIn", "markers.markOut") and self.project_panel.hover_mark(cmd.id == "markers.markIn"):
            return
        if cmd.id == "edit.selectAll" and self.effect_controls.select_all_keyframes():
            return
        if self.monitor.playing and cmd.id.startswith(("playhead.", "edit.", "timeline.", "markers.")):
            self.monitor.stop()
        if cmd.id == "app.quit":
            self.close()
            return
        if cmd.id in ("file.exportMedia", "export.queue.add"):
            self.open_export()
            return
        if self.ui_command(cmd.id):
            # a menu item or key whose dialog or file manager is the frontend's (clip and menu
            # dialogs, Sequence Settings…, Reveal in Finder…), as from ``invoke``
            return
        if self.active_monitor == "source" and cmd.id in SOURCE_STEPS and self.source_monitor.has_content():
            step = SOURCE_STEPS[cmd.id]
            start, end = self.source_monitor.range()
            if step == "start":
                self.source_monitor.seek(start)
            elif step == "end":
                self.source_monitor.seek(end - self.source_monitor.frame())
            else:
                self.source_monitor.step(int(step))
            return
        if cmd.id in REPLACES_PROJECT and self.voiceover.busy:
            # a take is placed in the project it was recorded for, before that project goes
            self.voiceover.finish_then(lambda: self.run_command(cmd, unsaved_ok))
            return
        if cmd.id in REPLACES_PROJECT or cmd.id in ("file.closeProject", "file.revert"):
            # the pass recorded while playing (mixer automation, multi-camera cuts) belongs to this project
            self.monitor.stop()
        if cmd.id in REPLACES_PROJECT and not unsaved_ok:
            self.confirm_unsaved(lambda: self.run_command(cmd, unsaved_ok=True))
            return
        if cmd.id == "file.revert" and not unsaved_ok:
            # saving first would overwrite the version being reverted to: Revert or Cancel only
            self.confirm_unsaved(
                lambda: self.run_command(cmd, unsaved_ok=True),
                question="Discard the unsaved changes to '{name}' and revert to the saved version?",
                can_save=False,
            )
            return
        if (cmd.id in REPLACES_PROJECT or cmd.id in ("file.closeProject", "file.revert")) and self.export_dialog is not None:
            # its sequence id belongs to this project: item ids restart in the next one, so a stale
            # dialog could export a different sequence than it names. Reopening re-reads it.
            self.export_dialog.hide()
        if self.interpret_footage(cmd.id, {}):
            return
        if cmd.id in SYNC_DIALOGS:
            self.open_sync_dialog(SYNC_DIALOGS[cmd.id])
            return
        if cmd.id == "clip.remix.properties":
            self.open_remix_properties()
            return
        project_path = (self.project or {}).get("path")
        params = path_params(cmd, self, project_path if isinstance(project_path, str) else None)
        if params is not None:
            if params.get("__cancel__"):
                return
            self.engine.execute(
                cmd.id,
                params,
                on_reply=lambda _r, _b: self.schedule_refresh(),
                on_error=lambda m: self._command_failed(cmd.id, m),
            )
            return
        auto = self.targeted(cmd, {})
        if self.missing_params(cmd, auto):
            self._ask_params(cmd, values=auto)
            return
        if cmd.id.startswith("multicam."):
            # keys 1–9 cut at the shown frame while playing (a recording pass) and the grid shows it
            auto = self.multicam.timed(auto)
            self.engine.execute(cmd.id, auto, on_reply=lambda _r, _b: self.multicam._changed(), on_error=lambda m: self._command_failed(cmd.id, m))
            return

        def failed(message: str) -> None:
            # commands whose dialog lives in the frontend: ask for the parameters instead
            if cmd.param_specs:
                self._ask_params(cmd, message)
            else:
                self._command_failed(cmd.id, message)

        self.engine.execute(cmd.id, auto, on_reply=lambda _r, _b: self.schedule_refresh(), on_error=failed)

    def _ask_params(self, cmd: Command, error: str = "", values: dict[str, Any] | None = None) -> None:
        dlg = ParamDialog(cmd, self, error, values)
        if not dlg.exec():
            return
        params = dlg.params()
        self.engine.execute(
            cmd.id,
            params,
            on_reply=lambda _r, _b: self.schedule_refresh(),
            on_error=lambda m: self._ask_params(cmd, m, params),
        )

    def targeted(self, cmd: Command, params: dict[str, Any]) -> dict[str, Any]:
        """Commands that take a ``target`` act on the focused monitor (Mark In with the Source
        monitor focused marks the source clip), as in the egui editor."""
        if "target" not in params and any(p.name == "target" and "source" in p.hint for p in cmd.param_specs):
            return {**params, "target": self.active_monitor}
        return params

    def missing_params(self, cmd: Command, params: dict[str, Any]) -> list[str]:
        """Required parameters the GUI cannot fill in itself."""
        return [p.name for p in cmd.param_specs if not p.optional and p.name not in params]

    def focused_monitor(self) -> Monitor:
        if self.active_monitor == "source" and self.source_monitor.has_content():
            return self.source_monitor
        return self.monitor

    def _focus_changed(self, _old: QWidget | None, new: QWidget | None) -> None:
        if new is None:
            return
        if new is self.project_panel or self.project_panel.isAncestorOf(new):
            self._follow("project")
        elif new is self.timeline or self.timeline.isAncestorOf(new):
            self._follow("timeline")
        if new is self.source_monitor or self.source_monitor.isAncestorOf(new):
            self.active_monitor = "source"
            QTimer.singleShot(0, self, self.update_timecode)
        elif new is self.monitor or self.monitor.isAncestorOf(new) or self.timeline.isAncestorOf(new):
            self.active_monitor = "program"
            QTimer.singleShot(0, self, self.update_timecode)

    # ----- Info, Metadata, Markers, Project Notes -----
    def _follow(self, source: str) -> None:
        if source != self.inspector_source:
            self.inspector_source = source
            self.update_inspectors()

    def selected_item(self) -> int | None:
        """The item Info and Metadata show (egui's metadata::target): the Project panel's
        selection or the item of the clip selected in the Timeline, whichever was used last."""
        project = next(iter(self.project_panel.selected_items()), None)
        timeline = None
        selection = (self.sequence or {}).get("selection") or []
        if selection:
            for track in [*((self.sequence or {}).get("video") or []), *((self.sequence or {}).get("audio") or [])]:
                for c in track.get("items") or []:
                    if c.get("clip") == selection[0] and isinstance(c.get("item"), int):
                        timeline = c["item"]
        return (timeline if timeline is not None else project) if self.inspector_source == "timeline" else (project if project is not None else timeline)

    def update_inspectors(self) -> None:
        """Ask the engine for what Info, Metadata, Markers and Project Notes show."""
        if not self.engine.running:
            return
        item = self.selected_item()
        self.target_item = item

        def got(result: Any, _b: bytes | None, it: int | None = item) -> None:
            if it != self.target_item:
                return
            data = result if isinstance(result, dict) else None
            self.metadata.set_fields(it, data)
            self.item_info = data
            self.info.update_info(data, self.sequence, self.monitor.playhead)

        if item is None:
            got(None, None)
        else:
            self.engine.execute("metadata.get", {"item": item}, on_reply=got, on_error=lambda _m: got(None, None))
        self.engine.execute("state.inspect", {}, on_reply=self._on_state_inspect)
        self.engine.execute("project.notes", {}, on_reply=lambda r, _b: self.notes.set_notes(str((r or {}).get("text") or "")))
        self.update_timecode()
        self.update_jobs()
        self.update_mixer()
        self.update_clip_mixer()
        for editor in list(self.fx_editors.values()):
            editor.refresh()

    def show_error(self, summary: str) -> None:
        """egui's error window: playback stops, the error is shown with its log file, the
        project can be saved from there."""
        if self.monitor.playing:
            self.monitor.stop()
        if self.error_dialog is None:
            self.error_dialog = ErrorDialog(self, lambda: self.invoke("file.save"))
        self.error_dialog.show_error(crash.take_last() or summary, crash.log_path())

    def open_fx_editor(self, target: dict[str, Any]) -> FxEditor:
        """Open (or raise) the Clip / Track Fx Editor of ``{"clip", "effect"}`` or
        ``{"strip", "slot"}``, as egui's ``audio_fx_editor::open``."""
        key = target_key(target)
        editor = self.fx_editors.get(key)
        if editor is None:
            def query(params: dict[str, Any], on_reply: Any, on_error: Any) -> None:
                self.engine.execute("audio.fxEditor", params, on_reply=lambda r, _b: on_reply(r), on_error=on_error)

            editor = FxEditor(target, query, self)
            editor.now = lambda: int(self.monitor.playhead) if self.monitor.playing else None
            editor.command.connect(self.execute)
            editor.closed.connect(lambda ed: self.fx_editors.pop(ed.key, None) if self.fx_editors.get(ed.key) is ed else None)
            self.fx_editors[key] = editor
            editor.refresh()
        editor.show()
        editor.raise_()
        editor.activateWindow()
        return editor

    def update_mixer(self) -> None:
        """The Audio Track Mixer's strips at the playhead (``mixer.inspect``)."""
        if self._mixer_busy:
            self._mixer_again = True
            return
        if not self.engine.running or not self.mixer.isVisible() or self.sequence is None:
            return
        playhead = self.monitor.playhead
        self._mixer_busy = True

        def done(result: Any, _b: bytes | None) -> None:
            self._mixer_busy = False
            self.mixer.set_state(result, self.sequence, playhead)
            if getattr(self, "_mixer_again", False):
                self._mixer_again = False
                self.update_mixer()

        def failed(_m: str) -> None:
            self._mixer_busy = False

        self.engine.execute("mixer.inspect", {"time": playhead}, on_reply=done, on_error=failed)

    def update_clip_mixer(self) -> None:
        """The Audio Clip Mixer: the clip under the playhead on each audio track."""
        if self._clip_mixer_busy:
            self._clip_mixer_again = True
            return
        if not self.engine.running or not self.clip_mixer.isVisible() or self.sequence is None:
            return
        self._clip_mixer_busy = True

        def done(result: Any, _b: bytes | None) -> None:
            self._clip_mixer_busy = False
            self.clip_mixer.set_state(result)
            if getattr(self, "_clip_mixer_again", False):
                self._clip_mixer_again = False
                self.update_clip_mixer()

        def failed(_m: str) -> None:
            self._clip_mixer_busy = False

        self.engine.call("clipMixer.inspect", {"time": self.monitor.playhead}, on_reply=done, on_error=failed)

    def place_files(self, paths: list[str], time: int, track: Any) -> None:
        """Files dropped on the Timeline: import them (``mediaBrowser.import``), then place each
        new item there, one after the other."""

        def imported(result: Any, _b: bytes | None) -> None:
            items = [i for i in (result or {}).get("items") or [] if isinstance(i, int)]
            if items:  # one request: the engine places them one after another, as one undo step
                self.execute("timeline.place", {"items": items, "time": time, "track": track})
            self.schedule_refresh()

        self.engine.execute("mediaBrowser.import", {"paths": paths}, on_reply=imported,
                            on_error=lambda m: self._command_failed("mediaBrowser.import", m))

    def update_essential_sound(self) -> None:
        """The Essential Sound panel: the selected audio clips' audio type and settings."""
        if self._essential_busy:
            self._essential_again = True
            return
        if not self.engine.running or not self.essential_sound.isVisible():
            return
        self._essential_busy = True

        def done(result: Any, _b: bytes | None) -> None:
            self._essential_busy = False
            self.essential_sound.set_state(result)
            if getattr(self, "_essential_again", False):
                self._essential_again = False
                self.update_essential_sound()

        def failed(_m: str) -> None:
            self._essential_busy = False
            self._essential_again = False
            self.essential_sound.set_state(None)

        self.engine.execute("essentialSound.inspect", {}, on_reply=done, on_error=failed)

    def update_essential_graphics(self, time: int | None = None) -> None:
        """The Essential Graphics ▸ Edit tab: the selected graphic clip at the playhead (``time``
        while playing)."""
        if self._graphics_busy:
            self._graphics_again = True
            return
        if not self.engine.running or not self.essential_graphics.isVisible() or self.essential_graphics.scrubbing():
            return
        self._graphics_busy = True

        def done(result: Any, _b: bytes | None) -> None:
            self._graphics_busy = False
            self.essential_graphics.set_state(result)
            if getattr(self, "_graphics_again", False):
                self._graphics_again = False
                self.update_essential_graphics()

        def failed(_m: str) -> None:
            self._graphics_busy = False
            self._graphics_again = False
            self.essential_graphics.set_state(None)

        self.engine.call("graphics.inspect", {} if time is None else {"time": time}, on_reply=done, on_error=failed)

    def update_lumetri(self) -> None:
        """The Lumetri Color panel: the selected video clip's Lumetri values at the playhead."""
        if self._lumetri_busy:
            self._lumetri_again = True
            return
        if not self.engine.running or not self.lumetri.isVisible():
            return
        if self.sequence is None:
            self.lumetri.set_state(None)
            return
        self._lumetri_busy = True
        playhead = self.monitor.playhead

        def done(result: Any, _b: bytes | None) -> None:
            self._lumetri_busy = False
            self.lumetri.playhead = playhead
            self.lumetri.set_state(result)
            if getattr(self, "_lumetri_again", False):
                self._lumetri_again = False
                self.update_lumetri()

        def failed(_m: str) -> None:
            self._lumetri_busy = False
            self._lumetri_again = False

        self.engine.call("lumetri.inspect", {"time": playhead}, on_reply=done, on_error=failed)

    def _mixer_meters(self) -> None:
        """Each strip's peaks of the audio being heard (the Program monitor's blocks)."""
        dt = min(0.1, self._mixer_clock.restart() / 1000.0)
        if not self.mixer.isVisible() and not self.clip_mixer.isVisible():
            return
        mon = self.monitor
        peaks: dict[Any, list[float]] = {}
        if mon.playing:
            now = mon.play_sample()
            gen = getattr(mon, "audio_generation", None)
            last = self._mixer_heard[1] if self._mixer_heard is not None and self._mixer_heard[0] == gen else now - 1
            for start, frames, strips in mon.strip_blocks:
                if start <= now and start + frames > last:  # heard since the last tick
                    for sid, values in strips.items():
                        cur = peaks.setdefault(sid, [0.0] * len(values))
                        for i, v in enumerate(values[: len(cur)]):
                            cur[i] = max(cur[i], float(v or 0.0))
            self._mixer_heard = (gen, now)
        else:
            self._mixer_heard = None
        self.mixer.input_level = self.voiceover.take_level() if self.voiceover.recording else 0.0
        self.mixer.set_levels(peaks, dt)
        self.clip_mixer.set_levels(peaks, dt)

    def _mixer_transport(self, key: str) -> None:
        mon = self.monitor
        if key == "goIn":
            mon.go_to_in()
        elif key == "goOut":
            mon.go_to_out()
        elif key == "toggle":
            mon.toggle_playback()
        elif key == "inToOut":
            mon.play_in_to_out()
        elif key == "loop":
            mon.set_view(looping=not mon.mv.looping)
        elif key == "voiceover.recordToggle":
            self.voiceover.toggle()

    def update_timecode(self) -> None:
        """The Timecode panel's rows, read out by the engine (with the playing monitors' times)."""
        panel = self.timecode_panel
        if self._timecode_busy:
            self._timecode_again = True  # ask again when this reply is in
            return
        if not self.engine.running or not panel.isVisible():
            return
        panel.active = "source" if self.active_monitor == "source" and self.source_monitor.has_content() else "program"
        params = panel.request()
        if self.monitor.playing:
            params["time"] = self.monitor.playhead
        if self.source_monitor.playing:
            params["sourceTime"] = self.source_monitor.playhead
        self._timecode_busy = True

        def done(result: Any, _b: bytes | None) -> None:
            self._timecode_busy = False
            panel.set_values(result)
            if getattr(self, "_timecode_again", False):
                self._timecode_again = False
                self.update_timecode()

        def failed(_m: str) -> None:
            self._timecode_busy = False
            self._timecode_again = False

        self.engine.call("time.readout", params, on_reply=done, on_error=failed)

    def update_jobs(self) -> None:
        if self.engine.running and (self.progress.isVisible() or self.progress.poll.isActive()):
            self.engine.execute("jobs.list", {}, on_reply=lambda r, _b: self.progress.set_jobs(r))

    def _on_state_inspect(self, result: Any, _b: bytes | None) -> None:
        state = result if isinstance(result, dict) else {}
        self.hidden_markers = list(state.get("hidden_marker_colors") or [])
        self.markers.set_state(self.sequence, self.hidden_markers)

    # ----- playhead -----
    def _scrub(self, ticks: int) -> None:
        self.monitor.set_playhead(ticks)
        self.reference.set_playhead(ticks)

    def _seek_program(self, ticks: int) -> None:
        """A panel (Effect Controls' keyframe navigation, a marker) moves the Program playhead.
        Playback stops first: its clock would put the playhead back on the next tick, and the
        engine's playhead is not set while playing."""
        if self.monitor.playing:
            self.monitor.stop(notify=False)
        self.monitor.set_playhead(ticks)
        self.reference.set_playhead(ticks)
        self.update_scopes(int(ticks))
        self.execute("playhead.set", {"time": int(ticks)})

    def update_scopes(self, ticks: int | None = None) -> None:
        """The Lumetri Scopes follow the Program frame."""
        if self.sequence is None:
            self.scopes.set_frame(None, self.revision)
            return
        t = self.monitor.playhead if ticks is None else ticks
        ft = self.monitor.frame()
        # with the sequence: opening another one doesn't change the revision
        self.scopes.set_frame(t // ft * ft, (self.sequence.get("id"), self.revision))

    def _playback_moved(self, ticks: int) -> None:
        self.update_scopes(ticks)
        self.reference.set_playhead(ticks)
        self.timeline.canvas.playhead = ticks
        self.timeline.canvas.update()
        if self.info.isVisible():
            self.info.update_info(getattr(self, "item_info", None), self.sequence, ticks)
        self.update_timecode()
        self.update_mixer()
        self.update_clip_mixer()
        self.update_lumetri()
        if self.essential_graphics.isVisible() and not self.essential_graphics.visibleRegion().isEmpty():
            self.update_essential_graphics(ticks)
        if self.text_panel.isVisible():
            self.text_panel.set_playhead(ticks)

    # ----- export -----
    def open_export(self) -> None:
        if self.export_dialog is None:
            self.export_dialog = ExportDialog(self.engine, self)
            self.export_dialog.finished_export.connect(self._export_finished)
        path = (self.project or {}).get("path")
        self.export_dialog.open_for(self.sequence, path if isinstance(path, str) else None)

    def _export_finished(self, path: str, ok: bool) -> None:
        text = (tr("Export finished: {path}") if ok else tr("Export failed: {path}")).replace("{path}", path)
        self.events.log(text, error=not ok)
        self.statusBar().showMessage(text, 10_000)

    # ----- misc -----
    def _about(self) -> None:
        box = QMessageBox(self)
        box.setObjectName("dialog.about")
        box.setWindowTitle(tr("About CFA4"))
        box.setIconPixmap(logo_pixmap(180))
        box.setStyleSheet("QLabel#qt_msgbox_label { min-width: 280px; }")
        box.setText(
            f"<b>CFA4</b> — CinemaFundArchitecture4<br>Qt 6 GUI {__version__}<br>{self.engine.hello.get('engine', '')}<br><br>"
            "GPL-3.0-or-later<br>Rust engine · Qt 6 / PySide6 · Ubuntu"
        )
        box.open()

    def confirm_unsaved(
        self,
        proceed: Callable[[], None],
        discard: Callable[[], None] | None = None,
        question: str = "Save changes to '{name}' before continuing?",
        can_save: bool = True,
    ) -> None:
        """Run ``proceed`` once unsaved changes are saved, or ``discard`` (default: ``proceed``)
        when the user doesn't save them; nothing on Cancel. The dirty flag comes from the engine,
        asked after every command already queued, not from the last (possibly stale) refresh;
        an engine that doesn't answer within ``STATE_TIMEOUT_MS`` falls back to the last refresh.
        Without ``can_save`` the choice is only Discard or Cancel (Cancel is the default)."""
        asked = [False]

        def decide(result: Any, _b: bytes | None) -> None:
            if asked[0]:
                return
            asked[0] = True
            self._on_state(result, _b)
            if not self.state.get("dirty"):
                proceed()
                return
            name = (self.project or {}).get("name") or tr("Untitled")
            box = QMessageBox(
                QMessageBox.Icon.Question,
                tr("CFA4"),
                tr(question).replace("{name}", str(name)),
                parent=self,
            )
            box.setObjectName("dialog.unsaved")
            if can_save:
                box.setStandardButtons(
                    QMessageBox.StandardButton.Save
                    | QMessageBox.StandardButton.Discard
                    | QMessageBox.StandardButton.Cancel
                )
            else:
                box.setStandardButtons(QMessageBox.StandardButton.Discard | QMessageBox.StandardButton.Cancel)
                box.setDefaultButton(QMessageBox.StandardButton.Cancel)
            answer = box.exec()
            if answer == QMessageBox.StandardButton.Discard:
                (discard or proceed)()
                return
            if answer != QMessageBox.StandardButton.Save or not can_save:
                return
            cmd = self.registry.commands.get("file.save") or Command("file.save", "Save", [], "", "{}", True)
            project_path = (self.project or {}).get("path")
            params = path_params(cmd, self, project_path if isinstance(project_path, str) else None)
            if params and params.get("__cancel__"):
                return

            def saved(_r: Any, _b: bytes | None) -> None:
                self.schedule_refresh()
                proceed()

            self.engine.execute(
                "file.save", params or {}, on_reply=saved, on_error=lambda m: self._command_failed("file.save", m)
            )

        def unanswered(_message: str = "") -> None:
            decide(dict(self.state), None)

        # an edit still in Effect Controls' send delay reaches the engine before it is asked
        self.flush_editors()
        self.engine.call("session.state", on_reply=decide, on_error=unanswered)
        QTimer.singleShot(STATE_TIMEOUT_MS, unanswered)

    def event(self, e: QEvent) -> bool:
        # Enter is also an editor shortcut (Render Effects In to Out): in a text field, a spin box
        # or a table cell editor it confirms the typing instead (the field ignores the override,
        # so it reaches the window)
        if e.type() == QEvent.Type.ShortcutOverride and isinstance(e, QKeyEvent) and e.key() in (Qt.Key.Key_Return, Qt.Key.Key_Enter):
            w = QApplication.focusWidget()
            if isinstance(w, QLineEdit | QAbstractSpinBox | QPlainTextEdit | QTextEdit) and not (
                isinstance(w, QTextEdit | QPlainTextEdit) and w.isReadOnly()
            ):
                e.accept()
                return True
        return super().event(e)

    def closeEvent(self, e: QCloseEvent) -> None:
        if self.engine.running and not self._quit_confirmed and self.voiceover.busy:
            # the take goes into the project first, so the unsaved-changes check includes it
            e.ignore()
            self.voiceover.finish_then(self.close)
            return
        if self.engine.running and not self._quit_confirmed:
            e.ignore()
            # a mixer automation (or multi-camera) pass is written when playback stops: stop first,
            # so the unsaved-changes check (asked after it) includes the pass
            self.monitor.stop()
            self.source_monitor.stop()

            def close_now() -> None:
                self._quit_confirmed = True
                self.close()

            def discard_and_close() -> None:
                # Don't Save: close the project, which also clears its recovery snapshot (as egui does)
                self.engine.execute("file.closeProject", {"force": True})
                close_now()

            self.confirm_unsaved(close_now, discard_and_close, "Save changes to '{name}' before closing?")
            return
        self._closing = True
        self.monitor.stop(notify=False)
        self.source_monitor.stop(notify=False)
        self.settings.setValue("geometry", self.saveGeometry())
        self.settings.setValue("windowState", self.saveState(LAYOUT_VERSION))
        self.engine.stop()
        e.accept()
