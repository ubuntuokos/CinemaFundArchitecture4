"""Desktop entry for running the Qt GUI from a source checkout (Linux).

KDE and GNOME show a window's name and icon from the ``.desktop`` file that matches its app id
(``io.github.ubuntuokos.cfa4``). Packages install one; a checkout run has none, so the panel
shows the interpreter ("python3") and a generic icon. ``cfa4-qt --install-desktop-entry`` (or
``CFA4_DEV_DESKTOP_ENTRY=1``, the same flag as the egui launcher's) writes
a user-level entry and the CFA4 icons into ``$XDG_DATA_HOME`` (``~/.local/share``).

Same rules as the egui launcher (``apps/cfa4/src/dev_icon.rs``):

- opt-in only: nothing is written into the user's profile unless asked;
- a packaged entry in a system data dir is never overridden;
- our own user-level copy is rewritten when it starts another program (another checkout);
- AppImage and Flatpak runs never write (they ship their own entry);
- failures are reported and skipped, never fatal.

The shell picks the entry up at the next start of the program.
"""

from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path

from .branding import ASSETS, ICON_NAME, ICON_SIZES, icon_file

ENV_FLAGS = ("CFA4_DEV_DESKTOP_ENTRY",)


def requested_by_env() -> bool:
    return any(os.environ.get(name) == "1" for name in ENV_FLAGS)


def user_data_dir() -> Path:
    xdg = os.environ.get("XDG_DATA_HOME")
    if xdg and Path(xdg).is_absolute():
        return Path(xdg)
    return Path.home() / ".local" / "share"


def system_data_dirs() -> list[Path]:
    raw = os.environ.get("XDG_DATA_DIRS") or "/usr/local/share:/usr/share"
    return [Path(d) for d in raw.split(":") if d and Path(d).is_absolute()]


def launch_command() -> list[str]:
    """How the shell starts this GUI: the installed ``cfa4-qt`` script next to this Python
    (a venv's bin/), else ``python -m cfa4_qt`` with this interpreter."""
    script = Path(sys.executable).with_name("cfa4-qt")
    if script.is_file() and os.access(script, os.X_OK):
        return [str(script)]
    found = shutil.which("cfa4-qt")
    if found:
        return [found]
    return [sys.executable, "-m", "cfa4_qt"]


def exec_line(argv: list[str]) -> str:
    """The ``Exec=`` value. Desktop entries forbid '%' outside field codes and newlines; such a
    path is refused rather than written broken."""
    for part in argv:
        if any(c in part for c in "%=\n\r"):
            raise ValueError(f"path cannot be used in a desktop entry: {part!r}")
    return " ".join(escape_exec_argument(a) for a in argv)


def escape_exec_argument(arg: str) -> str:
    """One quoted ``Exec=`` argument per the Desktop Entry spec: inside double quotes, `` ` ``,
    ``"``, ``$`` and ``\\`` get a backslash, then every backslash is doubled again because the
    key file's string decoding runs first (as ``escape_exec_argument`` in apps/cfa4/src/dev_icon.rs)."""
    quoted = "".join("\\" + c if c in '`"$\\' else c for c in arg)
    return '"' + quoted.replace("\\", "\\\\") + '"'


def entry_text(exec_value: str) -> str:
    return (
        "[Desktop Entry]\n"
        "Type=Application\n"
        "Name=CFA4 (development)\n"
        "GenericName=Video Editor\n"
        "GenericName[hu]=Videószerkesztő\n"
        "Comment=CFA4 Qt 6 editor run from a source checkout\n"
        f"Exec={exec_value} %f\n"  # one project per launch: cfa4-qt opens one
        f"Icon={ICON_NAME}\n"
        "Terminal=false\n"
        "Categories=AudioVideo;Video;AudioVideoEditing;\n"
        "X-CFA4-Dev-Entry=true\n"
    )


def install(out=sys.stdout) -> bool:
    """Install the user-level entry and icons. Returns whether the shell now has an entry."""
    if os.environ.get("APPIMAGE") or os.environ.get("FLATPAK_ID"):
        print("cfa4-qt: packaged run (AppImage/Flatpak): its own desktop entry is used", file=out)
        return False
    name = f"{ICON_NAME}.desktop"
    user = user_data_dir()
    for directory in system_data_dirs():
        if (directory / "applications" / name).is_file():
            print(f"cfa4-qt: the installed package's entry is used: {directory / 'applications' / name}", file=out)
            return True
    try:
        exec_value = exec_line(launch_command())
        target = user / "applications" / name
        text = entry_text(exec_value)
        if target.is_file() and target.read_text(encoding="utf-8", errors="replace") == text:
            print(f"cfa4-qt: desktop entry is up to date: {target}", file=out)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            tmp = target.with_suffix(".desktop.tmp")
            tmp.write_text(text, encoding="utf-8")
            tmp.replace(target)
            print(f"cfa4-qt: desktop entry written: {target}", file=out)
        for size in ICON_SIZES:
            src = icon_file(size)
            if src.is_file():
                dest = user / "icons" / "hicolor" / f"{size}x{size}" / "apps" / src.name
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(src, dest)
        svg = ASSETS / "hicolor" / "scalable" / "apps" / f"{ICON_NAME}.svg"
        if svg.is_file():
            dest = user / "icons" / "hicolor" / "scalable" / "apps" / svg.name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(svg, dest)
    except (OSError, ValueError) as exc:
        print(f"cfa4-qt: desktop entry not installed: {exc}", file=out)
        return False
    print("cfa4-qt: the taskbar shows CFA4 and its logo from the next start", file=out)
    return True
