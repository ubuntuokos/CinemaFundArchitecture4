"""F5 isolated runtime helpers: lightweight tests with no actual media binary."""
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import f5_staged_runtime as f5
from verify_cfa4_packages import AuditError


class F5StagingTests(unittest.TestCase):
    def test_isolated_environment_does_not_use_real_home(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = f5.isolated_env(Path(tmp))
            self.assertTrue(env["HOME"].startswith(tmp))
            self.assertTrue(Path(env["XDG_DATA_HOME"]).is_dir())

    def test_cli_version_must_match_cfa4_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp) / "cfa4-cli"
            app.write_text("#!/bin/sh\necho 'cfa4-cli 0.10.0'\n")
            app.chmod(0o755)
            self.assertTrue(f5.cli_smoke(app, "0.10.0", os.environ)["version_match"])
            with self.assertRaisesRegex(AuditError, "lacks expected version"):
                f5.cli_smoke(app, "0.11.0", os.environ)

    def test_ldd_missing_dependency_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            exe = Path(tmp) / "cfa4"
            exe.write_bytes(b"fake-ELF")
            with patch.object(f5, "invoke", return_value="libbroken.so => not found\n"):
                with self.assertRaisesRegex(AuditError, "missing ELF runtime dependencies"):
                    f5.library_inventory(exe, env=os.environ)

    def test_flatpak_ref_rejects_wrong_architecture(self):
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(f5, "invoke", side_effect=[
                "",
                "",
                "app/io.github.ubuntuokos.cfa4/aarch64/stable\n",
            ]):
                with self.assertRaisesRegex(AuditError, "unexpected app refs"):
                    f5.stage_flatpak(Path(tmp) / "fixture.flatpak", Path(tmp), os.environ)

    def test_flatpak_ref_accepts_exact_cfa4_amd64_id(self):
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(f5, "invoke", side_effect=[
                "",
                "",
                "app/io.github.ubuntuokos.cfa4/x86_64/stable\n",
            ]):
                result = f5.stage_flatpak(Path(tmp) / "fixture.flatpak", Path(tmp), os.environ)
                self.assertIn("/x86_64/", result["ref"])
                self.assertIn("NOT_FLATPAK_INSTALL", result["scope"])

    def test_invalid_command_is_not_success(self):
        with self.assertRaisesRegex(AuditError, "unable to run"):
            f5.invoke(["/nonexistent-cfa4-f5-command"], timeout=1)


if __name__ == "__main__":
    unittest.main()
