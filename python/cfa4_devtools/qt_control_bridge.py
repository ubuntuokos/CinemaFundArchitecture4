"""CFA4 F8: bounded LOCAL Rust control bridge for a future Qt6 controller.

Never call this synchronous adapter from the Qt GUI event thread. A future Qt
controller must invoke it in QThread/QThreadPool and deliver results by signals.
No network services, subprocess shell, cloud calls or package installation.
"""
from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path
from typing import Any

# Start read-only; mutating project operations require a separately reviewed
# Qt lifecycle transaction and confirmed error/timeout/undo semantics.
READ_ONLY_OPERATIONS = frozenset({"perf.stats", "sequence.inspect"})
MAX_RESPONSE_BYTES = 4 * 1024 * 1024


class LocalBridgeError(RuntimeError):
    """Rust editor bridge unavailable, untrusted or returned an error."""


def bridge_read(
    cli: Path, port: int, operation: str, params: dict[str, Any] | None = None,
    *, timeout: int = 10,
) -> dict[str, Any]:
    """Query a running Rust editor on 127.0.0.1 only."""
    if operation not in READ_ONLY_OPERATIONS:
        raise LocalBridgeError(f"unapproved read-only operation: {operation}")
    if not isinstance(port, int) or isinstance(port, bool) or not 1 <= port <= 65535:
        raise LocalBridgeError("loopback port must be 1..65535")
    if not 1 <= timeout <= 30:
        raise LocalBridgeError("timeout must be within 1..30 seconds")
    if params is None:
        params = {}
    if not isinstance(params, dict):
        raise LocalBridgeError("bridge parameters must be a JSON object")
    cli = Path(cli)
    if not cli.is_file() or not os.access(cli, os.X_OK):
        raise LocalBridgeError(f"local CFA4 CLI is missing or non-executable: {cli}")
    try:
        payload = json.dumps(params, separators=(",", ":"), allow_nan=False)
    except (TypeError, ValueError) as exc:
        raise LocalBridgeError("bridge parameters are not JSON-serializable") from exc
    argv = [str(cli), "--bridge", f"127.0.0.1:{port}", "exec", operation, payload]
    try:
        completed = subprocess.run(
            argv, capture_output=True, check=False, timeout=timeout,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise LocalBridgeError(f"local bridge command unavailable or timed out: {exc}") from exc
    if completed.returncode:
        raise LocalBridgeError(
            f"local bridge command failed: exit {completed.returncode}"
        )
    if not completed.stdout or len(completed.stdout) > MAX_RESPONSE_BYTES:
        raise LocalBridgeError("local bridge response empty or too large")
    try:
        result = json.loads(completed.stdout)
    except (ValueError, UnicodeDecodeError) as exc:
        raise LocalBridgeError("local bridge returned invalid JSON") from exc
    if not isinstance(result, dict):
        raise LocalBridgeError("local bridge must return a JSON object")
    if result.get("error"):
        raise LocalBridgeError("local bridge returned a command error")
    return result
