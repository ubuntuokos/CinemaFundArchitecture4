"""Offline, synthetic artifact tests. Not a substitute for real package builds."""
from __future__ import annotations

import hashlib
from io import BytesIO
from pathlib import Path
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

    def make_tar(self, path, *, remove=(), wrong_version=False, unsafe=False):
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

    def test_four_format_structure_is_checked_without_install(self):
        with patch.object(audit, "verify_deb") as fake_deb:
            report = audit.verify_package_set(self.dist, VERSION)
        self.assertEqual(report["status"], "PASS_PACKAGE_STRUCTURE_NOT_RUNTIME")
        self.assertEqual(len(report["files"]), 4)
        fake_deb.assert_called_once()

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
