"""``python -m cfa4_qt`` / ``cfa4-qt``: start the CFA4 Qt 6 GUI."""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

# files that open in the Credits workspace rather than as a project (docs/credits.md; the
# packaged io.github.ubuntuokos.cfa4.credits.desktop hands them over)
CREDITS_SUFFIXES = (".cfa4credits", ".cfa4credtpl")


def credits_file(path: str | None) -> str | None:
    """`path` when it is a credits file or credits template, made absolute; else None."""
    if not path or not path.lower().endswith(CREDITS_SUFFIXES):
        return None
    return str(Path(path).expanduser().resolve())


def is_project(path: str) -> bool:
    """Whether a launch file is a CFA4 project (`.fcproj`, any case: the MIME glob starts the app
    for `CUT.FCPROJ` too), as egui's ``args::is_project``."""
    return Path(path).suffix.lower() == ".fcproj" and Path(path).stem != ""


def launch_files(files: list[str]) -> tuple[str | None, str | None, list[str]]:
    """The launch files as egui's ``args::project_and_media``: the project to open (the first
    `.fcproj`), the credits file to open in the Credits workspace (the first credits file or
    template) and the media files to import (every other file), all made absolute. Media paths
    keep their symlinks (``os.path.abspath``), so the paths saved in the project are the ones given."""
    project = next((str(Path(f).expanduser().resolve()) for f in files if is_project(f)), None)
    credits = next((c for c in map(credits_file, files) if c), None)
    media = [os.path.abspath(os.path.expanduser(f)) for f in files if f and not is_project(f) and not credits_file(f)]
    return project, credits, media


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="cfa4-qt", description="CFA4 Qt 6 editor (Rust engine + PySide6)")
    parser.add_argument(
        "files",
        nargs="*",
        metavar="FILE",
        help="a project file (.fcproj) to open, a credits file (.cfa4credits) or template (.cfa4credtpl) to open in "
        "the Credits workspace, and media files to import (as the egui editor: the first project opens, "
        "every other file is imported)",
    )
    parser.add_argument("--credits", metavar="FILE", help="open this credits file (.cfa4credits), template (.cfa4credtpl) or spreadsheet in the Credits workspace")
    parser.add_argument("--engine", help="path to cfa4-cli (default: PATH, then target/)")
    parser.add_argument("--demo", action="store_true", help="open the built-in demo project")
    parser.add_argument("--data-dir", help="CFA4 data directory (auto-save, recovery, presets)")
    parser.add_argument("--no-autosave", action="store_true", help="no auto-save or crash-recovery journal")
    parser.add_argument("--lang", help="UI language (en, hu, de, ...); default: the system locale")
    parser.add_argument(
        "--install-desktop-entry",
        action="store_true",
        help="install a desktop entry and the CFA4 icons for this checkout into ~/.local/share "
        "(so the taskbar shows CFA4 and its logo), then exit",
    )
    parser.add_argument(
        "--control",
        type=int,
        metavar="PORT",
        help="serve the JSON-lines control channel on 127.0.0.1:PORT for agents (docs/control-protocol.md; "
        "default: CFA4_CONTROL_PORT)",
    )
    parser.add_argument(
        "--control-input-only",
        action="store_true",
        help="with --control: only the control channel's input reaches the window (native keyboard, pointer and drops are ignored)",
    )
    parser.add_argument("--smoke", action="store_true", help="start, wait for the engine, then quit (CI check)")
    parser.add_argument("--screenshot", help="with --smoke: save a PNG of the window before quitting")
    args = parser.parse_args(argv)

    if not sys.platform.startswith("linux"):
        parser.error("CFA4 targets Linux (Ubuntu) only")
    if args.install_desktop_entry:
        from .desktop_entry import install

        return 0 if install() else 1
    # before anything loads Qt Multimedia: a PipeWire library without its client configuration
    # would crash the first audio device query
    from .audio_backend import apply as choose_audio_backend

    choose_audio_backend(log=lambda line: print(line, file=sys.stderr))
    try:
        from PySide6.QtCore import QSettings, QTimer
        from PySide6.QtWidgets import QApplication, QMessageBox
    except ImportError as exc:
        parser.error(f"Qt 6 / PySide6 missing: {exc}; install: python -m pip install -e '.[qt]'")

    from . import i18n
    from .desktop_entry import install, requested_by_env
    from .engine import EngineClient, EngineError, locate_engine
    from .main_window import MainWindow

    if requested_by_env():  # opt-in, as the egui launcher's CFA4_DEV_DESKTOP_ENTRY=1
        install(sys.stderr)

    app = QApplication(sys.argv[:1])
    app.setApplicationName("CFA4")
    app.setOrganizationName("CFA4")
    # the freedesktop app id: Wayland and the taskbar match the window to the .desktop file
    # (packaging/linux/io.github.ubuntuokos.cfa4.desktop), so the panel shows CFA4 and its logo
    app.setDesktopFileName("io.github.ubuntuokos.cfa4")
    from .branding import app_icon

    icon = app_icon()
    if not icon.isNull():
        app.setWindowIcon(icon)
    # exceptions that escape a slot or an event handler go to <data dir>/Logs/crash-<day>.log and
    # the window's error dialog; the session keeps running (egui's crash::install)
    from . import crash

    data_dir = Path(args.data_dir).expanduser() if args.data_dir else crash.default_data_dir()
    crash.install(data_dir / "Logs" if data_dir is not None else None)
    settings = QSettings("CFA4", "CFA4-Qt")
    language = args.lang or str(settings.value("language") or "") or i18n.system_language()
    i18n.TR.set_language(language)

    rejected: list[str] = []
    engine_path = locate_engine(args.engine, rejected)
    if engine_path is None:
        message = i18n.tr(
            "The CFA4 engine (cfa4-cli) was not found. Build it with: cargo build --release -p cfa4-cli"
        )
        if rejected:
            message += "\n\n" + i18n.tr("Checked:") + "\n" + "\n".join(rejected)
            message += "\n\n" + i18n.tr(
                "An engine without `serve` was built from a branch without the Qt GUI: switch to the Qt GUI branch and build again, or pass --engine PATH."
            )
        if args.smoke:
            print(message, file=sys.stderr)
            return 1
        QMessageBox.critical(None, "CFA4", message)
        return 1
    project, credits, media = launch_files(args.files)
    if not credits and args.credits:
        credits = str(Path(args.credits).expanduser().resolve())
    engine_args: list[str] = []
    if project:
        engine_args += ["--project", project]
    elif args.demo:
        engine_args.append("--demo")
    if args.data_dir:
        engine_args += ["--data-dir", args.data_dir]
    if not args.no_autosave and not args.smoke:
        engine_args.append("--autosave")
    engine = EngineClient(engine_path, engine_args)
    window = MainWindow(engine, settings)
    try:
        engine.start()
    except EngineError as exc:
        QMessageBox.critical(None, "CFA4", str(exc))
        return 1
    port = args.control
    if port is None and os.environ.get("CFA4_CONTROL_PORT", "").strip().isdigit():
        port = int(os.environ["CFA4_CONTROL_PORT"])
    control = None
    if port is not None:
        from PySide6.QtCore import Qt

        from .control import ControlServer

        if not 0 <= port <= 65535:
            parser.error("--control PORT must be 0 to 65535")
        control = ControlServer(window)
        if not control.listen(port):
            print(f"cfa4-qt: cannot listen on 127.0.0.1:{port}: {control.server.errorString()}", file=sys.stderr)
            return 1
        control.set_input_only(args.control_input_only)
        print(f"cfa4-qt: control channel on 127.0.0.1:{control.port()}", file=sys.stderr, flush=True)
        # driving the app never steals the user's keyboard (egui's): opened without activating it
        window.setAttribute(Qt.WidgetAttribute.WA_ShowWithoutActivating)
    window.show()
    if media:
        # once the engine runs (and has opened the project, if one was given): egui's launch
        # import, `file.import` with every media file; an error goes to the status bar and Events
        def import_media(_hello: dict, paths: list[str] = media) -> None:
            QTimer.singleShot(0, lambda: window.execute("file.import", {"paths": paths}))

        engine.started.connect(import_media)
    if credits:
        # once the engine has a sequence: the workspace lays out against its frame
        def open_credits(_hello: dict, path: str = credits) -> None:
            QTimer.singleShot(0, lambda: window.open_credits_file(path))

        engine.started.connect(open_credits)

    if args.smoke:
        status = {"code": 1}

        def finish() -> None:
            if args.screenshot:
                window.grab().save(args.screenshot)
            window._quit_confirmed = True
            window.close()
            app.quit()

        def ready(_hello: dict) -> None:
            status["code"] = 0
            QTimer.singleShot(2500, finish)

        engine.started.connect(ready)
        QTimer.singleShot(60_000, finish)
        app.exec()
        engine.stop()
        return status["code"]
    code = app.exec()
    engine.stop()
    return code


if __name__ == "__main__":
    raise SystemExit(main())
