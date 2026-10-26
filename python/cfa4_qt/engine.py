"""Client for the headless Rust engine (`cfa4-cli serve`).

The Qt GUI owns no project data: the Rust engine does. This module starts the engine as a child
process and talks to it over stdin/stdout (protocol: ``docs/qt-gui.md``). Requests are written
from the GUI thread (small JSON lines); a reader thread parses replies (with optional binary
tails) and engine events, and hands them to the GUI thread through Qt signals. Nothing here
blocks the Qt event loop, opens a socket or touches the network.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import threading
from collections.abc import Callable
from pathlib import Path
from typing import Any

from PySide6.QtCore import QObject, Signal, Slot

PROTOCOL = 1
MAX_LINE = 4 * 1024 * 1024
MAX_BINARY = 512 * 1024 * 1024

Reply = Callable[[Any, bytes | None], None]
Failure = Callable[[str], None]


class EngineError(RuntimeError):
    """The engine is unavailable or refused a request."""


def engine_serves(executable: Path) -> bool:
    """Does this CLI have the `serve` subcommand the GUI needs? (`help` starts no engine.)"""
    try:
        out = subprocess.run(
            [str(executable), "help"], stdin=subprocess.DEVNULL, capture_output=True, timeout=15, check=False
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    return b"\n  serve" in out.stdout


def engine_candidates(explicit: str | os.PathLike[str] | None = None) -> list[Path]:
    """Where the engine may be, best first; nothing is installed or downloaded.

    An explicit path or ``$CFA4_ENGINE`` is the only candidate. Otherwise the build of the
    checkout this GUI runs from comes first (newest first: release and debug), so an older CLI
    installed on ``PATH`` cannot shadow the engine built next to this GUI; then ``cfa4-cli`` on
    ``PATH``.
    """
    chosen = explicit or os.environ.get("CFA4_ENGINE")
    if chosen:
        return [Path(chosen).expanduser()]
    root = Path(__file__).resolve().parents[2]
    builds = [root / r for r in ("target/release/cfa4-cli", "target/debug/cfa4-cli")]
    builds = [b for b in builds if b.is_file()]
    builds.sort(key=lambda b: b.stat().st_mtime, reverse=True)
    on_path = [Path(f) for f in (shutil.which(n) for n in ("cfa4-cli",)) if f]
    out: list[Path] = []
    for c in [*builds, *on_path]:
        if c not in out:
            out.append(c)
    return out


def locate_engine(explicit: str | os.PathLike[str] | None = None, rejected: list[str] | None = None) -> Path | None:
    """The first candidate that is an executable CFA4 CLI with `serve`. Each skipped candidate
    is added to ``rejected`` with the reason, for the message the user sees."""
    for candidate in engine_candidates(explicit):
        if not candidate.is_file() or not os.access(candidate, os.X_OK):
            if rejected is not None:
                rejected.append(f"{candidate}: not found or not executable")
            continue
        if not engine_serves(candidate):
            if rejected is not None:
                rejected.append(f"{candidate}: too old (no `serve` subcommand)")
            continue
        return candidate
    return None


class FrameReader:
    """Reads protocol messages (JSON line + optional binary tail) from a byte stream."""

    def __init__(self, stream: Any) -> None:
        self._stream = stream

    def read(self) -> tuple[dict[str, Any], bytes | None] | None:
        """Next message, or ``None`` at end of stream. Raises ``EngineError`` on bad framing."""
        line = self._stream.readline(MAX_LINE + 1)
        if not line:
            return None
        if len(line) > MAX_LINE:
            raise EngineError("engine message line too long")
        try:
            message = json.loads(line)
        except (ValueError, UnicodeDecodeError) as exc:
            raise EngineError(f"engine sent invalid JSON: {exc}") from exc
        if not isinstance(message, dict):
            raise EngineError("engine message is not a JSON object")
        size = message.get("binary")
        if size is None:
            return message, None
        if not isinstance(size, int) or isinstance(size, bool) or not 0 <= size <= MAX_BINARY:
            raise EngineError("engine announced an invalid binary size")
        chunks: list[bytes] = []
        remaining = size
        while remaining:
            chunk = self._stream.read(remaining)
            if not chunk:
                raise EngineError("engine stream ended inside a binary reply")
            chunks.append(chunk)
            remaining -= len(chunk)
        return message, b"".join(chunks)


class EngineClient(QObject):
    """Asynchronous handle on one engine process.

    ``call`` sends a request and returns at once; ``on_reply(result, binary)`` or
    ``on_error(message)`` runs later on the GUI thread.
    """

    started = Signal(dict)  # the `hello` reply
    event = Signal(dict)  # engine notifications (projectChanged, toast, openSequence, ...)
    stopped = Signal(str)  # the process ended; reason
    failure = Signal(str)  # any request error, for the Events panel
    _message = Signal(dict, object)  # reader thread -> GUI thread

    def __init__(self, engine: Path, args: list[str] | None = None, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self.engine = Path(engine)
        self.args = list(args or [])
        self.hello: dict[str, Any] = {}
        self._process: subprocess.Popen[bytes] | None = None
        self._next_id = 1
        self._pending: dict[int, tuple[Reply | None, Failure | None]] = {}
        self._lock = threading.Lock()
        self._message.connect(self._dispatch)

    # ----- lifecycle -----
    def start(self) -> None:
        if self._process is not None:
            return
        argv = [str(self.engine), "serve", *self.args]
        try:
            self._process = subprocess.Popen(
                argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE
            )
        except OSError as exc:
            raise EngineError(f"cannot start the engine {self.engine}: {exc}") from exc
        threading.Thread(target=self._read_loop, name="cfa4-engine-reader", daemon=True).start()
        threading.Thread(target=self._stderr_loop, name="cfa4-engine-stderr", daemon=True).start()
        self.call("hello", on_reply=self._on_hello)

    @property
    def running(self) -> bool:
        return self._process is not None and self._process.poll() is None

    def stop(self, timeout: float = 5.0) -> None:
        process = self._process
        if process is None:
            return
        if process.poll() is None:
            try:
                self._write({"id": 0, "method": "app.quit", "params": {}})
                process.wait(timeout=timeout)
            except (OSError, EngineError, subprocess.TimeoutExpired):
                process.kill()
                process.wait(timeout=timeout)
        self._process = None

    # ----- requests -----
    def call(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        on_reply: Reply | None = None,
        on_error: Failure | None = None,
    ) -> int:
        """Send ``method`` with ``params``. Returns the request id."""
        with self._lock:
            request_id = self._next_id
            self._next_id += 1
            self._pending[request_id] = (on_reply, on_error)
        try:
            self._write({"id": request_id, "method": method, "params": params or {}})
        except EngineError as exc:
            with self._lock:
                self._pending.pop(request_id, None)
            self._fail(on_error, str(exc))
        return request_id

    def execute(
        self,
        command: str,
        params: dict[str, Any] | None = None,
        on_reply: Reply | None = None,
        on_error: Failure | None = None,
    ) -> int:
        """Run an engine command by id (everything is a command)."""
        return self.call("engine.execute", {"command": command, "params": params or {}}, on_reply, on_error)

    def _write(self, request: dict[str, Any]) -> None:
        process = self._process
        if process is None or process.stdin is None or process.poll() is not None:
            raise EngineError("the engine is not running")
        try:
            line = json.dumps(request, separators=(",", ":"), allow_nan=False).encode() + b"\n"
        except (TypeError, ValueError) as exc:
            raise EngineError(f"request is not JSON-serializable: {exc}") from exc
        if len(line) > MAX_LINE:
            raise EngineError("request too large")
        try:
            process.stdin.write(line)
            process.stdin.flush()
        except (BrokenPipeError, OSError) as exc:
            raise EngineError(f"the engine stopped accepting requests: {exc}") from exc

    # ----- reader side -----
    def _post(self, message: dict[str, Any], binary: bytes | None) -> bool:
        """Hand a message from a reader thread to the GUI thread. False when this client is gone
        (the window closed first at exit, while the engine was still ending): the thread then just
        stops, instead of raising "Signal source has been deleted" in the background."""
        try:
            self._message.emit(message, binary)
        except RuntimeError:
            return False
        return True

    def _read_loop(self) -> None:
        process = self._process
        if process is None or process.stdout is None:
            return
        reader = FrameReader(process.stdout)
        reason = "the engine exited"
        while True:
            try:
                message = reader.read()
            except EngineError as exc:
                reason = str(exc)
                break
            except (OSError, ValueError) as exc:
                reason = f"engine stream error: {exc}"
                break
            if message is None:
                break
            if not self._post(message[0], message[1]):
                return
        # stdout can reach EOF a moment before the process is reaped: wait briefly so the exit
        # status is not lost from the reason the user sees
        try:
            code = process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            code = process.poll()
        if code is not None and code != 0:
            reason = f"{reason} (exit status {code})"
        self._post({"__stopped__": reason}, None)

    def _stderr_loop(self) -> None:
        process = self._process
        if process is None or process.stderr is None:
            return
        for raw in process.stderr:
            text = raw.decode(errors="replace").rstrip()
            if text and not self._post({"__stderr__": text}, None):
                return

    @Slot(dict, object)
    def _dispatch(self, message: dict[str, Any], binary: bytes | None) -> None:
        if "__stopped__" in message:
            failed = list(self._pending.values())
            self._pending.clear()
            for _, on_error in failed:
                self._fail(on_error, "the engine stopped")
            self.stopped.emit(str(message["__stopped__"]))
            return
        if "__stderr__" in message:
            self.failure.emit(str(message["__stderr__"]))
            return
        if "event" in message and "id" not in message:
            self.event.emit(message)
            return
        request_id = message.get("id")
        with self._lock:
            handlers = self._pending.pop(request_id, None) if isinstance(request_id, int) else None
        if handlers is None:
            if not message.get("ok", True):
                self.failure.emit(str(message.get("error", "engine error")))
            return
        on_reply, on_error = handlers
        if message.get("ok"):
            if on_reply is not None:
                on_reply(message.get("result"), binary)
        else:
            self._fail(on_error, str(message.get("error", "engine error")))

    def _fail(self, on_error: Failure | None, text: str) -> None:
        if on_error is not None:
            on_error(text)
        else:
            self.failure.emit(text)

    def _on_hello(self, result: Any, _binary: bytes | None) -> None:
        hello = result if isinstance(result, dict) else {}
        if hello.get("protocol") != PROTOCOL:
            self.failure.emit(f"engine protocol {hello.get('protocol')!r}, this GUI speaks {PROTOCOL}")
        self.hello = hello
        self.started.emit(hello)
