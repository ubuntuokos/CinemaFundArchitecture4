#!/usr/bin/env python3
"""The Linux packages and DCP validation (plan DCP-18, docs/plans/DCP-VALIDATION-2026-10-27.md §7.3).

Static checks of the packaging sources: every package method ships the engine that serves the
`dcp.*` commands, none bundles Ruby or the optional dcp_inspect reference engine, none installs
gems globally, and the Flatpak sandbox can reach the folders a DCP is exported to or validated in.
"""

import re
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
APP_ID = "io.github.ubuntuokos.cfa4"
SOURCES = ("package.sh", "nfpm.yaml", "install.sh", "AppRun", "postinst.sh")


def sources():
    for name in SOURCES:
        yield name, (HERE / name).read_text()
    for path in sorted((HERE / "flatpak").glob("*.yml")):
        yield f"flatpak/{path.name}", path.read_text()


class DcpValidationPackagingTests(unittest.TestCase):
    def test_engine_with_dcp_commands_is_in_every_package(self):
        self.assertIn('install -Dm755 "$BIN/cfa4-cli" "$STAGE/usr/bin/cfa4-cli"', (HERE / "package.sh").read_text())
        self.assertIn("dst: /usr/bin/cfa4-cli", (HERE / "nfpm.yaml").read_text())
        # the AppImage is assembled from the same stage as the .deb (package.sh)
        self.assertIn("-p cfa4-cli", (HERE / "flatpak" / f"{APP_ID}.yml").read_text())
        bundle = (HERE / "flatpak" / f"{APP_ID}.bundle.yml").read_text()
        self.assertIn("install -Dm755 bin/cfa4-cli /app/bin/cfa4-cli", bundle)

    def test_no_package_bundles_ruby_or_the_reference_engine(self):
        for name, text in sources():
            self.assertNotRegex(text, r"(?i)\bruby\b|\bgem install\b|nokogiri|dcp_inspect|dcp-inspect", name)

    def test_no_package_writes_global_gems(self):
        for name, text in sources():
            self.assertNotRegex(text, r"(?i)\bgem\s+(install|update)\b|GEM_HOME|GEM_PATH", name)

    def test_flatpak_reaches_dcp_folders(self):
        manifest = (HERE / "flatpak" / f"{APP_ID}.yml").read_text()
        grants = set(re.findall(r"(?m)^\s*- --filesystem=(\S+)", manifest))
        # exports and imported packages live in the home directory or on external drives
        self.assertTrue({"home", "/media", "/run/media", "/mnt"} <= grants, grants)
        args = re.findall(r"(?m)^\s*- (--\S+)", manifest.split("finish-args:", 1)[1].split("build-options:", 1)[0])
        self.assertNotIn("--share=network", args)
        self.assertFalse([a for a in args if re.fullmatch(r"--filesystem=(host|host-os|host-etc)(:\w+)?", a)], args)


if __name__ == "__main__":
    unittest.main()
