#!/usr/bin/env python3
"""CFA4 F5: non-invasive staged runtime audit on Ubuntu 26.04 AMD64.

Does NOT certify actual system .deb install, Flatpak sandbox launch, GPU/GUI,
AppImage AppRun, legal redistribution, or complete dlopen dependency inventory.
All extracted files, trial prefix installs and imported bundle data are temporary.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

from verify_cfa4_packages import APP_ID, AuditError, repo_version, verify_package_set

MAX_OUTPUT = 64000


def invoke(args: list[str], *, cwd: Path | None = None, env: dict | None = None,
           timeout: int = 90) -> str:
    try:
        result = subprocess.run(
            args, cwd=cwd, env=env, capture_output=True, text=True,
            timeout=timeout, check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise AuditError(f"unable to run {args[0]}: {exc}") from exc
    if result.returncode:
        raise AuditError(
            f"{args[0]} exited {result.returncode}: "
            f"{(result.stderr + result.stdout)[-1500:]}"
        )
    return (result.stdout + result.stderr)[:MAX_OUTPUT]


def isolated_env(root: Path) -> dict[str, str]:
    home = root / "home"
    for name in ("config", "cache", "data"):
        (home / f".{name}").mkdir(parents=True, exist_ok=True)
    return {
        **os.environ,
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / ".config"),
        "XDG_CACHE_HOME": str(home / ".cache"),
        "XDG_DATA_HOME": str(home / ".data"),
        "XDG_STATE_HOME": str(home / ".state"),
    }


def library_inventory(exe: Path, *, env: dict[str, str]) -> dict:
    if not exe.is_file():
        raise AuditError(f"missing executable: {exe}")
    result = invoke(["ldd", str(exe)], env=env)
    missing = re.findall(r"^\s*(\S+)\s*=>\s*not found\b", result, flags=re.M)
    if missing:
        raise AuditError(f"missing ELF runtime dependencies for {exe.name}: {missing}")
    return {"executable": exe.name, "missing": [], "ldd": result.strip()}


def cli_smoke(exe: Path, version: str, env: dict[str, str]) -> dict:
    if not exe.is_file() or not os.access(exe, os.X_OK):
        raise AuditError(f"missing executable CFA4 CLI: {exe}")
    output = invoke([str(exe), "--version"], env=env, timeout=20).strip()
    if version not in output:
        raise AuditError(f"CFA4 CLI output lacks expected version {version}: {output!r}")
    return {"cli_version": output, "version_match": True}


def stage_tarball(archive_path: Path, target: Path, version: str,
                  env: dict[str, str]) -> dict:
    stage = target / "tar-source"
    stage.mkdir()
    # Already inspected by verify_package_set; tarfile data filter also rejects
    # unexpected links, traversal and device nodes.
    with tarfile.open(archive_path, "r:gz") as archive:
        archive.extractall(stage, filter="data")
    unpacked = stage / f"cfa4-{version}-linux-x86_64"
    installed = target / "tar-prefix"
    invoke(["bash", str(unpacked / "install.sh"), "--prefix", str(installed)], env=env)
    if not (installed / "share/applications" / f"{APP_ID}.desktop").is_file():
        raise AuditError("tar prefix installer omitted CFA4 desktop entry")
    if not (installed / "share/metainfo" / f"{APP_ID}.metainfo.xml").is_file():
        raise AuditError("tar prefix installer omitted CFA4 AppStream data")
    binaries = [installed / "bin/cfa4", installed / "bin/cfa4-cli"]
    return {
        "scope": "PREFIX_INSTALL_AND_CLI_SMOKE_NOT_GUI",
        **cli_smoke(binaries[1], version, env),
        "libraries": [library_inventory(path, env=env) for path in binaries],
        "cleanup": "isolated temporary prefix deleted by TemporaryDirectory",
    }


def stage_deb(package: Path, target: Path, version: str,
              env: dict[str, str]) -> dict:
    stage = target / "deb-unpacked"
    stage.mkdir()
    invoke(["dpkg-deb", "--extract", str(package), str(stage)], env=env)
    binaries = [stage / "usr/bin/cfa4", stage / "usr/bin/cfa4-cli"]
    return {
        "scope": "DEB_UNPACK_AND_CLI_SMOKE_NOT_DPKG_INSTALL",
        **cli_smoke(binaries[1], version, env),
        "libraries": [library_inventory(path, env=env) for path in binaries],
        "cleanup": "temporary extracted FHS tree removed",
    }


def stage_appimage(package: Path, target: Path, version: str,
                   env: dict[str, str]) -> dict:
    work = target / "appimage-unpacked"
    work.mkdir()
    invoke([str(package), "--appimage-extract"], cwd=work, env=env, timeout=120)
    appdir = work / "squashfs-root"
    if not (appdir / "AppRun").exists():
        raise AuditError("AppImage extract has no AppRun")
    binaries = [appdir / "usr/bin/cfa4", appdir / "usr/bin/cfa4-cli"]
    return {
        "scope": "APPIMAGE_EXTRACT_AND_CLI_SMOKE_NOT_APPRUN",
        **cli_smoke(binaries[1], version, env),
        "libraries": [library_inventory(path, env=env) for path in binaries],
        "cleanup": "temporary AppDir extracted files removed",
    }


def stage_flatpak(package: Path, target: Path, env: dict[str, str]) -> dict:
    # Import to a private temporary OSTree repo, not ~/.local/share/flatpak or
    # the system installation. Neither app nor runtime is installed.
    repo = target / "private-flatpak-repo"
    invoke(["ostree", "init", f"--repo={repo}", "--mode=archive-z2"], env=env)
    invoke(["flatpak", "build-import-bundle", str(repo), str(package)], env=env,
           timeout=120)
    refs = invoke(["ostree", f"--repo={repo}", "refs"], env=env).splitlines()
    prefix = f"app/{APP_ID}/x86_64/"
    found = sorted(ref for ref in refs if ref.startswith(prefix))
    if len(found) != 1:
        raise AuditError(f"Flatpak bundle unexpected app refs: {refs}")
    return {
        "scope": "PRIVATE_BUNDLE_IMPORT_NOT_FLATPAK_INSTALL_OR_RUNTIME",
        "ref": found[0],
        "cleanup": "private temporary OSTree repo removed",
    }


def check_platform() -> None:
    if sys.platform != "linux" or os.uname().machine != "x86_64":
        raise AuditError("F5 requires Ubuntu 26.04 x86_64")
    data = {}
    for line in Path("/etc/os-release").read_text().splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            data[k] = v.strip('"')
    if data.get("VERSION_ID") != "26.04" or (
        data.get("ID") != "ubuntu" and "ubuntu" not in data.get("ID_LIKE", "").split()
    ):
        raise AuditError("F5 requires Ubuntu 26.04 or an Ubuntu 26.04 flavour")


def audit(dist: Path, version: str) -> dict:
    check_platform()
    # Verify integrity of all four artifacts before executing any CFA4 binary.
    structure = verify_package_set(dist, version)
    basename = f"cfa4-{version}-linux-x86_64"
    with tempfile.TemporaryDirectory(prefix="cfa4-f5-staging-") as tmp:
        root = Path(tmp)
        env = isolated_env(root)
        result = {
            "schema": "CFA4_F5_STAGING_V1",
            "build_version": version,
            "structure": structure["status"],
            "tarball": stage_tarball(dist / f"{basename}.tar.gz", root, version, env),
            "deb": stage_deb(dist / f"{basename}.deb", root, version, env),
            "appimage": stage_appimage(dist / f"{basename}.AppImage", root, version, env),
            "flatpak": stage_flatpak(dist / f"{basename}.flatpak", root, env),
            "status": "PASS_STAGING_ONLY",
            "not_proven": [
                "clean VM .deb install and removal",
                "Flatpak runtime install/sandbox launch/removal",
                "AppImage AppRun GUI launch",
                "KDE/GNOME/Cinnamon desktop integration",
                "GPU, dynamic-loaded libraries, licensing and distribution",
            ],
        }
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--report", type=Path, help="Optional JSON audit report path")
    args = parser.parse_args()
    try:
        result = audit(args.dist.resolve(), repo_version())
    except (AuditError, OSError, subprocess.CalledProcessError, tarfile.TarError) as exc:
        print(f"FAIL CFA4 F5 staged validation: {exc}", file=sys.stderr)
        return 1
    payload = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(payload, encoding="utf-8")
    print(payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
