"""Offline, synthetic artifact tests. Not a substitute for real package builds."""
from __future__ import annotations

import hashlib
from io import BytesIO
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

RELEASE_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(RELEASE_DIR))
import verify_cfa4_packages as audit

VERSION = "0.10.0"
ROOT_NAME = f"cfa4-{VERSION}-linux-x86_64"
DOC = "share/doc/cfa4"
HAVE_SQUASHFS = bool(shutil.which("mksquashfs") and shutil.which("unsquashfs"))


def elf_runtime() -> bytes:
    """A 64-byte x86_64 ELF header whose (empty) section header table ends at byte 64."""
    elf = bytearray(64)
    elf[:6] = b"\x7fELF\x02\x01"
    elf[18:20] = (62).to_bytes(2, "little")
    elf[40:48] = (64).to_bytes(8, "little")
    elf[58:60] = (64).to_bytes(2, "little")
    return bytes(elf)


def doc_entries(base: str) -> dict[str, bytes]:
    return {f"{base}/{name}": b"licence text" for name in audit.package_docs()}


class FourFormatArtifactTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="cfa4-four-formats-")
        self.addCleanup(self.tmp.cleanup)
        self.dist = Path(self.tmp.name)
        self.names = audit.expected_names(VERSION)
        for name in self.names:
            path = self.dist / name
            if name.endswith(".AppImage"):
                elf = bytearray(64)
                elf[:6] = b"\x7fELF\x02\x01"
                elf[18:20] = (62).to_bytes(2, "little")
                path.write_bytes(elf)
            elif name.endswith(".deb"):
                path.write_bytes(b"!<arch>\nfixture")
            elif name.endswith(".flatpak"):
                path.write_bytes(b"Flatpak fixture not a real bundle")
            else:
                self.make_tar(path)
        self.write_sums()

    def make_tar(self, path, *, remove=(), wrong_version=False, unsafe=False, extra=()):
        entries = {
            "install.sh": b"#!/bin/sh\n",
            "bin/cfa4": b"native editor",
            "bin/cfa4-cli": b"native cli",
            f"share/applications/{audit.APP_ID}.desktop":
                f"[Desktop Entry]\nName=CFA4\nIcon={audit.APP_ID}\n".encode(),
            f"share/mime/packages/{audit.APP_ID}.xml": b"<mime-info/>",
            f"share/metainfo/{audit.APP_ID}.metainfo.xml":
                (f"<component><id>{audit.APP_ID}</id><releases><release "
                 f"version=\"{'0.9.9' if wrong_version else VERSION}\"/></releases>"
                 f"</component>").encode(),
            f"share/icons/hicolor/scalable/apps/{audit.APP_ID}.svg": b"<svg/>",
            f"{audit.FFMPEG_DIR}/lib/libavcodec.so.62": b"\x7fELF fixture",
            f"{audit.FFMPEG_DIR}/lib/libavformat.so.62": b"\x7fELF fixture",
            f"{audit.FFMPEG_DIR}/lib/libavutil.so.60": b"\x7fELF fixture",
            f"{audit.FFMPEG_DIR}/lib/libdav1d.so.7": b"\x7fELF fixture",
            f"{audit.FFMPEG_DIR}/share/doc/ffmpeg/BUILDINFO": b"FFmpeg 9.0.2",
            f"{audit.FFMPEG_DIR}/share/doc/ffmpeg/COPYING.GPLv3": b"GPL",
            f"{audit.FFMPEG_DIR}/share/doc/ffmpeg/LICENSE.md": b"licence",
            **doc_entries(DOC),
            **{name: b"fixture" for name in extra},
        }
        with tarfile.open(path, "w:gz") as archive:
            for filename, content in entries.items():
                if filename in remove:
                    continue
                info = tarfile.TarInfo(f"{ROOT_NAME}/{filename}")
                info.mode = 0o755 if filename in ("install.sh", "bin/cfa4", "bin/cfa4-cli") else 0o644
                info.size = len(content)
                archive.addfile(info, BytesIO(content))
            if unsafe:
                data = b"unsafe"
                bad = tarfile.TarInfo(f"{ROOT_NAME}/../evil")
                bad.size = len(data)
                archive.addfile(bad, BytesIO(data))

    def write_sums(self):
        lines = []
        for name in self.names:
            sha = hashlib.sha256((self.dist / name).read_bytes()).hexdigest()
            lines.append(f"{sha}  {name}")
        (self.dist / "SHA256SUMS").write_text("\n".join(lines) + "\n")

    def make_appimage(self, path, *, remove=(), extra=()):
        """A real squashfs behind an ELF runtime header, laid out as package.sh builds it."""
        tree = Path(self.tmp.name) / "AppDir"
        shutil.rmtree(tree, ignore_errors=True)
        ffmpeg = f"usr/{audit.FFMPEG_DIR}"
        entries = {
            "AppRun": b"#!/bin/sh\n",
            f"{audit.APP_ID}.desktop": b"[Desktop Entry]\n",
            f"{audit.APP_ID}.png": b"PNG",
            "usr/bin/cfa4": b"editor", "usr/bin/cfa4-cli": b"cli", "usr/bin/cfa4-qt": b"qt",
            "usr/lib/cfa4/python/cfa4_qt/__main__.py": b"main()",
            f"{audit.PYSIDE6_DIR}/PySide6/__init__.py": b"#",
            f"usr/share/metainfo/{audit.APP_ID}.metainfo.xml": b"<component/>",
            f"{ffmpeg}/lib/libavcodec.so.62": b"ELF", f"{ffmpeg}/lib/libavformat.so.62": b"ELF",
            f"{ffmpeg}/lib/libavutil.so.60": b"ELF",
            **{f"{ffmpeg}/{doc}": b"text" for doc in audit.FFMPEG_DOCS},
            **doc_entries(f"usr/{DOC}"),
            **{f"usr/{DOC}/pyside6/{doc}": b"LGPL" for doc in audit.PYSIDE6_DOCS},
            **{name: b"fixture" for name in extra},
        }
        for name, content in entries.items():
            if name in remove:
                continue
            target = tree / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
        (tree / ".DirIcon").symlink_to(f"{audit.APP_ID}.png")
        image = Path(self.tmp.name) / "image.squashfs"
        subprocess.run(["mksquashfs", str(tree), str(image), "-noappend", "-quiet",
                        "-all-root"], check=True, capture_output=True)
        path.write_bytes(elf_runtime() + image.read_bytes())

    def test_four_format_structure_is_checked_without_install(self):
        with patch.object(audit, "verify_deb") as fake_deb, \
                patch.object(audit, "verify_appimage") as fake_appimage:
            report = audit.verify_package_set(self.dist, VERSION)
        self.assertEqual(report["status"], "PASS_PACKAGE_STRUCTURE_NOT_RUNTIME")
        self.assertEqual(len(report["files"]), 4)
        fake_deb.assert_called_once()
        fake_appimage.assert_called_once()

    def test_package_docs_match_the_shell_check(self):
        script = (RELEASE_DIR.parents[1] / "scripts/ci/check-package-docs.sh").read_text()
        for name in audit.package_docs():
            if not name.startswith("OFL-"):
                self.assertIn(name, script)

    def test_reject_tarball_without_licence_file(self):
        path = self.dist / self.names[3]
        self.make_tar(path, remove={f"{DOC}/THIRD-PARTY-LICENSES.txt"})
        with self.assertRaisesRegex(audit.AuditError, "THIRD-PARTY-LICENSES.txt"):
            audit.verify_tarball(path, VERSION)

    def test_deb_listing_needs_licence_files(self):
        def listing(skip=()):
            files = {
                "usr/bin/cfa4": 9, "usr/bin/cfa4-cli": 9, "usr/bin/cfa4-qt": 9,
                "usr/lib/cfa4/python/cfa4_qt/__main__.py": 9,
                f"usr/share/applications/{audit.APP_ID}.desktop": 9,
                f"usr/share/mime/packages/{audit.APP_ID}.xml": 9,
                f"usr/share/metainfo/{audit.APP_ID}.metainfo.xml": 9,
                f"usr/share/icons/hicolor/scalable/apps/{audit.APP_ID}.svg": 9,
                f"{audit.FFMPEG_DEB_DIR}/lib/libavcodec.so.62": 9,
                f"{audit.FFMPEG_DEB_DIR}/lib/libavformat.so.62": 9,
                f"{audit.FFMPEG_DEB_DIR}/lib/libavutil.so.60": 9,
                **{f"{audit.FFMPEG_DEB_DIR}/{doc}": 9 for doc in audit.FFMPEG_DOCS},
                **{f"usr/{DOC}/{doc}": 9 for doc in audit.package_docs()},
            }
            lines = ["drwxr-xr-x root/root 0 2026-11-10 12:00 ./usr/"]
            lines += [f"-rw-r--r-- root/root {size} 2026-11-10 12:00 ./{name}"
                      for name, size in files.items() if name not in skip]
            lines.append(f"lrwxrwxrwx root/root 0 2026-11-10 12:00 ./{audit.FFMPEG_DEB_DIR}"
                         "/lib/libavcodec.so -> libavcodec.so.62")
            return "\n".join(lines)

        def fake_dpkg(contents):
            def run(args, **_):
                if args[1] == "--contents":
                    return contents
                return {"Package": "cfa4", "Architecture": "amd64", "Version": f"{VERSION}-1",
                        "Depends": "libgstreamer1.0-0"}[args[3]]
            return run

        deb = self.dist / self.names[0]
        with patch.object(audit.subprocess, "check_output", side_effect=fake_dpkg(listing())):
            audit.verify_deb(deb, VERSION)
        with patch.object(audit.subprocess, "check_output",
                          side_effect=fake_dpkg(listing({f"usr/{DOC}/copyright"}))):
            with self.assertRaisesRegex(audit.AuditError, "licence/attribution.*copyright"):
                audit.verify_deb(deb, VERSION)
        with patch.object(audit.subprocess, "check_output",
                          side_effect=fake_dpkg(listing({"usr/bin/cfa4-qt"}))):
            with self.assertRaisesRegex(audit.AuditError, "missing ./usr/bin/cfa4-qt"):
                audit.verify_deb(deb, VERSION)

    def test_listing_rejects_entries_outside_the_package(self):
        with self.assertRaisesRegex(audit.AuditError, "unexpected package entry"):
            audit.parse_listing("-rw-r--r-- root/root 1 2026-11-10 12:00 /etc/passwd", "./")
        with self.assertRaisesRegex(audit.AuditError, "unsafe"):
            audit.parse_listing("-rw-r--r-- root/root 1 2026-11-10 12:00 ./usr/../../x", "./")

    @unittest.skipUnless(HAVE_SQUASHFS, "squashfs-tools (mksquashfs, unsquashfs) not installed")
    def test_appimage_contents_are_listed_without_running_it(self):
        path = self.dist / self.names[1]
        self.make_appimage(path)
        audit.verify_appimage(path)
        for missing, message in (
            (f"usr/{DOC}/LICENSE", "licence/attribution.*'LICENSE'"),
            (f"usr/{DOC}/pyside6/LICENSE.LGPL-3.0", "PySide6 / Qt licences"),
            (f"usr/{audit.FFMPEG_DIR}/lib/libavutil.so.60", "lacks FFmpeg 9.0.2 lib/libavutil"),
            ("usr/bin/cfa4-qt", "AppImage missing usr/bin/cfa4-qt"),
        ):
            with self.subTest(missing=missing):
                self.make_appimage(path, remove={missing})
                with self.assertRaisesRegex(audit.AuditError, message):
                    audit.verify_appimage(path)
        self.make_appimage(path, extra={f"usr/{audit.FFMPEG_DIR}/lib/libx265.so.215"})
        with self.assertRaisesRegex(audit.AuditError, "patent-restricted"):
            audit.verify_appimage(path)

    def test_reject_appimage_without_squashfs(self):
        path = self.dist / self.names[1]
        path.write_bytes(elf_runtime() + b"not a filesystem")
        with self.assertRaisesRegex(audit.AuditError, "no squashfs"):
            audit.verify_appimage(path)

    def test_reject_modified_bytes(self):
        (self.dist / self.names[2]).write_bytes(b"changed")
        with self.assertRaisesRegex(audit.AuditError, "SHA256 mismatch"):
            audit.verify_checksums(self.dist, self.names)

    def test_reject_wrong_artifact_name(self):
        old = self.names[0]
        (self.dist / "SHA256SUMS").write_text(
            (self.dist / "SHA256SUMS").read_text().replace(old, "../fake.deb")
        )
        with self.assertRaisesRegex(audit.AuditError, "invalid SHA256SUMS entry"):
            audit.verify_checksums(self.dist, self.names)

    def test_reject_missing_tarball_component(self):
        path = self.dist / self.names[3]
        self.make_tar(path, remove={"bin/cfa4-cli"})
        with self.assertRaisesRegex(audit.AuditError, "missing required entries"):
            audit.verify_tarball(path, VERSION)

    def test_reject_tarball_version_drift(self):
        path = self.dist / self.names[3]
        self.make_tar(path, wrong_version=True)
        with self.assertRaisesRegex(audit.AuditError, "AppStream version"):
            audit.verify_tarball(path, VERSION)

    def test_reject_path_traversal_in_tarball(self):
        path = self.dist / self.names[3]
        self.make_tar(path, unsafe=True)
        with self.assertRaisesRegex(audit.AuditError, "unsafe"):
            audit.verify_tarball(path, VERSION)

    def test_reject_tarball_without_ffmpeg(self):
        path = self.dist / self.names[3]
        self.make_tar(path, remove={f"{audit.FFMPEG_DIR}/lib/libavcodec.so.62"})
        with self.assertRaisesRegex(audit.AuditError, "lacks FFmpeg 9.0.2 lib/libavcodec"):
            audit.verify_tarball(path, VERSION)

    def test_reject_tarball_without_ffmpeg_licence(self):
        path = self.dist / self.names[3]
        self.make_tar(path, remove={f"{audit.FFMPEG_DIR}/share/doc/ffmpeg/BUILDINFO"})
        with self.assertRaisesRegex(audit.AuditError, "licence/source file"):
            audit.verify_tarball(path, VERSION)

    def test_reject_patent_restricted_codec_in_base_tarball(self):
        path = self.dist / self.names[3]
        self.make_tar(path, extra={f"{audit.FFMPEG_DIR}/lib/libx264.so.164"})
        with self.assertRaisesRegex(audit.AuditError, "patent-restricted"):
            audit.verify_tarball(path, VERSION)

    def test_ffmpeg_check_accepts_deb_layout(self):
        names = {f"{audit.FFMPEG_DEB_DIR}/{n}" for n in (
            "lib/libavcodec.so.62", "lib/libavformat.so.62", "lib/libavutil.so.60", *audit.FFMPEG_DOCS)}
        audit.verify_ffmpeg(names, audit.FFMPEG_DEB_DIR, ".deb")
        with self.assertRaisesRegex(audit.AuditError, "patent-restricted"):
            audit.verify_ffmpeg(names | {"opt/cfa4/ffmpeg-9.0.2-restricted/lib/libavcodec.so.62"},
                                audit.FFMPEG_DEB_DIR, ".deb")

    def test_reject_other_cpu_architecture_in_appimage(self):
        path = self.dist / self.names[1]
        elf = bytearray(path.read_bytes())
        elf[18:20] = (183).to_bytes(2, "little")
        path.write_bytes(elf)
        with self.assertRaisesRegex(audit.AuditError, "not x86_64"):
            audit.verify_appimage(path)

    def test_reject_non_deb_magic(self):
        (self.dist / self.names[0]).write_bytes(b"WRONG")
        with self.assertRaisesRegex(audit.AuditError, "invalid .deb"):
            audit.verify_deb(self.dist / self.names[0], VERSION)


if __name__ == "__main__":
    unittest.main()
