"""Python-side crash guard, as egui's ``crash.rs``: an exception that escapes a slot, an event
handler or a background thread is written to ``<data dir>/Logs/crash-<day>.log`` with its
thread and traceback, kept as the last error and reported to the window (``reporter().error``),
which shows it and keeps the session running. Qt already keeps the event loop alive after such an
exception; without this the user only sees a widget stop working and no trace is left.
"""

from __future__ import annotations

import os
import sys
import threading
import time
import traceback
from pathlib import Path
from types import TracebackType
from typing import Any

from PySide6.QtCore import QObject, Signal

from . import __version__

# a broken paintEvent raises on every repaint: the log keeps the first ones, not a disk's worth
MAX_LOGGED = 50

_last: str | None = None
_log_dir: Path | None = None
_logged = 0
_lock = threading.Lock()
_installed: tuple[Any, Any] | None = None  # the previous sys / threading hooks
_reporter: Reporter | None = None
_reporting = threading.local()  # an error while reporting one is logged, not reported again


class Reporter(QObject):
    """``error(summary)``: queued to the GUI thread whichever thread raised."""

    error = Signal(str)


def reporter() -> Reporter:
    global _reporter
    if _reporter is None:
        _reporter = Reporter()
    return _reporter


def default_data_dir() -> Path | None:
    """The engine's data directory (``autosave::default_data_dir`` on Linux): ``CFA4_DATA_DIR``,
    else ``$XDG_DATA_HOME/cfa4``, else ``~/.local/share/cfa4``."""
    for key, sub in (("CFA4_DATA_DIR", ""), ("XDG_DATA_HOME", "cfa4")):
        value = os.environ.get(key)
        if value:
            return Path(value) / sub if sub else Path(value)
    home = os.environ.get("HOME")
    return Path(home) / ".local" / "share" / "cfa4" if home else None


def log_path() -> Path | None:
    """Today's crash log, if logging to disk."""
    if _log_dir is None:
        return None
    return _log_dir / f"crash-{int(time.time()) // 86_400}.log"


def take_last() -> str | None:
    """The last error's summary, once."""
    global _last
    with _lock:
        last, _last = _last, None
    return last


def report(exc_type: type[BaseException], exc: BaseException, tb: TracebackType | None, thread: str | None = None) -> str:
    """Record an exception that nothing caught; returns its summary."""
    global _last, _logged
    frames = traceback.extract_tb(tb)
    at = f"{frames[-1].filename}:{frames[-1].lineno}" if frames else ""
    thread = thread or threading.current_thread().name
    summary = f"{exc_type.__name__}: {exc} (at {at}, thread {thread})"
    with _lock:
        _last = summary
        write = _logged < MAX_LOGGED
        _logged += 1
    path = log_path()
    if write and path is not None:
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("a", encoding="utf-8") as f:
                f.write(f"=== cfa4-qt CFA4 {__version__} (unix {int(time.time())})\n{summary}\n")
                f.write("".join(traceback.format_exception(exc_type, exc, tb)))
                f.write("\n")
        except OSError:
            pass  # a full or read-only disk must not turn one error into two
    if not getattr(_reporting, "active", False):
        _reporting.active = True
        try:
            reporter().error.emit(summary)
        except RuntimeError:
            pass  # the window is already gone (shutting down)
        finally:
            _reporting.active = False
    return summary


def install(log_dir: Path | None) -> None:
    """Install the hooks (again: only the log directory changes). Exceptions still reach the
    previous hooks, so they are printed to stderr as before."""
    global _log_dir, _installed
    _log_dir = Path(log_dir) if log_dir is not None else None
    if _installed is not None:
        return
    previous, previous_thread = sys.excepthook, threading.excepthook
    _installed = (previous, previous_thread)

    def hook(exc_type: type[BaseException], exc: BaseException, tb: TracebackType | None) -> None:
        if issubclass(exc_type, KeyboardInterrupt):
            previous(exc_type, exc, tb)
            return
        report(exc_type, exc, tb)
        previous(exc_type, exc, tb)

    def thread_hook(args: threading.ExceptHookArgs) -> None:
        if args.exc_value is not None and not issubclass(args.exc_type, SystemExit):
            report(args.exc_type, args.exc_value, args.exc_traceback, args.thread.name if args.thread else None)
        previous_thread(args)

    sys.excepthook = hook
    threading.excepthook = thread_hook


def uninstall() -> None:
    """Put the previous hooks back (tests)."""
    global _installed, _log_dir, _logged
    if _installed is not None:
        sys.excepthook, threading.excepthook = _installed
    _installed, _log_dir, _logged = None, None, 0
