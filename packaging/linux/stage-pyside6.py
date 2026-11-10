#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Stage PySide6 / Qt 6 for the relocatable CFA4 packages that cannot depend on Ubuntu's
``python3-pyside6.*`` (the AppImage and the Flatpak; docs/qt-parity.md 1.10). Called by
package.sh and flatpak-bundle.sh; it never downloads, builds or installs anything.

    stage-pyside6.py --wheels DIR --dest DIR [--doc DIR] [--requirements FILE]

--wheels DIR        a local wheelhouse with the pinned manylinux x86_64 wheels of
                    pyside6-essentials, pyside6-addons and shiboken6 (CFA4_PYSIDE6_WHEELS); each
                    must match a ``--hash`` of requirements/linux-amd64.txt, or nothing is staged
--dest DIR          the directory put on the GUI's ``sys.path`` (``lib/cfa4/pyside6``): it gets
                    ``PySide6/`` and ``shiboken6/``
--doc DIR           where the licence texts and the source offer go (default: DEST)
--requirements FILE the hash-pinned requirements (default: requirements/linux-amd64.txt)
--host-xcb          also copy the X11 helper libraries Qt's xcb plugin loads and an Ubuntu
                    desktop may lack (xcb-util-cursor, -image, -keysyms, -renderutil, -wm,
                    xkbcommon-x11) from the build host, with their copyright files (the AppImage;
                    the Flatpak runtime has them)

Only what the Qt GUI loads is kept: the Python modules it imports (QtCore, QtGui, QtWidgets,
QtNetwork, QtTest, QtMultimedia and QtDBus), the platform, input-method, theme, image-format,
Wayland, TLS and multimedia plugins, Qt's own translations, and the Qt libraries those need
(followed through DT_NEEDED); no QML, WebEngine, Designer, tools or headers. Libraries every
Ubuntu 26.04 desktop and the Flatpak runtime provide (glibc, libstdc++, X11 / Wayland / GL,
fontconfig, GLib, PulseAudio) stay on the host, as Qt's wheels expect.
"""

from __future__ import annotations

import argparse
import hashlib
import re
import shutil
import struct
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
VERSION = "6.12.0"
WHEELS = ("pyside6-essentials", "pyside6-addons", "shiboken6")
# the PySide6 modules cfa4_qt imports (grep "PySide6.Qt" python/cfa4_qt), and QtDBus, which the
# platform theme and input contexts load
MODULES = ("QtCore", "QtGui", "QtWidgets", "QtNetwork", "QtTest", "QtMultimedia", "QtDBus")
PLUGIN_DIRS = (
    "platforms",
    "platformthemes",
    "platforminputcontexts",  # ibus / compose: Japanese and other input methods
    "imageformats",
    "iconengines",
    "xcbglintegrations",
    "wayland-decoration-client",
    "wayland-graphics-integration-client",
    "wayland-shell-integration",
    "tls",
    "networkinformation",
    "multimedia",
)
# what Qt's xcb plugin needs beyond libxcb itself, which a desktop install may lack (Qt 6.5+ will not
# start on X11 without libxcb-cursor0); SONAME -> Ubuntu package
HOST_XCB = {
    "libxcb-cursor.so.0": "libxcb-cursor0",
    "libxcb-icccm.so.4": "libxcb-icccm4",
    "libxcb-image.so.0": "libxcb-image0",
    "libxcb-keysyms.so.1": "libxcb-keysyms1",
    "libxcb-render-util.so.0": "libxcb-render-util0",
    "libxcb-util.so.1": "libxcb-util1",
    "libxkbcommon-x11.so.0": "libxkbcommon-x11-0",
}
# desktop sessions only: X11 and Wayland, plus offscreen for headless runs (no EGLFS / framebuffer)
PLATFORMS = ("libqxcb.so", "libqwayland.so", "libqwayland-egl.so", "libqwayland-generic.so", "libqoffscreen.so", "libqminimal.so")
PYTHON_FILES = ("__init__.py", "_config.py", "_git_pyside_version.py", "_git_shiboken_module_version.py", "py.typed")
LIBRARY_SOURCES = (
    "https://download.qt.io/official_releases/QtForPython/pyside6/ (PySide6 / Shiboken6 sources)",
    "https://download.qt.io/official_releases/qt/6.12/ (Qt 6.12 sources)",
    "https://code.qt.io/ (the Qt and Qt for Python repositories)",
)


def fail(message: str) -> None:
    sys.stderr.write(f"error: {message}\n")
    sys.exit(1)


def pinned_hashes(requirements: Path) -> dict[str, set[str]]:
    """``name -> {sha256, …}`` for the wheels CFA4 stages, at VERSION, from a ``--require-hashes``
    requirements file."""
    text = requirements.read_text().replace("\\\n", " ")
    out: dict[str, set[str]] = {}
    for line in text.splitlines():
        m = re.match(r"\s*([A-Za-z0-9_.-]+)==([^\s;]+)(.*)", line)
        if m is None:
            continue
        name = m.group(1).lower().replace("_", "-")
        if name in WHEELS:
            if m.group(2) != VERSION:
                fail(f"{requirements} pins {name}=={m.group(2)}, stage-pyside6.py stages {VERSION}")
            out[name] = set(re.findall(r"--hash=sha256:([0-9a-f]{64})", m.group(3)))
    missing = [w for w in WHEELS if not out.get(w)]
    if missing:
        fail(f"{requirements} has no hash-pinned {', '.join(missing)}")
    return out


def find_wheel(wheels: Path, name: str) -> Path:
    stem = name.replace("-", "_")
    found = sorted(wheels.glob(f"{stem}-{VERSION}-*manylinux*x86_64.whl"))
    if not found:
        fail(f"no {stem}-{VERSION}-…-manylinux…x86_64.whl in {wheels} (CFA4_PYSIDE6_WHEELS)")
    return found[-1]


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def wanted(member: str) -> bool:
    """The wheel members the GUI can load (the libraries are pruned to the needed ones later)."""
    parts = member.split("/")
    if parts[0] not in ("PySide6", "shiboken6") or member.endswith("/"):
        return False
    if len(parts) == 2:
        name = parts[1]
        if name in PYTHON_FILES or name.startswith(("libpyside6.abi3.so", "libshiboken6.abi3.so")):
            return True
        if name in ("Shiboken.abi3.so",):
            return True
        return name.endswith(".abi3.so") and name[: -len(".abi3.so")] in MODULES
    if parts[1] == "support":  # PySide6's signature loader (imported by every module)
        return not member.endswith(".pyi")
    if parts[1] != "Qt":
        return False
    if parts[2] == "lib":
        return len(parts) == 4
    if parts[2] == "plugins":
        if len(parts) < 5 or parts[3] not in PLUGIN_DIRS or "virtualkeyboard" in parts[4]:  # QML keyboard
            return False
        if parts[4] == "libqpdf.so":  # PDF as an image format: it alone would pull in Qt Pdf
            return False
        return parts[3] != "platforms" or parts[4] in PLATFORMS
    if parts[2] == "translations":  # Qt's own strings (standard dialogs, shortcuts)
        return len(parts) == 4 and re.fullmatch(r"qt(base|multimedia)_[a-z_A-Z]+\.qm", parts[3]) is not None
    return False


def needed(path: Path) -> list[str]:
    """DT_NEEDED of a 64-bit little-endian ELF shared object."""
    data = path.read_bytes()
    if data[:4] != b"\x7fELF" or data[4] != 2 or data[5] != 1:
        return []
    shoff, = struct.unpack_from("<Q", data, 0x28)
    shentsize, shnum = struct.unpack_from("<HH", data, 0x3A)
    sections = [struct.unpack_from("<IIQQQQIIQQ", data, shoff + i * shentsize) for i in range(shnum)]
    out = []
    for _name, kind, _flags, _addr, offset, size, link, _info, _align, entsize in sections:
        if kind != 6:  # SHT_DYNAMIC
            continue
        strtab = sections[link]
        for i in range(size // (entsize or 16)):
            tag, value = struct.unpack_from("<qQ", data, offset + i * 16)
            if tag == 0:
                break
            if tag == 1:  # DT_NEEDED
                start = strtab[4] + value
                out.append(data[start : data.index(b"\0", start)].decode())
    return out


def prune_libraries(site: Path) -> tuple[int, int]:
    """Keep only the Qt/lib files some kept module or plugin needs (directly or through another
    library); the wheels carry each library under every symlink name as a full copy."""
    lib = site / "PySide6" / "Qt" / "lib"
    roots = [p for p in site.rglob("*.so*") if p.parent != lib and p.is_file()]
    keep: set[str] = set()
    todo = list(roots)
    while todo:
        for name in needed(todo.pop()):
            if name not in keep and (lib / name).is_file():
                keep.add(name)
                todo.append(lib / name)
    removed = 0
    for p in lib.iterdir():
        if p.name not in keep:
            p.unlink()
            removed += 1
    return len(keep), removed


def copy_host_xcb(lib: Path, doc: Path) -> None:
    """The HOST_XCB libraries from the build host's linker cache into Qt/lib (Qt's libraries find
    them there through their ``$ORIGIN`` RUNPATH), with each package's copyright file."""
    cache = subprocess.run(["ldconfig", "-p"], capture_output=True, text=True, check=True).stdout
    found = dict(re.findall(r"^\s*(\S+) \(libc6,x86-64\) => (\S+)$", cache, re.MULTILINE))
    missing = sorted(pkg for name, pkg in HOST_XCB.items() if name not in found)
    if missing:
        fail(f"the AppImage bundles X11 helper libraries from this host: sudo apt install {' '.join(missing)}")
    for name, package in HOST_XCB.items():
        shutil.copyfile(Path(found[name]).resolve(), lib / name)
        (lib / name).chmod(0o644)
        copyright = Path("/usr/share/doc") / package / "copyright"
        if not copyright.is_file():
            fail(f"{copyright} is missing: the bundled {name} needs its licence")
        target = doc / "x11-libraries" / f"{package}.copyright"
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(copyright, target)


def write_notice(doc: Path, wheels: dict[str, Path]) -> None:
    doc.mkdir(parents=True, exist_ok=True)
    for text, name in (("LGPL-3", "LICENSE.LGPL-3.0"), ("LGPL-2.1", "LICENSE.LGPL-2.1")):
        src = Path("/usr/share/common-licenses") / text
        if not src.is_file():
            fail(f"{src} is missing (Ubuntu's base-files): the bundled Qt needs its licence text")
        shutil.copyfile(src, doc / name)
    lines = [
        "PySide6 / Qt for Python and Qt 6 in this package",
        "",
        f"The CFA4 Qt GUI (cfa4-qt) runs on PySide6 {VERSION} and the Qt {VERSION} libraries from the",
        "official Qt for Python wheels, unmodified and dynamically loaded:",
        "",
        *(f"  {w.name}  sha256:{sha256(w)}" for w in wheels.values()),
        "",
        "Licence: LGPL-3.0-only (LICENSE.LGPL-3.0 here, with the GPL-3.0 in LICENSE beside it).",
        "Qt Multimedia's FFmpeg libraries (libavcodec, libavformat, libavutil, libswresample,",
        "libswscale) as built by The Qt Company: LGPL-2.1-or-later (LICENSE.LGPL-2.1).",
        "Copyright (C) The Qt Company Ltd. and other contributors; FFmpeg: the FFmpeg developers.",
        "",
        "You may replace these libraries with your own build of the same versions: they sit,",
        "unchanged, in lib/cfa4/pyside6 (PySide6/Qt/lib and PySide6/Qt/plugins). The",
        "corresponding source code is available from:",
        *(f"  {s}" for s in LIBRARY_SOURCES),
        "and on request from the CFA4 maintainers for three years after this release.",
        "",
    ]
    (doc / "PYSIDE6-QT-NOTICE.txt").write_text("\n".join(lines))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--wheels", type=Path, required=True)
    ap.add_argument("--dest", type=Path, required=True)
    ap.add_argument("--doc", type=Path)
    ap.add_argument("--requirements", type=Path, default=ROOT / "requirements" / "linux-amd64.txt")
    ap.add_argument("--host-xcb", action="store_true")
    a = ap.parse_args()
    if not a.wheels.is_dir():
        fail(f"{a.wheels} is not a directory (CFA4_PYSIDE6_WHEELS: a local wheelhouse, never downloaded here)")
    pins = pinned_hashes(a.requirements)
    wheels = {name: find_wheel(a.wheels, name) for name in WHEELS}
    for name, path in wheels.items():
        digest = sha256(path)
        if digest not in pins[name]:
            fail(f"{path.name} has sha256 {digest}, not one pinned in {a.requirements}; refusing it")
    if a.dest.exists():
        shutil.rmtree(a.dest)
    a.dest.mkdir(parents=True)
    for path in wheels.values():
        with zipfile.ZipFile(path) as z:
            for info in z.infolist():
                if not wanted(info.filename):
                    continue
                target = a.dest / info.filename
                target.parent.mkdir(parents=True, exist_ok=True)
                with z.open(info) as src, target.open("wb") as out:
                    shutil.copyfileobj(src, out)
                mode = (info.external_attr >> 16) & 0o777
                target.chmod(mode or 0o644)
    for module in MODULES:
        if not (a.dest / "PySide6" / f"{module}.abi3.so").is_file():
            fail(f"PySide6.{module} is not in the wheels")
    if not (a.dest / "PySide6" / "Qt" / "plugins" / "platforms" / "libqxcb.so").is_file():
        fail("the wheels have no xcb platform plugin")
    kept, removed = prune_libraries(a.dest)
    write_notice(a.doc or a.dest, wheels)
    if a.host_xcb:
        copy_host_xcb(a.dest / "PySide6" / "Qt" / "lib", a.doc or a.dest)
    size = sum(p.stat().st_size for p in a.dest.rglob("*") if p.is_file())
    print(f"staged PySide6 {VERSION} in {a.dest}: {len(MODULES)} modules, {kept} Qt libraries ({removed} unused left out), {size / 1e6:.0f} MB")


if __name__ == "__main__":
    main()
