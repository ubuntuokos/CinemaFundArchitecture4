#!/usr/bin/env python3
"""CFA4 F6 XDG metadata consistency, no graphical session or external packages."""
from __future__ import annotations

import configparser
import re
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

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
    match = re.search(rf"^{re.escape(field)}:\s*(.+?)\s*$", content, flags=re.MULTILINE)
    if not match:
        raise AssertionError(f"{filename}: {field} missing")
    return match.group(1).strip("'\"")


class XDGDesktopMetadataTests(unittest.TestCase):
    def test_desktop_identity_and_cli_launch(self) -> None:
        entry = desktop_metadata()
        self.assertEqual(entry["Type"], "Application")
        self.assertEqual(entry["Name"], "CFA4")
        self.assertEqual(entry["Icon"], APP)
        # the Qt 6 GUI is what the menu and the file manager start (owner decision 2026-11-12)
        self.assertEqual(entry["Exec"], "cfa4-qt %F")
        self.assertEqual(entry["TryExec"], "cfa4-qt")
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

    def test_credits_types_open_in_the_qt_workspace_and_claim_nothing_else(self) -> None:
        """Credits plan C9: `.cfa4credits` / `.cfa4credtpl` are CFA4's own types in its namespace,
        opened by the Qt GUI through a hidden entry; Cinecred's files are never claimed."""
        tree = ET.parse(PACKAGE / f"{APP}.mime.xml")
        globs = {
            t.get("type"): [g.get("pattern") for g in t.findall(MIME_NS + "glob")]
            for t in tree.getroot().findall(MIME_NS + "mime-type")
        }
        self.assertEqual(globs["application/x-cfa4-credits"], ["*.cfa4credits"])
        self.assertEqual(globs["application/x-cfa4-credits-template"], ["*.cfa4credtpl"])
        for t, patterns in globs.items():
            self.assertTrue(t.startswith("application/x-cfa4"), t)
            for p in patterns:
                # spreadsheets, TOML (Cinecred's Styling.toml) and JSON stay with their own apps
                self.assertFalse(p.lower().endswith((".toml", ".csv", ".ods", ".xlsx", ".json", ".tsv")), p)
        config = configparser.ConfigParser(interpolation=None, strict=True)
        config.optionxform = str
        with (PACKAGE / f"{APP}.credits.desktop").open(encoding="utf-8") as file:
            config.read_file(file)
        entry = config["Desktop Entry"]
        self.assertEqual(entry["Exec"], "cfa4-qt %f")
        self.assertEqual(entry["TryExec"], "cfa4-qt")
        self.assertEqual(entry["NoDisplay"], "true")
        self.assertEqual(entry["Icon"], APP)
        self.assertEqual(set(filter(None, entry["MimeType"].split(";"))), {"application/x-cfa4-credits", "application/x-cfa4-credits-template"})
        # the main entry does not claim them: the hidden credits entry opens them in the workspace
        self.assertNotIn("application/x-cfa4-credits", desktop_metadata()["MimeType"])
        contents = (PACKAGE / "package.sh").read_text(encoding="utf-8")
        self.assertIn("$HERE/$APP_ID.credits.desktop", contents)

    def test_credits_module_brings_no_binary_service_or_port(self) -> None:
        """Credits plan C9: the credits module is a library inside CFA4: no program, service or
        network port of its own, so it cannot clash with Cinecred or anything else."""
        crate = ROOT / "crates/credits"
        manifest = (crate / "Cargo.toml").read_text(encoding="utf-8")
        self.assertNotIn("[[bin]]", manifest)
        self.assertFalse((crate / "src/main.rs").exists() or (crate / "src/bin").exists())
        for src in (crate / "src").rglob("*.rs"):
            text = src.read_text(encoding="utf-8")
            for banned in ("std::net", "TcpListener", "UdpSocket", "std::process::Command"):
                self.assertNotIn(banned, text, f"{src}: {banned}")
        services = [p for p in PACKAGE.rglob("*") if p.suffix in (".service", ".socket") or "dbus" in p.name.lower()]
        self.assertEqual(services, [])

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
        self.assertEqual([b.text for b in xml.findall("./provides/binary")], ["cfa4", "cfa4-qt"])
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
        # both start the Qt 6 GUI (owner decision 2026-11-12)
        self.assertEqual(flatpak_value(bundle, "command"), "cfa4-qt")

    def test_both_flatpaks_build_kerberos_for_the_qt_gui(self) -> None:
        """The Qt 6 wheels' QtNetwork and QtMultimedia need libgssapi_krb5.so.2, which the freedesktop
        runtime lacks: both manifests build the same pinned MIT krb5 before CFA4, keep its licence,
        and the bundle's smoke test imports every module the GUI loads."""
        modules = {}
        for name in (f"{APP}.yml", f"{APP}.bundle.yml"):
            text = (PACKAGE / "flatpak" / name).read_text(encoding="utf-8")
            self.assertLess(text.index("  - name: krb5\n"), text.index("  - name: cfa4\n"), name)
            block = text[text.index("  - name: krb5\n"):text.index("  - name: cfa4\n")]
            self.assertIn("install -Dm644 ../NOTICE /app/share/doc/cfa4/krb5/NOTICE", block)
            self.assertRegex(block, r"url: https://kerberos\.org/dist/krb5/\S+\.tar\.gz\n\s+sha256: [0-9a-f]{64}")
            modules[name] = block
        self.assertEqual(modules[f"{APP}.yml"], modules[f"{APP}.bundle.yml"])
        smoke = (PACKAGE / "flatpak-bundle.sh").read_text(encoding="utf-8")
        self.assertIn("from PySide6 import QtCore, QtGui, QtWidgets, QtNetwork, QtTest, QtMultimedia, QtDBus", smoke)

    def test_from_source_flatpak_builds_the_qt_gui_from_the_pinned_wheels(self) -> None:
        """The development manifest stages the Qt GUI as package.sh does, with PySide6 / Qt 6 from
        wheels whose SHA-256 are the ones pinned in requirements/linux-amd64.txt."""
        source = (PACKAGE / "flatpak" / f"{APP}.yml").read_text(encoding="utf-8")
        for step in (
            "install -Dm755 packaging/linux/cfa4-qt /app/bin/cfa4-qt",
            "cp -R python/cfa4_qt /app/lib/cfa4/python/",
            "cp i18n/*.tsv i18n/*.tsv.attribution /app/lib/cfa4/python/cfa4_qt/i18n/",
            "python3 packaging/linux/stage-pyside6.py --wheels pyside6-wheels --licences gnu-licences --dest /app/lib/cfa4/pyside6",
            "dest-filename: LGPL-3",
            "dest-filename: LGPL-2.1",
            f"packaging/linux/{APP}.credits.desktop",
        ):
            self.assertIn(step, source)
        pins = (ROOT / "requirements" / "linux-amd64.txt").read_text(encoding="utf-8")
        wheels = re.findall(r"url: \S+/((?:pyside6_essentials|pyside6_addons|shiboken6)-6\.12\.0-\S+\.whl)\n\s+sha256: ([0-9a-f]{64})", source)
        self.assertEqual(sorted(w.split("-")[0] for w, _ in wheels), ["pyside6_addons", "pyside6_essentials", "shiboken6"])
        for wheel, digest in wheels:
            self.assertIn("manylinux_2_34_x86_64", wheel)
            self.assertIn(f"--hash=sha256:{digest}", pins, wheel)

    def test_local_package_install_paths_are_cfa4_only(self) -> None:
        contents = (PACKAGE / "package.sh").read_text(encoding="utf-8")
        self.assertIn(f"APP_ID={APP}", contents)
        self.assertIn("$HERE/$APP_ID.desktop", contents)
        self.assertIn("$HERE/$APP_ID.mime.xml", contents)
        self.assertIn("$HERE/$APP_ID.metainfo.xml.in", contents)
        self.assertIn("assets/cfa4/hicolor", contents)


if __name__ == "__main__":
    unittest.main()
