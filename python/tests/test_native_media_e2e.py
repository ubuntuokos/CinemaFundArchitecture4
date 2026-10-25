"""Pure unittest suite for #16's native bridge (no GUI, downloads or GPU)."""

from __future__ import annotations

import json
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

from cfa4_devtools import native_media_e2e as audit


class NativeMediaE2ESourceTests(unittest.TestCase):
    def test_bridge_command_uses_localhost_and_typed_json(self) -> None:
        params = {"items": [23], "range": "entire", "wait": True}
        completed = subprocess.CompletedProcess(args=[], returncode=0, stdout='{"sequence":101}\n', stderr="")
        with patch.object(audit, "run", return_value=completed) as mocked:
            result = audit.bridge(Path("/tmp/cfa4-cli"), 19274, "file.newSequenceFromClip", params)
        self.assertEqual(result["sequence"], 101)
        args = mocked.call_args.args[0]
        self.assertEqual(
            args[:5], ["/tmp/cfa4-cli", "--bridge", "127.0.0.1:19274", "exec", "file.newSequenceFromClip"]
        )
        self.assertEqual(json.loads(args[5]), params)

    def test_hardware_export_failure_does_not_pass(self) -> None:
        result = subprocess.CompletedProcess(args=[], returncode=1, stdout="", stderr="export: unsupported HEVC")
        with self.assertRaisesRegex(RuntimeError, "unsupported HEVC"):
            audit.parse_command_json(result, "native export")

    def test_hardware_counters_are_separate(self) -> None:
        data = {
            "decode": {"hardware": {"frames": 18}},
            "export": {"hardware": {"frames": 8, "sessions": 1}},
        }
        self.assertEqual(audit.hardware_stats(data), (18, 8, 1))

    def test_non_json_output_is_rejected(self) -> None:
        completed = subprocess.CompletedProcess(args=[], returncode=0, stdout="not JSON", stderr="")
        with self.assertRaisesRegex(RuntimeError, "non-JSON"):
            audit.parse_command_json(completed, "import")

    def test_non_object_json_is_rejected(self) -> None:
        completed = subprocess.CompletedProcess(args=[], returncode=0, stdout="[]", stderr="")
        with self.assertRaisesRegex(TypeError, "expected JSON object"):
            audit.parse_command_json(completed, "import")


if __name__ == "__main__":
    unittest.main()
