from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SKILL_PATH = ROOT / ".claude/skills/cfa4-skill-review/SKILL.md"
INSTALLER_PATH = ROOT / "scripts/dev/install-skillspector.sh"
RUNNER_PATH = ROOT / "scripts/dev/skillspector.sh"


class SkillSpectorIntegrationTests(unittest.TestCase):
    def test_claude_skill_is_explicit_and_static_by_default(self) -> None:
        text = SKILL_PATH.read_text(encoding="utf-8")
        frontmatter = text.split("---", 2)[1]
        body = text.split("---", 2)[2]

        self.assertIn("name: cfa4-skill-review", frontmatter)
        self.assertIn("Copyright 2026 NVIDIA CORPORATION & AFFILIATES", text)
        self.assertIn("Adapted and modified for CFA4", text)
        register = (ROOT / "docs/licensing.md").read_text(encoding="utf-8")
        self.assertIn("NVIDIA CORPORATION & AFFILIATES (Copyright 2026)", register)
        self.assertIn("disable-model-invocation: true", frontmatter)
        self.assertIn("argument-hint:", frontmatter)
        self.assertNotIn("allowed-tools:", frontmatter)
        self.assertIn("--no-llm", body)
        self.assertIn("helyesen idézőjelezett shell-argumentumként", body)
        self.assertIn("APPROVE", body)
        self.assertIn("CAUTION", body)
        self.assertIn("REJECT", body)
        self.assertIn("ne futtasd", body.lower())

    def test_installer_pins_version_and_uses_a_venv(self) -> None:
        text = INSTALLER_PATH.read_text(encoding="utf-8")

        self.assertIn("version='2.13.2'", text)
        self.assertIn("commit='abd82935b67f765a9541013c0866c6349251dcdf'", text)
        self.assertIn("git+https://github.com/NVIDIA/SkillSpector.git@$commit", text)
        self.assertNotIn("skillspector==", text)
        self.assertIn("-m venv", text)
        self.assertNotIn("sudo ", text)
        self.assertNotIn("uv tool install", text)

    def test_runner_uses_only_the_isolated_binary_and_passes_arguments(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            tool_home = Path(temp) / "tools"
            binary = tool_home / "skillspector/2.13.2/venv/bin/skillspector"
            binary.parent.mkdir(parents=True)
            binary.write_text("#!/bin/sh\nprintf '%s\\n' \"$@\"\n", encoding="utf-8")
            binary.chmod(0o755)

            result = subprocess.run(
                [
                    "/bin/bash",
                    str(RUNNER_PATH),
                    "scan",
                    "/tmp/untrusted-skill",
                    "--no-llm",
                ],
                check=False,
                capture_output=True,
                text=True,
                env={**os.environ, "CFA4_SKILLSPECTOR_HOME": str(tool_home)},
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                result.stdout.splitlines(),
                ["scan", "/tmp/untrusted-skill", "--no-llm"],
            )

    def test_runner_fails_closed_when_isolated_cli_is_missing(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            tool_home = Path(temp) / "empty"
            fake_path = Path(temp) / "path"
            fake_path.mkdir()
            fallback = fake_path / "skillspector"
            fallback.write_text("#!/bin/sh\necho PATH fallback used\n", encoding="utf-8")
            fallback.chmod(0o755)

            result = subprocess.run(
                ["/bin/bash", str(RUNNER_PATH), "scan", "/tmp/untrusted-skill"],
                check=False,
                capture_output=True,
                text=True,
                env={
                    **os.environ,
                    "CFA4_SKILLSPECTOR_HOME": str(tool_home),
                    "PATH": str(fake_path) + os.pathsep + os.environ.get("PATH", ""),
                },
            )

            self.assertEqual(result.returncode, 127)
            self.assertNotIn("PATH fallback used", result.stdout)
            self.assertIn("install-skillspector.sh", result.stderr)


if __name__ == "__main__":
    unittest.main()

