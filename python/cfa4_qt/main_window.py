"""CFA4 Qt 6 main window: docks, menus from the engine's command registry, and the glue
between panels and the engine. The window holds views of engine state only; every edit is an
engine command and every redraw follows the engine's ``projectChanged``."""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QByteArray, QSettings, Qt, QTimer
from PySide6.QtGui import QAction, QActionGroup, QCloseEvent, QKeySequence
from PySide6.QtWidgets import QApplication, QDockWidget, QLabel, QMainWindow, QMessageBox, QTabBar, QWidget

from . import __version__
from .branding import logo_pixmap
from .commands import Command, CommandRegistry
from .dialogs import ParamDialog, path_params
from .engine import EngineClient
from .i18n import LANGUAGES, N_, TR, tr
from .panels.export import ExportDialog
from .panels.monitor import Monitor, ProgramMonitor, SourceMonitor
from .panels.project import ProjectPanel
from .panels.side import EffectsPanel, EventsPanel, HistoryPanel, InspectorPanel
from .panels.timeline import TimelinePanel

LAYOUT_VERSION = 2  # 2: Source monitor dock

# panels docked around the Program monitor, in the order they are created
DOCK_PLAN = (
    ("Source", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Project", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Effects", Qt.DockWidgetArea.LeftDockWidgetArea),
    ("Inspector", Qt.DockWidgetArea.RightDockWidgetArea),
    ("History", Qt.DockWidgetArea.RightDockWidgetArea),
    ("Timeline", Qt.DockWidgetArea.BottomDockWidgetArea),
    ("Events", Qt.DockWidgetArea.BottomDockWidgetArea),
)

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

        self.monitor = ProgramMonitor(engine, self)
        self.source_monitor = SourceMonitor(engine, self)
        self.timeline = TimelinePanel(self)
        self.project_panel = ProjectPanel(self)
        self.effects = EffectsPanel(self)
        self.inspector = InspectorPanel(self)
        self.history = HistoryPanel(self)
        self.events = EventsPanel(self)

        self.setCentralWidget(self.monitor)
        self.setDockOptions(
            QMainWindow.DockOption.AllowTabbedDocks
            | QMainWindow.DockOption.AllowNestedDocks
            | QMainWindow.DockOption.AnimatedDocks
        )
        self.docks: dict[str, QDockWidget] = {}
        widgets = {
            "Source": self.source_monitor,
            "Project": self.project_panel,
            "Effects": self.effects,
            "Inspector": self.inspector,
            "History": self.history,
            "Timeline": self.timeline,
            "Events": self.events,
        }
        for name, area in DOCK_PLAN:
            self._dock(name, widgets[name], area)
        # The window itself only ever gets its tab groups through restoreState() (see
        # _default_layout_state and docs/qt-gui.md, "Known Qt issues").
        self._default_state = self._default_layout_state()

        self.status_engine = QLabel(tr("Starting engine…"))
        self.status_engine.setObjectName("status.engine")
        self.statusBar().addPermanentWidget(self.status_engine)

        for panel in (self.monitor, self.source_monitor, self.timeline.canvas, self.project_panel, self.effects, self.inspector):
            panel.command.connect(self.execute)
        app = QApplication.instance()
        if isinstance(app, QApplication):
            app.focusChanged.connect(self._focus_changed)
        self.project_panel.invoke.connect(self.invoke)
        self.timeline.canvas.seek.connect(self._scrub)
        self.monitor.playhead_moved.connect(self._playback_moved)

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
        if not (isinstance(state, QByteArray) and self.restoreState(state, LAYOUT_VERSION)):
            self.restoreState(self._default_state, LAYOUT_VERSION)
        self._update_title()

    @staticmethod
    def arrange_default(win: QMainWindow, d: dict[str, QDockWidget]) -> None:
        """Source over Project/Effects on the left, Program in the middle, Inspector/History on
        the right, Timeline/Events at the bottom."""
        win.splitDockWidget(d["Source"], d["Project"], Qt.Orientation.Vertical)
        win.tabifyDockWidget(d["Project"], d["Effects"])
        win.tabifyDockWidget(d["Inspector"], d["History"])
        win.tabifyDockWidget(d["Timeline"], d["Events"])
        for name in ("Project", "Inspector", "Timeline"):
            d[name].raise_()
        win.resizeDocks([d["Timeline"]], [300], Qt.Orientation.Vertical)
        win.resizeDocks([d["Source"], d["Inspector"]], [460, 280], Qt.Orientation.Horizontal)
        win.resizeDocks([d["Source"], d["Project"]], [320, 300], Qt.Orientation.Vertical)

    def _default_layout_state(self) -> QByteArray:
        """The default layout, arranged in a hidden stand-in window with empty docks of the
        same names: tabifying the real window's docks directly leaves orphaned tab bars on screen
        (Qt 6.10.3+) or can crash once it is shown (Qt 6.10.2); restoring a state is clean."""
        helper = QMainWindow()
        helper.setDockOptions(self.dockOptions())
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
        """View ▸ Reset Panel Layout."""
        before = set(self.findChildren(QTabBar))
        self.restoreState(self._default_state, LAYOUT_VERSION)
        self._retire_tab_bars(before)
        QTimer.singleShot(0, self, lambda: self._retire_tab_bars(before))

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
        return dock

    # ----- actions owned by the Qt GUI (view, playback, help) -----
    def _build_local_actions(self) -> None:
        self.play_action = QAction(tr("Play/Stop"), self)
        self.play_action.setObjectName("qt.playback.toggle")
        self.play_action.triggered.connect(lambda: self.focused_monitor().toggle_playback())
        self.tool_actions = []
        for tool, label in (("selection", N_("Selection Tool")), ("razor", N_("Razor Tool"))):
            a = QAction(tr(label), self)
            a.setObjectName(f"qt.tool.{tool}")
            a.triggered.connect(lambda _c=False, t=tool: self.timeline.set_tool(t))
            self.tool_actions.append(a)
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
            (self.tool_actions[0], ["V"]),
            (self.tool_actions[1], ["C"]),
        ):
            free = [QKeySequence(k) for k in keys if k not in bound_keys]
            action.setShortcuts(free)
            self.addAction(action)

    def _build_menus(self) -> None:
        view_actions: list[QAction] = [self.fit_action, self.reset_layout]
        self.registry.build_menus(
            self.menuBar(), self, {
                "File": [self.quit_action],
                "Sequence": [self.play_action, *self.tool_actions],
                "Help": [self.about_action],
            }
        )
        window_menu = self.menuBar().addMenu(tr("Panels"))
        window_menu.setObjectName("menu.Panels")
        for dock in self.docks.values():
            window_menu.addAction(dock.toggleViewAction())
        window_menu.addSeparator()
        for a in view_actions:
            window_menu.addAction(a)
        lang = window_menu.addMenu(tr("Language"))
        for a in self.lang_actions.actions():
            lang.addAction(a)
        self._bind_local_shortcuts(self.registry.claimed_keys)

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
        self.engine.execute("effects.list", {}, on_reply=lambda r, _b: self.effects.set_effects(r))
        self.refresh()
        recovery = hello.get("recovery")
        if isinstance(recovery, list) and recovery and isinstance(recovery[0], dict):
            QTimer.singleShot(0, lambda c=recovery[0]: self._offer_recovery(c))

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
        self.project = result if isinstance(result, dict) else None
        self.project_panel.set_project(self.project)
        self._update_title()

    def _on_sequence(self, result: Any, _b: bytes | None) -> None:
        self.sequence = result if isinstance(result, dict) else None
        if self.monitor.playing and self.sequence is not None:
            self.sequence["playhead"] = self.monitor.playhead
        self.timeline.set_sequence(self.sequence)
        self.monitor.set_sequence(self.sequence, self.revision)
        self.inspector.set_sequence(self.sequence)

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

    def run_command(self, cmd: Command, unsaved_ok: bool = False) -> None:
        """A menu, button or shortcut ran ``cmd``: ask for what it needs, then execute."""
        if self.monitor.playing and cmd.id.startswith(("playhead.", "edit.", "timeline.")):
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
        if (cmd.id in REPLACES_PROJECT or cmd.id == "file.closeProject") and self.export_dialog is not None:
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
        if new is self.source_monitor or self.source_monitor.isAncestorOf(new):
            self.active_monitor = "source"
        elif new is self.monitor or self.monitor.isAncestorOf(new) or self.timeline.isAncestorOf(new):
            self.active_monitor = "program"

    # ----- playhead -----
    def _scrub(self, ticks: int) -> None:
        self.monitor.set_playhead(ticks)

    def _playback_moved(self, ticks: int) -> None:
        self.timeline.canvas.playhead = ticks
        self.timeline.canvas.update()

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
    ) -> None:
        """Run ``proceed`` once unsaved changes are saved, or ``discard`` (default: ``proceed``)
        when the user doesn't save them; nothing on Cancel. The dirty flag comes from the engine,
        asked after every command already queued, not from the last (possibly stale) refresh;
        an engine that doesn't answer within ``STATE_TIMEOUT_MS`` falls back to the last refresh."""
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
            box.setStandardButtons(
                QMessageBox.StandardButton.Save | QMessageBox.StandardButton.Discard | QMessageBox.StandardButton.Cancel
            )
            answer = box.exec()
            if answer == QMessageBox.StandardButton.Discard:
                (discard or proceed)()
                return
            if answer != QMessageBox.StandardButton.Save:
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
