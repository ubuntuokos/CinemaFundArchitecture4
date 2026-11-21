"""F8 local bridge contract; no Qt process, Rust compilation or network."""
import json
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

from cfa4_devtools.qt_app import locate_engine
from cfa4_devtools.qt_control_bridge import LocalBridgeError, bridge_read


class LocalQtBridgeContract(unittest.TestCase):
    def setUp(self):
        from tempfile import TemporaryDirectory
        self.directory = TemporaryDirectory(prefix="cfa4-f8-")
        self.addCleanup(self.directory.cleanup)
        self.cli = Path(self.directory.name) / "cfa4-cli"
        self.cli.write_text("#!/bin/sh\nexit 0\n")
        self.cli.chmod(0o755)

    def test_only_allowlisted_read_only_commands(self):
        with self.assertRaisesRegex(LocalBridgeError, "unapproved"):
            bridge_read(self.cli, 8132, "file.exportMedia", {})
        with self.assertRaisesRegex(LocalBridgeError, "unapproved"):
            bridge_read(self.cli, 8132, "file.import", {})

    def test_subprocess_uses_loopback_and_typed_json(self):
        answer = subprocess.CompletedProcess([], 0, json.dumps({"decode": {}}).encode(), b"")
        with patch("cfa4_devtools.qt_control_bridge.subprocess.run", return_value=answer) as p:
            result = bridge_read(self.cli, 8132, "perf.stats", {"flag": True})
        self.assertEqual(result, {"decode": {}})
        args, kwargs = p.call_args
        self.assertEqual(args[0][:5], [str(self.cli), "--bridge", "127.0.0.1:8132",
                                       "exec", "perf.stats"])
        self.assertEqual(json.loads(args[0][5]), {"flag": True})
        self.assertNotIn("shell", kwargs)
        self.assertLessEqual(kwargs["timeout"], 30)

    def test_invalid_ports_and_timeout_fail(self):
        for port in (0, 65536, True, "8132"):
            with self.subTest(port=port), self.assertRaises(LocalBridgeError):
                bridge_read(self.cli, port, "perf.stats")
        with self.assertRaises(LocalBridgeError):
            bridge_read(self.cli, 8132, "perf.stats", timeout=999)

    def test_invalid_json_params_fail_before_subprocess(self):
        with self.assertRaisesRegex(LocalBridgeError, "JSON"):
            bridge_read(self.cli, 8132, "perf.stats", {"not-json": object()})

    def test_failure_is_not_a_successful_response(self):
        with (
            patch(
                "cfa4_devtools.qt_control_bridge.subprocess.run",
                return_value=subprocess.CompletedProcess([], 7, b"{}", b"bad"),
            ),
            self.assertRaisesRegex(LocalBridgeError, "exit 7"),
        ):
            bridge_read(self.cli, 8132, "perf.stats")

    def test_invalid_return_shape_and_reported_error_fail(self):
        for body in (b"[]", b"{", b'{"error":"denied"}'):
            with (
                self.subTest(body=body),
                patch(
                    "cfa4_devtools.qt_control_bridge.subprocess.run",
                    return_value=subprocess.CompletedProcess([], 0, body, b""),
                ),
                self.assertRaises(LocalBridgeError),
            ):
                bridge_read(self.cli, 8132, "sequence.inspect")

    def test_timeout_is_reported_not_suppressed(self):
        with (
            patch(
                "cfa4_devtools.qt_control_bridge.subprocess.run",
                side_effect=subprocess.TimeoutExpired("cfa4-cli", 3),
            ),
            self.assertRaisesRegex(LocalBridgeError, "timed out"),
        ):
            bridge_read(self.cli, 8132, "perf.stats", timeout=3)

    def test_local_engine_locator_prefers_cfa4_executable(self):
        def where(name):
            return "/usr/bin/cfa4-cli" if name == "cfa4-cli" else None
        with patch("cfa4_devtools.qt_app.shutil.which", side_effect=where):
            self.assertEqual(locate_engine(), Path("/usr/bin/cfa4-cli"))


if __name__ == "__main__":
    unittest.main()
