#!/usr/bin/env python3
"""Validate CFA4 local candidate artifacts without publishing or installing packages.

This is a package-structure/checksum gate, not a clean-install or license gate.
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
    for entry in (
        "./usr/bin/cfa4", "./usr/bin/cfa4-cli",
        f"./usr/share/applications/{APP_ID}.desktop",
        f"./usr/share/mime/packages/{APP_ID}.xml",
        f"./usr/share/metainfo/{APP_ID}.metainfo.xml",
        f"./usr/share/icons/hicolor/scalable/apps/{APP_ID}.svg",
    ):
        if entry not in files:
            raise AuditError(f".deb missing {entry}")


def verify_appimage(path: Path) -> None:
    with path.open("rb") as file:
        elf = file.read(20)
    if len(elf) < 20 or elf[:5] != b"\x7fELF\x02" or elf[5] != 1:
        raise AuditError("AppImage is not a 64-bit little-endian ELF")
    if int.from_bytes(elf[18:20], "little") != 62:
        raise AuditError("AppImage ELF machine is not x86_64")


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
        "limitations": ["Flatpak ref/runtime not inspected", "AppImage runtime not executed",
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
