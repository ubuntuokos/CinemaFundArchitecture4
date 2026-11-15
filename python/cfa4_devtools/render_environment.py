"""CFA4-RENDER-INTEGRATION-PLAN-001 F0a: read-only snapshot of the render toolchain and host.

Records what the plan's A10 rule (reproducibility) needs next to every benchmark and release test:
source revision and lockfile hashes, Rust toolchain and MSRV, FFmpeg (CFA4's own 9.0.2 build and
the system one), Qt / PySide6, kernel, distribution, CPU, NVIDIA driver, Vulkan, display server
and the system Python. It only reads files and runs ``--version``-style queries: no root, no
installs, no GPU work, no network. A missing tool is reported as ``null``, never as an error::

    python3 python/cfa4_devtools/render_environment.py            # JSON to stdout
    python3 python/cfa4_devtools/render_environment.py -o env.json

Run it on the reference machine (docs/development/reference-machine.md) to fill the machine
fields of docs/audit/RENDER-F0a-ENVIRONMENT-*.md.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import tomllib
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
LOCKFILES = ("Cargo.lock", "requirements/linux-amd64.txt", "requirements/otio-oracle.txt")
CFA4_FFMPEG = Path("/opt/cfa4/ffmpeg-9.0.2")


def run(cmd: list[str], timeout: float = 20.0) -> str | None:
    if shutil.which(cmd[0]) is None and not Path(cmd[0]).is_file():
        return None
    try:
        done = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, check=False)
    except (OSError, subprocess.SubprocessError):
        return None
    text = (done.stdout or done.stderr).strip()
    return text if done.returncode == 0 and text else None


def first_line(text: str | None) -> str | None:
    return text.splitlines()[0].strip() if text else None


def sha256(path: Path) -> str | None:
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None


def os_release() -> dict[str, str]:
    path = Path("/etc/os-release")
    if not path.is_file():
        return {}
    pairs = (line.split("=", 1) for line in path.read_text(encoding="utf-8").splitlines() if "=" in line)
    return {k: v.strip('"') for k, v in pairs if k in {"NAME", "VERSION_ID", "VERSION_CODENAME", "PRETTY_NAME"}}


def toolchain(root: Path) -> dict[str, Any]:
    pinned = tomllib.loads((root / "rust-toolchain.toml").read_text(encoding="utf-8")).get("toolchain", {})
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    pkg = workspace.get("workspace", {}).get("package", {})
    return {
        "rust_toolchain_toml": pinned.get("channel"),
        "msrv": pkg.get("rust-version"),
        "edition": pkg.get("edition"),
        "rustc": first_line(run(["rustc", "--version"])),
        "cargo": first_line(run(["cargo", "--version"])),
    }


def ffmpeg() -> dict[str, Any]:
    buildinfo = CFA4_FFMPEG / "share/doc/ffmpeg/BUILDINFO"
    own = CFA4_FFMPEG / "bin/ffmpeg"
    return {
        "cfa4_build_prefix": str(CFA4_FFMPEG) if CFA4_FFMPEG.is_dir() else None,
        "cfa4_build_version": first_line(run([str(own), "-version"])) if own.is_file() else None,
        "cfa4_buildinfo": buildinfo.read_text(encoding="utf-8") if buildinfo.is_file() else None,
        "system_version": first_line(run(["ffmpeg", "-version"])),
    }


def qt(root: Path) -> dict[str, Any]:
    venv_py = root / ".venv/bin/python"
    pyside = None
    if venv_py.is_file():
        pyside = first_line(
            run([str(venv_py), "-c", "import PySide6, PySide6.QtCore as c; print(PySide6.__version__, c.qVersion())"])
        )
    return {
        "qt6_system": first_line(run(["qtpaths6", "--qt-version"]))
        or first_line(run(["pkg-config", "--modversion", "Qt6Core"])),
        "pyside6_venv": pyside,
    }


def gpu() -> dict[str, Any]:
    smi = run(["nvidia-smi", "--query-gpu=name,driver_version,memory.total", "--format=csv,noheader"])
    cuda = run(["nvidia-smi"])
    cuda_version = None
    if cuda:
        m = re.search(r"CUDA Version:\s*([0-9.]+)", cuda)
        cuda_version = m.group(1) if m else None
    vulkan = run(["vulkaninfo", "--summary"])
    vk_api = sorted(set(re.findall(r"apiVersion\s*=\s*([0-9.]+)", vulkan or "")))
    vk_dev = sorted(set(re.findall(r"deviceName\s*=\s*(.+)", vulkan or "")))
    return {
        "nvidia_gpus": smi.splitlines() if smi else None,
        "cuda_driver_api": cuda_version,
        "vulkan_api_versions": vk_api or None,
        "vulkan_devices": [d.strip() for d in vk_dev] or None,
        "vaapi": first_line(run(["vainfo", "--display", "drm"])),
    }


def host() -> dict[str, Any]:
    lscpu = run(["lscpu"]) or ""
    model = re.search(r"Model name:\s*(.+)", lscpu)
    return {
        "os_release": os_release(),
        "kernel": platform.release(),
        "machine": platform.machine(),
        "cpu_model": model.group(1).strip() if model else None,
        "cpus_online": os.cpu_count(),
        "display_server": os.environ.get("XDG_SESSION_TYPE")
        or ("wayland" if os.environ.get("WAYLAND_DISPLAY") else None),
        "desktop": os.environ.get("XDG_CURRENT_DESKTOP"),
        "system_python": first_line(run(["/usr/bin/python3", "--version"])),
    }


def snapshot(root: Path) -> dict[str, Any]:
    return {
        "schema": "cfa4.render-environment/1",
        "source": {
            "head": first_line(run(["git", "-C", str(root), "rev-parse", "HEAD"])),
            "branch": first_line(run(["git", "-C", str(root), "rev-parse", "--abbrev-ref", "HEAD"])),
            "lockfiles_sha256": {name: sha256(root / name) for name in LOCKFILES},
        },
        "toolchain": toolchain(root),
        "ffmpeg": ffmpeg(),
        "qt": qt(root),
        "gpu": gpu(),
        "host": host(),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else None)
    parser.add_argument("--root", type=Path, default=ROOT, help="repository root")
    parser.add_argument("-o", "--output", type=Path, help="write the JSON here instead of stdout")
    args = parser.parse_args(argv)
    text = json.dumps(snapshot(args.root), indent=1, ensure_ascii=False, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    else:
        print(text, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
