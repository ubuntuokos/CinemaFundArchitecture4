"""External timecode sync in the Qt GUI (plan TC-6): the Program monitor's transport goes to the
engine (``timecodeSync.transport``), whose LTC and MIDI Timecode outputs follow it; the engine's
``timecodeSync.locate`` / ``timecodeSync.stop`` events move the monitor when it chases an external
clock; a status-bar label shows the chase state and device errors (``timecodeSync.status``)."""

from __future__ import annotations

from typing import Any, Protocol

from PySide6.QtCore import QObject, QTimer
from PySide6.QtWidgets import QLabel, QWidget

from .i18n import N_, tr

STATE_TEXT = {
    "noSignal": N_("No signal"),
    "locating": N_("Locating"),
    "locked": N_("Locked"),
    "freewheel": N_("Freewheel"),
    "scrubbing": N_("Scrubbing"),
}


class Monitor(Protocol):
    playing: bool
    speed: float
    playhead: int

    def seek(self, ticks: int) -> None: ...
    def play(self, speed: float = 1.0) -> None: ...
    def stop(self, notify: bool = True) -> None: ...


def transport_of(monitor: Monitor) -> dict[str, Any]:
    """The ``timecodeSync.transport`` parameters for the monitor's state."""
    return {"playing": bool(monitor.playing), "time": int(monitor.playhead), "speed": float(monitor.speed) if monitor.playing else 1.0}


def status_text(st: Any) -> tuple[str, str, bool]:
    """(label text, tooltip, error) for a ``timecodeSync.status`` result; empty text when sync is off."""
    if not isinstance(st, dict):
        return "", "", False
    parts: list[str] = []
    chase = st.get("chaseStatus")
    if st.get("chase") and isinstance(chase, dict):
        state = tr(STATE_TEXT.get(str(chase.get("state")), str(chase.get("state"))))
        tc = chase.get("timecode")
        parts.append(f"{tr('Chase')} {state}" + (f" {tc}" if isinstance(tc, str) and tc else ""))
    outs = [name for key, name in (("ltcOutRunning", "LTC"), ("mtcOutRunning", "MTC")) if st.get(key)]
    if outs:
        parts.append(f"{tr('Sending')} {' + '.join(outs)}")
    error = st.get("error")
    warning = st.get("warning")
    if not parts and not error:
        return "", "", False
    tip = "\n".join(str(x) for x in (error, warning) if isinstance(x, str) and x)
    text = "TC: " + (" · ".join(parts) if parts else tr("Error"))
    return text, tip, bool(error)


class TimecodeSyncLink(QObject):
    """Connects the Program monitor with the engine's timecode sync."""

    def __init__(self, engine: Any, monitor: Monitor, parent: QWidget) -> None:
        super().__init__(parent)
        self.engine = engine
        self.monitor = monitor
        self._sent: dict[str, Any] | None = None
        self.label = QLabel(parent)
        self.label.setObjectName("status.timecodeSync")
        self.label.setContentsMargins(8, 0, 8, 0)
        self.label.hide()
        for name in ("playback_started", "playback_stopped"):
            sig = getattr(monitor, name, None)
            if sig is not None:
                sig.connect(self.report)
        # seeks while stopped (a locate MTC/LTC output repeats) and missed signals
        self._poll = QTimer(self)
        self._poll.setInterval(200)
        self._poll.timeout.connect(self._check)
        self._poll.start()
        self._status = QTimer(self)
        self._status.setInterval(500)
        self._status.timeout.connect(self.refresh_status)
        self._status.start()

    def _running(self) -> bool:
        return bool(getattr(self.engine, "running", False))

    def report(self, force: bool = True) -> None:
        """Send the monitor's transport (``force``: even when unchanged, e.g. a restart of play)."""
        if not self._running():
            return
        t = transport_of(self.monitor)
        if not force and self._sent is not None:
            same_state = (t["playing"], t["speed"]) == (self._sent["playing"], self._sent["speed"])
            # while playing the engine runs the clock on: only a start, stop or speed change counts
            if same_state and (t["playing"] or t["time"] == self._sent["time"]):
                return
        self._sent = t
        self.engine.call("timecodeSync.transport", t, on_error=lambda _m: None)

    def _check(self) -> None:
        self.report(force=False)

    def on_event(self, ev: dict[str, Any]) -> bool:
        """Handles ``timecodeSync.*`` events; False for other events."""
        kind = ev.get("event")
        if kind == "timecodeSync.locate":
            t = ev.get("time")
            if not isinstance(t, int) or isinstance(t, bool):
                return True
            self.monitor.seek(t)
            if ev.get("play") is True:
                self.monitor.play(1.0)
            self.report()
            return True
        if kind == "timecodeSync.stop":
            self.monitor.stop()
            self.report()
            return True
        return False

    def refresh_status(self) -> None:
        if not self._running():
            return
        self.engine.execute("timecodeSync.status", {}, on_reply=lambda r, _b: self.show_status(r), on_error=lambda _m: None)

    def show_status(self, st: Any) -> None:
        text, tip, error = status_text(st)
        self.label.setVisible(bool(text))
        self.label.setText(text)
        self.label.setToolTip(tip)
        self.label.setStyleSheet("color: #e5534b;" if error else "")
