"""CFA4 Qt 6 main window: docks, menus from the engine's command registry, and the glue
between panels and the engine. The window holds views of engine state only; every edit is an
engine command and every redraw follows the engine's ``projectChanged``."""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QByteArray, QElapsedTimer, QSettings, Qt, QTimer
from PySide6.QtGui import QAction, QActionGroup, QCloseEvent, QKeySequence
from PySide6.QtWidgets import (
    QApplication,
    QDockWidget,
    QLabel,
    QMainWindow,
    QMenu,
    QMessageBox,
    QTabBar,
    QTabWidget,
    QWidget,
)

from . import __version__, theme
from .branding import logo_pixmap
from .commands import Command, CommandRegistry
from .dialogs import ParamDialog, path_params
from .engine import EngineClient
from .i18n import LANGUAGES, N_, TR, tr
from .panels.effect_controls import EffectControlsPanel
from .panels.export import ExportDialog
from .panels.info import InfoPanel, MarkersPanel, MetadataPanel, NotesPanel
from .panels.meters import AudioMetersPanel
from .panels.mixer import TrackMixerPanel
from .panels.monitor import Monitor, ProgramMonitor, SourceMonitor
from .panels.monitor_view import build_view_menu
from .panels.project import ProjectPanel
from .panels.readouts import ProgressPanel, TimecodePanel
from .panels.side import EffectsPanel, EventsPanel, HistoryPanel, InspectorPanel
from .panels.timeline import TimelinePanel
from .panels.tools import ToolsPanel
from .window_menu import Workspaces, bind_free, build_window_menu

LAYOUT_VERSION = 10  # 2: Source monitor; 3: Effect Controls; 4: Properties (egui name); 5: proportional sizes; 6: Tools panel; 7: Audio Meters; 8: Info, Markers, Metadata, Project Notes, egui's groups; 9: Timecode, Progress; 10: Audio Track Mixer

# panels docked around the Program monitor, in the order they are created
DOCK_PLAN = (
    ("Source", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Effect Controls", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Metadata", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Project", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Info", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Effects", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Markers", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Project Notes", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Timecode", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Audio Track Mixer", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Progress", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Properties", Qt.DockWidgetArea.RightDockWidgetArea),
    ("History", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Tools", Qt.DockWidgetArea.BottomDockWidgetArea),
    ("Timeline", Qt.DockWidgetArea.BottomDockWidgetArea),
    ("Events", Qt.DockWidgetArea.BottomDockWidgetArea),
    ("Audio Meters", Qt.DockWidgetArea.BottomDockWidgetArea),
)
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


class MainWindow(QMainWindow):
    def __init__(self, engine: EngineClient, settings: QSettings | None = None) -> None:
        super().__init__()
        self.setObjectName("cfa4.main")
        self.engine = engine
        self.settings = settings or QSettings("CFA4", "CFA4-Qt")
        self.registry = CommandRegistry(self.run_command)
        self.project: dict[str, Any] | None = None
        self.sequence: dict[str, Any] | None = None
        self.state: dict[str, Any] = {}
        self.revision = -1
        self.source_state: dict[str, Any] | None = None
        self.export_dialog: ExportDialog | None = None
        # the monitor that keyboard commands with a `target` act on (the focused panel, as egui)
        self.active_monitor = "program"
        self._closing = False
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
        for mon in (self.monitor, self.source_monitor):
            mon.attach_settings(self.settings)
            self.tools.tool_selected.connect(lambda tool, m=mon: m.view.set_hand(tool == "hand"))
            mon.run.connect(self.run_preset)
            mon.message.connect(lambda m: self.events.log(m, error=True))
        self.meters = AudioMetersPanel(self)
        self.meters.watch(self.monitor, self.source_monitor)
        self.project_panel = ProjectPanel(self)
        self.effects = EffectsPanel(self)
        self.inspector = InspectorPanel(self)
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
        self._mixer_busy = False
        self._mixer_clock = QElapsedTimer()
        self._mixer_clock.start()
        self._mixer_heard: int | None = None
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
        }
        for name, area in DOCK_PLAN:
            self._dock(name, widgets[name], area)
        # The window itself only ever gets its tab groups through restoreState() (see
        # _default_layout_state and docs/qt-gui.md, "Known Qt issues").
        self._default_state = self._default_layout_state()
        self.workspaces = Workspaces(self, self.settings, self._default_state, LAYOUT_VERSION)

        self.status_engine = QLabel(tr("Starting engine…"))
        self.status_engine.setObjectName("status.engine")
        self.statusBar().addPermanentWidget(self.status_engine)

        for panel in (
            self.monitor,
            self.source_monitor,
            self.timeline.canvas,
            self.project_panel,
            self.effects,
            self.inspector,
            self.effect_controls,
            self.metadata,
            self.notes,
            self.progress,
            self.mixer,
        ):
            panel.command.connect(self.execute)
        self.markers.command.connect(lambda c, p: self._seek_program(p["time"]) if c == "playhead.set" else self.execute(c, p))
        self.effect_controls.seek.connect(self._seek_program)
        app = QApplication.instance()
        if isinstance(app, QApplication):
            app.focusChanged.connect(self._focus_changed)
        self.project_panel.invoke.connect(self.invoke)
        self.timeline.canvas.seek.connect(self._scrub)
        self.monitor.playhead_moved.connect(self._playback_moved)
        self.source_monitor.playhead_moved.connect(lambda _t: self.update_timecode())
        self.docks["Timecode"].visibilityChanged.connect(lambda shown: shown and self.update_timecode())
        self.docks["Progress"].visibilityChanged.connect(lambda shown: shown and self.update_jobs())
        self.docks["Audio Track Mixer"].visibilityChanged.connect(lambda shown: shown and self.update_mixer())

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
        win.tabifyDockWidget(d["Effect Controls"], d["Metadata"])
        win.tabifyDockWidget(d["Metadata"], d["Audio Track Mixer"])
        d["Audio Track Mixer"].hide()  # in egui's Audio workspace; Window ▸ Audio Track Mixer
        win.tabifyDockWidget(d["Project"], d["Info"])
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
        for name in ("Source", "Project", "Properties", "Timeline"):
            d[name].raise_()
        win.resizeDocks([d["Timeline"]], [300], Qt.Orientation.Vertical)
        win.resizeDocks([d["Tools"], d["Audio Meters"]], [TOOLS_W, METERS_W], Qt.Orientation.Horizontal)
        win.resizeDocks([d["Source"], d["Properties"]], [460, 280], Qt.Orientation.Horizontal)
        win.resizeDocks([d["Source"], d["Project"]], [320, 300], Qt.Orientation.Vertical)

    def _default_layout_state(self) -> QByteArray:
        """The default layout, arranged in a hidden stand-in window with empty docks of the
        same names: tabifying the real window's docks directly leaves orphaned tab bars on screen
        (Qt 6.10.3+) or can crash once it is shown (Qt 6.10.2); restoring a state is clean."""
        helper = QMainWindow()
        helper.setDockOptions(self.dockOptions())
        helper.setTabPosition(Qt.DockWidgetArea.AllDockWidgetAreas, QTabWidget.TabPosition.North)
        helper.setCentralWidget(QWidget(helper))
        helper.resize(1440, 900)
        stand_ins: dict[str, QDockWidget] = {}
        for name, area in DOCK_PLAN:
            dock = QDockWidget(name, helper)
            dock.setObjectName(f"dock.{name}")
            dock.setWidget(QWidget(dock))
            helper.addDockWidget(area, dock)
            stand_ins[name] = dock
        self.arrange_default(helper, stand_ins)
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
        self._retire_tab_bars(before)
        QTimer.singleShot(0, self, lambda: self._retire_tab_bars(before))
        self._titles_later()

    def _retire_tab_bars(self, old: set[QTabBar]) -> None:
        """restoreState() on a window that is on screen makes new tab bars and leaves the old
        ones visible under or over them (Qt 6.10.3, 6.11, 6.12). Hide an old bar when a new
        bar shows the same tabs; a stale bar on top would swallow the clicks on the tabs."""
        def tabs(bar: QTabBar) -> tuple[str, ...]:
            return tuple(bar.tabText(i) for i in range(bar.count()))

        visible = [b for b in self.findChildren(QTabBar) if b.parent() is self and b.isVisible()]
        fresh = {tabs(b) for b in visible if b not in old}
        for bar in visible:
            if bar in old and (bar.count() == 0 or tabs(bar) in fresh):
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
        self.shuttle_actions: list[tuple[QAction, list[str]]] = []
        for name, label, keys, slot in (
            ("reverse", N_("Shuttle Left"), ["J"], lambda: self.focused_monitor().shuttle(-1)),
            ("stop", N_("Shuttle Stop"), ["K"], lambda: self.focused_monitor().stop()),
            ("forward", N_("Shuttle Right"), ["L"], lambda: self.focused_monitor().shuttle(1)),
            ("inToOut", N_("Play In to Out"), ["Alt+K"], lambda: self.focused_monitor().play_in_to_out()),
        ):
            a = QAction(tr(label), self)
            a.setObjectName(f"qt.playback.{name}")
            a.triggered.connect(slot)
            self.shuttle_actions.append((a, keys))
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

    def _bind_local_shortcuts(self, bound_keys: set[str]) -> None:
        """Playback and tool keys, unless an engine command already claims them."""
        for action, keys in (
            (self.play_action, ["Space"]),
            (self.export_action, ["Ctrl+M"]),
            (self.quit_action, ["Ctrl+Q"]),
            *self.shuttle_actions,
        ):
            free = [QKeySequence(k) for k in keys if k not in bound_keys]
            action.setShortcuts(free)
            self.addAction(action)

    def _build_menus(self) -> None:
        self.registry.build_menus(
            self.menuBar(), self, {
                "File": [self.quit_action],
                "Sequence": [self.play_action, *(a for a, _k in self.shuttle_actions)],
                "View": [self.fit_action],
                "Help": [self.about_action],
            }
        )
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
        keys = self.registry.claimed_keys
        self._bind_local_shortcuts(keys)
        bind_free(self, self.tools.shortcut_pairs(), keys)  # V, A, B, N, R, C, Y, U, H, Z ... as egui
        self._window_actions = bind_free(self, window_shortcuts, keys) + [
            a for a in window_menu.actions() if a not in {p[0] for p in window_shortcuts}
        ]

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
        self.engine.call("engine.commands", on_reply=self._on_commands)
        self.engine.execute("effects.list", {}, on_reply=self._on_effects_list)
        self.load_appearance()
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

        self.engine.execute("prefs.get", {"key": "appearance"}, on_reply=got)

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
        if isinstance(result, list):
            self.inspector.effect_names = {
                str(e["id"]): str(e.get("name") or e["id"]) for e in result if isinstance(e, dict) and e.get("id")
            }

    def _on_commands(self, result: Any, _binary: bytes | None) -> None:
        self.registry.load(result)
        self._build_menus()

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
        # window-modal but non-blocking: a nested event loop here would freeze the GUI's own
        # handling of the stopped engine (and hang unattended runs)
        box = QMessageBox(
            QMessageBox.Icon.Critical,
            tr("CFA4"),
            tr(
                "The CFA4 engine stopped unexpectedly. Unsaved changes are kept in the recovery journal; restart CFA4 to recover them."
            )
            + f"\n\n{reason}",
            QMessageBox.StandardButton.Ok,
            self,
        )
        box.setObjectName("dialog.engineStopped")
        box.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        box.open()
        self.engine_stopped_box = box

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

    def _on_project(self, result: Any, _b: bytes | None) -> None:
        old = self.project or {}
        self.project = result if isinstance(result, dict) else None
        new = self.project or {}
        if (old.get("path"), old.get("name")) != (new.get("path"), new.get("name")):
            # another project: its item ids mean other media, so cached pictures are wrong
            self.timeline.canvas.media.clear()
        self.project_panel.set_project(self.project)
        self._update_title()

    def _on_sequence(self, result: Any, _b: bytes | None) -> None:
        self.sequence = result if isinstance(result, dict) else None
        if self.monitor.playing and self.sequence is not None:
            self.sequence["playhead"] = self.monitor.playhead
        self.timeline.set_sequence(self.sequence)
        self.monitor.set_sequence(self.sequence, self.revision)
        self.inspector.set_sequence(self.sequence)
        self.effect_controls.refresh((self.sequence or {}).get("selection"))
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
        self.engine.execute(
            command,
            params or {},
            on_reply=lambda _r, _b: self.schedule_refresh(),
            on_error=lambda m, c=command: self._command_failed(c, m),
        )

    def _command_failed(self, command: str, message: str) -> None:
        text = f"{command}: {message}"
        self.events.log(text, error=True)
        self.statusBar().showMessage(text, 6000)

    def invoke(self, cid: str) -> None:
        cmd = self.registry.commands.get(cid)
        if cmd is not None:
            self.run_command(cmd)
        else:
            self.execute(cid, {})

    def run_preset(self, cid: str, params: dict[str, Any]) -> None:
        """A panel runs ``cid`` with ``params`` already known (the monitor's Export Frame knows
        its monitor and time): ask only for what is still missing, such as the file."""
        cmd = self.registry.commands.get(cid)
        if cmd is None:
            self.execute(cid, params)
            return
        project_path = (self.project or {}).get("path")
        asked = path_params(cmd, self, project_path if isinstance(project_path, str) else None) or {}
        if asked.get("__cancel__"):
            return
        self.engine.execute(
            cid,
            {**params, **asked},
            on_reply=lambda _r, _b: self.schedule_refresh(),
            on_error=lambda m: self._command_failed(cid, m),
        )

    def run_command(self, cmd: Command, unsaved_ok: bool = False) -> None:
        """A menu, button or shortcut ran ``cmd``: ask for what it needs, then execute."""
        # an Effect Controls value still in its send delay goes first (Save must write it)
        self.effect_controls.flush()
        self.notes.flush()
        if self.monitor.playing and cmd.id.startswith(("playhead.", "edit.", "timeline.", "markers.")):
            self.monitor.stop()
        if cmd.id == "app.quit":
            self.close()
            return
        if cmd.id in ("file.exportMedia", "export.queue.add"):
            self.open_export()
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

    def _mixer_meters(self) -> None:
        """Each strip's peaks of the audio being heard (the Program monitor's blocks)."""
        dt = min(0.1, self._mixer_clock.restart() / 1000.0)
        if not self.mixer.isVisible():
            return
        mon = self.monitor
        peaks: dict[Any, list[float]] = {}
        if mon.playing:
            now = mon.play_sample()
            last = self._mixer_heard if self._mixer_heard is not None else now - 1
            for start, frames, strips in mon.strip_blocks:
                if start <= now and start + frames > last:  # heard since the last tick
                    for sid, values in strips.items():
                        cur = peaks.setdefault(sid, [0.0] * len(values))
                        for i, v in enumerate(values[: len(cur)]):
                            cur[i] = max(cur[i], float(v or 0.0))
            self._mixer_heard = now
        else:
            self._mixer_heard = None
        self.mixer.set_levels(peaks, dt)

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

    def _seek_program(self, ticks: int) -> None:
        """A panel (Effect Controls' keyframe navigation) moves the Program playhead."""
        self.monitor.set_playhead(ticks)
        self.execute("playhead.set", {"time": int(ticks)})

    def _playback_moved(self, ticks: int) -> None:
        self.timeline.canvas.playhead = ticks
        self.timeline.canvas.update()
        if self.info.isVisible():
            self.info.update_info(getattr(self, "item_info", None), self.sequence, ticks)
        self.update_timecode()
        self.update_mixer()

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
        self.effect_controls.flush()
        self.notes.flush()
        self.engine.call("session.state", on_reply=decide, on_error=unanswered)
        QTimer.singleShot(STATE_TIMEOUT_MS, unanswered)

    def closeEvent(self, e: QCloseEvent) -> None:
        if self.engine.running and not self._quit_confirmed:
            e.ignore()

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
