#!/usr/bin/env python3
"""Validate CFA4 local candidate artifacts without publishing or installing packages.

This is a package-structure/checksum gate, not a clean-install or license gate. It lists the
real contents of the .deb (dpkg-deb) and of the AppImage (its squashfs, with unsquashfs, without
running it) and requires the licence and attribution files in each (#220).
The Flatpak bundle's internal ref and the AppImage runtime are tested in F5.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tomllib
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
APP_ID = "io.github.ubuntuokos.cfa4"
FORMATS = ("deb", "AppImage", "flatpak", "tar.gz")
# CFA4's FFmpeg 9.0.2 (ADR 0005 step 4): in /opt/cfa4 in the .deb, beside the binaries elsewhere.
FFMPEG_DIR = "lib/cfa4/ffmpeg-9.0.2"
FFMPEG_DEB_DIR = "opt/cfa4/ffmpeg-9.0.2"
FFMPEG_REQUIRED = ("lib/libavcodec.so.", "lib/libavformat.so.", "lib/libavutil.so.")
FFMPEG_DOCS = ("share/doc/ffmpeg/BUILDINFO", "share/doc/ffmpeg/COPYING.GPLv3", "share/doc/ffmpeg/LICENSE.md")
# Patent-restricted libraries belong only in cfa4-restricted-extras (docs/licensing.md §7).
RESTRICTED = re.compile(r"(^|/)libx26[45]\.so")
# The AppImage's and the Flatpak's own PySide6 / Qt 6 (stage-pyside6.py) and its licences.
PYSIDE6_DIR = "usr/lib/cfa4/pyside6"
PYSIDE6_DOCS = ("LICENSE.LGPL-3.0", "LICENSE.LGPL-2.1", "PYSIDE6-QT-NOTICE.txt")


def package_docs(root: Path = ROOT) -> tuple[str, ...]:
    """The licence/attribution files every package carries in share/doc/cfa4
    (scripts/ci/check-package-docs.sh, the same list)."""
    fonts = sorted(path.name for path in (root / "assets/fonts").glob("OFL-*.txt"))
    return ("LICENSE", "NOTICE", "ATTRIBUTION.md", "LICENSE-MIT", "LICENSE-APACHE",
            "LICENSE-logo.txt", "copyright", "THIRD-PARTY-LICENSES.txt", "cfa4.cdx.json", *fonts)


class AuditError(ValueError):
    pass


def repo_version(root: Path = ROOT) -> str:
    data = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    version = data["workspace"]["package"]["version"]
    if not isinstance(version, str) or not re.fullmatch(r"[0-9A-Za-z][0-9A-Za-z.+-]*", version):
        raise AuditError("invalid Cargo.toml workspace package version")
    return version


def expected_names(version: str) -> tuple[str, ...]:
    prefix = f"cfa4-{version}-linux-x86_64"
    return tuple(f"{prefix}.{extension}" for extension in FORMATS)


def file_digest(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as src:
        for block in iter(lambda: src.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def verify_checksums(dist: Path, names: tuple[str, ...]) -> dict[str, str]:
    checksum_file = dist / "SHA256SUMS"
    try:
        lines = checksum_file.read_text(encoding="ascii").splitlines()
    except (OSError, UnicodeError) as exc:
        raise AuditError(f"missing or invalid SHA256SUMS: {exc}") from exc
    received: dict[str, str] = {}
    for line in lines:
        match = re.fullmatch(r"([0-9a-f]{64})  ([^/\\]+)", line)
        if match is None:
            raise AuditError("invalid SHA256SUMS entry")
        sha, name = match.groups()
        if name in received:
            raise AuditError(f"duplicate checksum entry: {name}")
        received[name] = sha
    if set(received) != set(names):
        raise AuditError(f"wrong artifact names in SHA256SUMS: {sorted(received)}")
    for name in names:
        path = dist / name
        if not path.is_file() or path.is_symlink() or not path.stat().st_size:
            raise AuditError(f"missing, empty or unsafe package: {name}")
        actual = file_digest(path)
        if actual != received[name]:
            raise AuditError(f"SHA256 mismatch: {name}")
    return received


def verify_ffmpeg(names: set[str], base: str, package: str) -> None:
    """The FFmpeg 9.0.2 libraries and their licences under `base`, and no restricted codec."""
    for prefix in FFMPEG_REQUIRED:
        if not any(name.startswith(f"{base}/{prefix}") for name in names):
            raise AuditError(f"{package} lacks FFmpeg 9.0.2 {prefix}* in {base}")
    for doc in FFMPEG_DOCS:
        if f"{base}/{doc}" not in names:
            raise AuditError(f"{package} lacks the FFmpeg licence/source file {base}/{doc}")
    restricted = sorted(name for name in names if RESTRICTED.search(name))
    if restricted or any("-restricted/" in name for name in names):
        raise AuditError(f"{package} contains patent-restricted codecs: {restricted}")


def verify_docs(sizes: dict[str, int], base: str, package: str) -> None:
    """Every licence/attribution file, non-empty, in `base` (a share/doc/cfa4 directory)."""
    missing = [doc for doc in package_docs() if sizes.get(f"{base}/{doc}", 0) <= 0]
    if missing:
        raise AuditError(f"{package} lacks licence/attribution files in {base}: {missing}")


def parse_listing(listing: str, strip: str) -> dict[str, int]:
    """Regular files and links of a `dpkg-deb --contents` / `unsquashfs -lls` listing
    ("-rw-r--r-- root/root 1234 2026-11-08 07:00 ./usr/bin/cfa4"), relative to `strip`, with their
    sizes (a link counts as non-empty)."""
    entries: dict[str, int] = {}
    for line in listing.splitlines():
        fields = line.split(None, 5)
        if len(fields) < 6 or fields[0][:1] not in ("-", "l"):
            continue
        name = fields[5].split(" -> ", 1)[0] if fields[0].startswith("l") else fields[5]
        if not name.startswith(strip):
            raise AuditError(f"unexpected package entry: {name}")
        name = name[len(strip):]
        if not name or ".." in Path(name).parts:
            raise AuditError(f"unsafe package entry: {fields[5]}")
        try:
            size = int(fields[2])
        except ValueError as exc:
            raise AuditError(f"unreadable package listing line: {line}") from exc
        entries[name] = max(size, 1) if fields[0].startswith("l") else size
    return entries


def verify_tarball(path: Path, version: str) -> None:
    folder = f"cfa4-{version}-linux-x86_64"
    required = {
        "install.sh",
        "bin/cfa4",
        "bin/cfa4-cli",
        f"share/applications/{APP_ID}.desktop",
        f"share/mime/packages/{APP_ID}.xml",
        f"share/metainfo/{APP_ID}.metainfo.xml",
        f"share/icons/hicolor/scalable/apps/{APP_ID}.svg",
    }
    with tarfile.open(path, "r:gz") as archive:
        files = {}
        for member in archive.getmembers():
            name = member.name
            parts = Path(name).parts
            if not parts or parts[0] != folder or ".." in parts or name.startswith("/"):
                raise AuditError(f"unsafe or unrelated tar member: {name}")
            if member.issym() or member.islnk():
                raise AuditError(f"unexpected tar link: {name}")
            if member.isfile():
                relative = "/".join(parts[1:])
                if relative in files:
                    raise AuditError(f"duplicate tar member: {relative}")
                files[relative] = member
        missing = required - files.keys()
        if missing:
            raise AuditError(f"tarball missing required entries: {sorted(missing)}")
        verify_ffmpeg(set(files), FFMPEG_DIR, "tarball")
        verify_docs({name: member.size for name, member in files.items()}, "share/doc/cfa4", "tarball")
        for name in ("install.sh", "bin/cfa4", "bin/cfa4-cli"):
            if not files[name].mode & 0o111 or files[name].size == 0:
                raise AuditError(f"tarball executable invalid: {name}")
        def read(name: str) -> bytes:
            stream = archive.extractfile(files[name])
            if stream is None:
                raise AuditError(f"cannot read tar member: {name}")
            return stream.read()

        desktop = read(f"share/applications/{APP_ID}.desktop").decode("utf-8")
        if "Name=CFA4" not in desktop or f"Icon={APP_ID}" not in desktop:
            raise AuditError("tar desktop entry lacks CFA4 identity")
        xml = ET.fromstring(read(f"share/metainfo/{APP_ID}.metainfo.xml"))
        if xml.findtext("id") != APP_ID:
            raise AuditError("tar AppStream ID mismatch")
        release = xml.find("./releases/release")
        if release is None or release.get("version") != version:
            raise AuditError("tar AppStream version mismatch")


def verify_deb(path: Path, version: str) -> None:
    with path.open("rb") as source:
        magic = source.read(8)
    if magic != b"!<arch>\n":
        raise AuditError("invalid .deb ar magic")
    def dpkg(*args: str) -> str:
        try:
            return subprocess.check_output(
                ["dpkg-deb", *args], text=True, stderr=subprocess.STDOUT, timeout=30
            ).strip()
        except (OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired) as exc:
            raise AuditError(f"dpkg-deb validation unavailable or failed: {exc}") from exc

    for field, expected in (("Package", "cfa4"), ("Architecture", "amd64")):
        actual = dpkg("--field", str(path), field)
        if actual != expected:
            raise AuditError(f".deb {field}: expected {expected}, got {actual}")
    package_version = dpkg("--field", str(path), "Version")
    if package_version not in (version, f"{version}-1"):
        raise AuditError(f".deb version mismatch: {package_version} != {version}")
    files = dpkg("--contents", str(path))
    sizes = parse_listing(files, "./")
    for entry in (
        "usr/bin/cfa4", "usr/bin/cfa4-cli", "usr/bin/cfa4-qt",
        "usr/lib/cfa4/python/cfa4_qt/__main__.py",
        f"usr/share/applications/{APP_ID}.desktop",
        f"usr/share/mime/packages/{APP_ID}.xml",
        f"usr/share/metainfo/{APP_ID}.metainfo.xml",
        f"usr/share/icons/hicolor/scalable/apps/{APP_ID}.svg",
    ):
        if sizes.get(entry, 0) <= 0:
            raise AuditError(f".deb missing ./{entry}")
    names = set(sizes)
    verify_ffmpeg(names, FFMPEG_DEB_DIR, ".deb")
    verify_docs(sizes, "usr/share/doc/cfa4", ".deb")
    # the Ubuntu archive provides PySide6 to the .deb; it never carries its own
    if any(name.startswith(PYSIDE6_DIR + "/") for name in names):
        raise AuditError(".deb bundles PySide6; it depends on python3-pyside6.* instead")
    if any(name.startswith(f"usr/{FFMPEG_DIR}") for name in names):
        raise AuditError(".deb carries the relocatable FFmpeg copy; it belongs in /opt/cfa4")
    depends = dpkg("--field", str(path), "Depends")
    if "libgstreamer1.0-0" not in depends:
        raise AuditError(f".deb Depends lacks the system libraries: {depends}")


def appimage_offset(path: Path) -> int:
    """Where the squashfs starts: right after the runtime ELF's section header table."""
    with path.open("rb") as file:
        elf = file.read(64)
        if len(elf) < 64 or elf[:5] != b"\x7fELF\x02" or elf[5] != 1:
            raise AuditError("AppImage is not a 64-bit little-endian ELF")
        if int.from_bytes(elf[18:20], "little") != 62:
            raise AuditError("AppImage ELF machine is not x86_64")
        shoff = int.from_bytes(elf[40:48], "little")
        shentsize = int.from_bytes(elf[58:60], "little")
        shnum = int.from_bytes(elf[60:62], "little")
        offset = shoff + shentsize * shnum
        file.seek(offset)
        if file.read(4) != b"hsqs":
            raise AuditError("AppImage carries no squashfs after its runtime")
    return offset


def verify_appimage(path: Path) -> None:
    offset = appimage_offset(path)
    try:
        listing = subprocess.check_output(
            ["unsquashfs", "-o", str(offset), "-lls", str(path)],
            text=True, stderr=subprocess.STDOUT, timeout=120,
        )
    except (OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired) as exc:
        raise AuditError(f"unsquashfs listing of the AppImage unavailable or failed: {exc}") from exc
    sizes = parse_listing(listing, "squashfs-root/")
    for entry in (
        "AppRun", f"{APP_ID}.desktop", f"{APP_ID}.png", ".DirIcon",
        "usr/bin/cfa4", "usr/bin/cfa4-cli", "usr/bin/cfa4-qt",
        "usr/lib/cfa4/python/cfa4_qt/__main__.py",
        f"{PYSIDE6_DIR}/PySide6/__init__.py",
        f"usr/share/metainfo/{APP_ID}.metainfo.xml",
    ):
        if sizes.get(entry, 0) <= 0:
            raise AuditError(f"AppImage missing {entry}")
    verify_ffmpeg(set(sizes), f"usr/{FFMPEG_DIR}", "AppImage")
    verify_docs(sizes, "usr/share/doc/cfa4", "AppImage")
    missing = [doc for doc in PYSIDE6_DOCS if sizes.get(f"usr/share/doc/cfa4/pyside6/{doc}", 0) <= 0]
    if missing:
        raise AuditError(f"AppImage lacks the bundled PySide6 / Qt licences: {missing}")


def verify_package_set(dist: Path, version: str) -> dict:
    names = expected_names(version)
    hashes = verify_checksums(dist, names)
    prefix = f"cfa4-{version}-linux-x86_64"
    verify_deb(dist / f"{prefix}.deb", version)
    verify_appimage(dist / f"{prefix}.AppImage")
    verify_tarball(dist / f"{prefix}.tar.gz", version)
    # Flatpak bundle internal metadata and launch are checked during F5 clean installation.
    return {
        "application": "CFA4",
        "version": version,
        "target": "Ubuntu 26.04 AMD64",
        "status": "PASS_PACKAGE_STRUCTURE_NOT_RUNTIME",
        "files": [{"name": name, "bytes": (dist / name).stat().st_size, "sha256": hashes[name]}
                  for name in names],
        "limitations": ["Flatpak ref/runtime not inspected",
                        "AppImage contents listed, its runtime not executed",
                        "no clean-install, distribution or license approval"],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--version")
    args = parser.parse_args()
    try:
        version = repo_version()
        if args.version and args.version != version:
            raise AuditError(f"requested version {args.version} differs from Cargo {version}")
        report = verify_package_set(args.dist.resolve(), version)
    except (AuditError, OSError, tarfile.TarError, ET.ParseError, KeyError) as exc:
        print(f"FAIL CFA4 F4: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
