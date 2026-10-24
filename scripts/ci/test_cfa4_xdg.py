#!/usr/bin/env python3
"""CFA4 F6 XDG metadata consistency, no graphical session or external packages."""
from __future__ import annotations

import configparser
from pathlib import Path
import re
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
APP = "io.github.ubuntuokos.cfa4"
PACKAGE = ROOT / "packaging/linux"
ICON = ROOT / "assets/cfa4/hicolor/scalable/apps" / f"{APP}.svg"
MIME_NS = "{http://www.freedesktop.org/standards/shared-mime-info}"


def desktop_metadata() -> configparser.SectionProxy:
    config = configparser.ConfigParser(interpolation=None, strict=True)
    config.optionxform = str
    with (PACKAGE / f"{APP}.desktop").open(encoding="utf-8") as file:
        config.read_file(file)
    return config["Desktop Entry"]


def flatpak_value(filename: str, field: str) -> str:
    content = (PACKAGE / "flatpak" / filename).read_text(encoding="utf-8")
    match = re.search(rf"^{re.escape(field)}:\s*(.+?)\s*$", content, flags=re.M)
    if not match:
        raise AssertionError(f"{filename}: {field} missing")
    return match.group(1).strip("'\"")


class XDGDesktopMetadataTests(unittest.TestCase):
    def test_desktop_identity_and_cli_launch(self) -> None:
        entry = desktop_metadata()
        self.assertEqual(entry["Type"], "Application")
        self.assertEqual(entry["Name"], "CFA4")
        self.assertEqual(entry["Icon"], APP)
        self.assertEqual(entry["Exec"], "cfa4 %F")
        self.assertEqual(entry["TryExec"], "cfa4")
        self.assertEqual(entry["Terminal"], "false")
        self.assertIn("AudioVideo", entry["Categories"].split(";"))

    def test_mime_project_compatibility_and_icon(self) -> None:
        types = set(filter(None, desktop_metadata()["MimeType"].split(";")))
        self.assertIn("application/x-cfa4", types)
        tree = ET.parse(PACKAGE / f"{APP}.mime.xml")
        self.assertEqual(tree.getroot().tag, MIME_NS + "mime-info")
        project = tree.find(MIME_NS + "mime-type")
        self.assertIsNotNone(project)
        self.assertEqual(project.get("type"), "application/x-cfa4")
        self.assertEqual(project.find(MIME_NS + "icon").get("name"), APP)
        self.assertEqual(project.find(MIME_NS + "glob").get("pattern"), "*.fcproj")

    def test_cfa4_icon_is_valid_svg_and_has_identity(self) -> None:
        root = ET.parse(ICON).getroot()
        self.assertEqual(root.tag, "{http://www.w3.org/2000/svg}svg")
        self.assertIn("512", root.get("viewBox", ""))
        title = root.find("{http://www.w3.org/2000/svg}title")
        self.assertIsNotNone(title)
        self.assertIn("CFA4", title.text or "")

    def test_appstream_identity_and_nontrivial_first_paragraph(self) -> None:
        xml = ET.parse(PACKAGE / f"{APP}.metainfo.xml.in").getroot()
        self.assertEqual(xml.findtext("id"), APP)
        self.assertEqual(xml.find("launchable").text, f"{APP}.desktop")
        self.assertEqual(xml.find("icon").text, APP)
        self.assertEqual(xml.findtext("./provides/binary"), "cfa4")
        self.assertEqual(xml.findtext("./provides/mediatype"), "application/x-cfa4")
        desc = xml.find("./description/p")
        self.assertIsNotNone(desc)
        paragraph = " ".join("".join(desc.itertext()).split())
        self.assertGreaterEqual(len(paragraph), 180)
        self.assertIn("Ubuntu 26.04", paragraph)
        self.assertIn("under development", paragraph)
        release = xml.find("./releases/release")
        self.assertEqual(release.get("version"), "@VERSION@")
        self.assertEqual(release.get("date"), "@DATE@")

    def test_flatpak_manifests_share_application_and_runtime(self) -> None:
        from_source = f"{APP}.yml"
        bundle = f"{APP}.bundle.yml"
        for field in ("id", "runtime", "runtime-version", "sdk", "command"):
            self.assertEqual(flatpak_value(from_source, field), flatpak_value(bundle, field))
        self.assertEqual(flatpak_value(from_source, "id"), APP)
        self.assertEqual(flatpak_value(bundle, "command"), "cfa4")

    def test_local_package_install_paths_are_cfa4_only(self) -> None:
        contents = (PACKAGE / "package.sh").read_text(encoding="utf-8")
        self.assertIn(f"APP_ID={APP}", contents)
        self.assertIn(f"$HERE/$APP_ID.desktop", contents)
        self.assertIn(f"$HERE/$APP_ID.mime.xml", contents)
        self.assertIn(f"$HERE/$APP_ID.metainfo.xml.in", contents)
        self.assertIn("assets/cfa4/hicolor", contents)


if __name__ == "__main__":
    unittest.main()
