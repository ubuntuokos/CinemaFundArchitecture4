"""CFA4 Qt 6 main window: docks, menus from the engine's command registry, and the glue
between panels and the engine. The window holds views of engine state only; every edit is an
engine command and every redraw follows the engine's ``projectChanged``."""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QByteArray, QSettings, Qt, QTimer
from PySide6.QtGui import QAction, QActionGroup, QCloseEvent, QKeySequence
from PySide6.QtWidgets import QDockWidget, QLabel, QMainWindow, QMessageBox, QWidget

from . import __version__
from .branding import logo_pixmap
from .commands import Command, CommandRegistry
from .dialogs import ParamDialog, path_params
from .engine import EngineClient
from .i18n import LANGUAGES, TR, tr
from .panels.monitor import ProgramMonitor
from .panels.project import ProjectPanel
from .panels.side import EffectsPanel, EventsPanel, HistoryPanel, InspectorPanel
from .panels.timeline import TimelinePanel

LAYOUT_VERSION = 1


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
        self._closing = False
        self._quit_confirmed = False

        self.monitor = ProgramMonitor(engine, self)
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
        self._dock("Project", self.project_panel, Qt.DockWidgetArea.LeftDockWidgetArea)
        self._dock("Effects", self.effects, Qt.DockWidgetArea.LeftDockWidgetArea)
        self.tabifyDockWidget(self.docks["Project"], self.docks["Effects"])
        self.docks["Project"].raise_()
        self._dock("Inspector", self.inspector, Qt.DockWidgetArea.RightDockWidgetArea)
        self._dock("History", self.history, Qt.DockWidgetArea.RightDockWidgetArea)
        self.tabifyDockWidget(self.docks["Inspector"], self.docks["History"])
        self.docks["Inspector"].raise_()
        self._dock("Timeline", self.timeline, Qt.DockWidgetArea.BottomDockWidgetArea)
        self._dock("Events", self.events, Qt.DockWidgetArea.BottomDockWidgetArea)
        self.tabifyDockWidget(self.docks["Timeline"], self.docks["Events"])
        self.docks["Timeline"].raise_()
        self.resizeDocks([self.docks["Timeline"]], [300], Qt.Orientation.Vertical)
        self.resizeDocks([self.docks["Project"], self.docks["Inspector"]], [300, 280], Qt.Orientation.Horizontal)
        self._default_state = self.saveState(LAYOUT_VERSION)

        self.status_engine = QLabel(tr("Starting engine…"))
        self.status_engine.setObjectName("status.engine")
        self.statusBar().addPermanentWidget(self.status_engine)

        for panel in (self.monitor, self.timeline.canvas, self.project_panel, self.effects, self.inspector):
            panel.command.connect(self.execute)
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
        if isinstance(state, QByteArray):
            self.restoreState(state, LAYOUT_VERSION)
        self._update_title()

    def _dock(self, name: str, widget: QWidget, area: Qt.DockWidgetArea) -> None:
        dock = QDockWidget(tr(name), self)
        dock.setObjectName(f"dock.{name}")
        dock.setWidget(widget)
        self.addDockWidget(area, dock)
        self.docks[name] = dock

    # ----- actions owned by the Qt GUI (view, playback, help) -----
    def _build_local_actions(self) -> None:
        self.play_action = QAction(tr("Play/Stop"), self)
        self.play_action.setObjectName("qt.playback.toggle")
        self.play_action.triggered.connect(self.monitor.toggle_playback)
        self.tool_actions = []
        for tool, label in (("selection", "Selection Tool"), ("razor", "Razor Tool")):
            a = QAction(tr(label), self)
            a.setObjectName(f"qt.tool.{tool}")
            a.triggered.connect(lambda _c=False, t=tool: self.timeline.set_tool(t))
            self.tool_actions.append(a)
        self.fit_action = QAction(tr("Zoom to Sequence"), self)
        self.fit_action.triggered.connect(self.timeline.canvas.zoom_to_fit)
        self.reset_layout = QAction(tr("Reset Panel Layout"), self)
        self.reset_layout.triggered.connect(lambda: self.restoreState(self._default_state, LAYOUT_VERSION))
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
            (self.quit_action, ["Ctrl+Q"]),
            (self.tool_actions[0], ["V"]),
            (self.tool_actions[1], ["C"]),
        ):
            free = [QKeySequence(k) for k in keys if k not in bound_keys]
            action.setShortcuts(free)
            self.addAction(action)

    def _build_menus(self) -> None:
        view_actions: list[QAction] = [self.fit_action, self.reset_layout]
        bound = self.registry.build_menus(
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
        keys = {s.toString() for cid in bound for s in self.registry.actions[cid].shortcuts()}
        self._bind_local_shortcuts(keys)

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
        import datetime

        when = (
            datetime.datetime.fromtimestamp(int(cand.get("saved_unix") or 0), tz=datetime.UTC)
            .astimezone()
            .strftime("%Y-%m-%d %H:%M:%S")
        )
        name = str(cand.get("project_name") or tr("Untitled"))
        text = tr("CFA4 quit unexpectedly while '{name}' had unsaved changes. Recover unsaved changes from {time}?")
        if cand.get("clean_exit"):
            text = tr("CFA4 was closed while '{name}' had unsaved changes. Recover unsaved changes from {time}?")
        box = QMessageBox(
            QMessageBox.Icon.Question,
            tr("Recover Unsaved Changes"),
            text.replace("{name}", name).replace("{time}", when),
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
        self.monitor.stop()
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

    def _update_title(self) -> None:
        name = (self.project or {}).get("name") or tr("Untitled")
        dirty = "*" if self.state.get("dirty") else ""
        self.setWindowTitle(f"{name}{dirty} — CFA4")

    # ----- commands -----
    def execute(self, command: str, params: dict[str, Any] | None = None) -> None:
        """Run an engine command; errors go to the status bar and the Events panel."""
        if command == "playhead.set" and self.monitor.playing:
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

    def run_command(self, cmd: Command) -> None:
        """A menu, button or shortcut ran ``cmd``: ask for what it needs, then execute."""
        if self.monitor.playing and cmd.id.startswith(("playhead.", "edit.", "timeline.")):
            self.monitor.stop()
        if cmd.id == "app.quit":
            self.close()
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
        if cmd.needs_params:
            self._ask_params(cmd)
            return

        def failed(message: str) -> None:
            # commands whose dialog lives in the frontend: ask for the parameters instead
            if cmd.param_specs:
                self._ask_params(cmd, message)
            else:
                self._command_failed(cmd.id, message)

        self.engine.execute(cmd.id, {}, on_reply=lambda _r, _b: self.schedule_refresh(), on_error=failed)

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

    # ----- playhead -----
    def _scrub(self, ticks: int) -> None:
        self.monitor.set_playhead(ticks)

    def _playback_moved(self, ticks: int) -> None:
        self.timeline.canvas.playhead = ticks
        self.timeline.canvas.update()

    # ----- misc -----
    def _about(self) -> None:
        box = QMessageBox(self)
        box.setObjectName("dialog.about")
        box.setWindowTitle(tr("About CFA4"))
        box.setIconPixmap(logo_pixmap(96))
        box.setText(
            f"<b>CFA4</b> — CinemaFundArchitecture4<br>Qt 6 GUI {__version__}<br>{self.engine.hello.get('engine', '')}<br><br>"
            "GPL-3.0-or-later<br>Rust engine · Qt 6 / PySide6 · Ubuntu"
        )
        box.open()

    def closeEvent(self, e: QCloseEvent) -> None:
        if self.engine.running and self.state.get("dirty") and not self._quit_confirmed:
            name = (self.project or {}).get("name") or tr("Untitled")
            box = QMessageBox(
                QMessageBox.Icon.Question,
                tr("CFA4"),
                tr("Save changes to '{name}' before closing?").replace("{name}", str(name)),
                parent=self,
            )
            box.setStandardButtons(
                QMessageBox.StandardButton.Save | QMessageBox.StandardButton.Discard | QMessageBox.StandardButton.Cancel
            )
            answer = box.exec()
            if answer == QMessageBox.StandardButton.Cancel:
                e.ignore()
                return
            if answer == QMessageBox.StandardButton.Save:
                e.ignore()
                cmd = self.registry.commands.get("file.save") or Command("file.save", "Save", [], "", "{}", True)
                params = path_params(cmd, self, (self.project or {}).get("path"))
                if params and params.get("__cancel__"):
                    return

                def saved(_r: Any, _b: bytes | None) -> None:
                    self._quit_confirmed = True
                    self.close()

                self.engine.execute(
                    "file.save", params or {}, on_reply=saved, on_error=lambda m: self._command_failed("file.save", m)
                )
                return
            # Don't Save: close the project, which also clears its recovery snapshot (as egui does)
            self.engine.execute("file.closeProject", {"force": True})
        self._closing = True
        self.monitor.stop()
        self.settings.setValue("geometry", self.saveGeometry())
        self.settings.setValue("windowState", self.saveState(LAYOUT_VERSION))
        self.engine.stop()
        e.accept()
