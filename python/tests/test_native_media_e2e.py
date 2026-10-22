"""Pure unit tests for #16's native bridge audit harness (no GUI/GPU required)."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path
from unittest.mock import patch

import pytest

from cfa4_devtools import native_media_e2e as audit


def test_bridge_builds_loopback_command_with_typed_json() -> None:
    params = {"items": [23], "range": "entire", "wait": True}
    completed = subprocess.CompletedProcess(args=[], returncode=0, stdout='{"sequence":101}\n', stderr="")
    with patch.object(audit, "run", return_value=completed) as mocked:
        result = audit.bridge(Path("/tmp/filmcraft-cli"), 19274, "file.newSequenceFromClip", params)
    assert result["sequence"] == 101
    args = mocked.call_args.args[0]
    assert args[:5] == ["/tmp/filmcraft-cli", "--bridge", "127.0.0.1:19274", "exec", "file.newSequenceFromClip"]
    assert json.loads(args[5]) == params


def test_command_failed_must_not_be_accepted_as_a_native_gui_pass() -> None:
    result = subprocess.CompletedProcess(args=[], returncode=1, stdout="", stderr="export: unsupported HEVC")
    with pytest.raises(RuntimeError, match="unsupported HEVC"):
        audit.parse_command_json(result, "native export")


def test_native_perf_hardware_counters_distinguish_decode_from_export() -> None:
    data = {
        "decode": {"hardware": {"frames": 18}},
        "export": {"hardware": {"frames": 8, "sessions": 1}},
    }
    assert audit.hardware_stats(data) == (18, 8, 1)


def test_malformed_json_must_be_a_failure() -> None:
    completed = subprocess.CompletedProcess(args=[], returncode=0, stdout="not JSON", stderr="")
    with pytest.raises(RuntimeError, match="non-JSON"):
        audit.parse_command_json(completed, "import")


def test_bridge_nonobject_is_a_failure() -> None:
    completed = subprocess.CompletedProcess(args=[], returncode=0, stdout="[]", stderr="")
    with pytest.raises(RuntimeError, match="expected JSON object"):
        audit.parse_command_json(completed, "import")
