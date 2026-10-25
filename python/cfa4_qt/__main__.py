"""``python -m cfa4_qt`` / ``cfa4-qt``: start the CFA4 Qt 6 GUI."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="cfa4-qt", description="CFA4 Qt 6 editor (Rust engine + PySide6)")
    parser.add_argument("project", nargs="?", help="project file (.fcproj) to open")
    parser.add_argument("--engine", help="path to cfa4-cli / filmcraft-cli (default: PATH, then target/)")
    parser.add_argument("--demo", action="store_true", help="open the built-in demo project")
    parser.add_argument("--data-dir", help="CFA4 data directory (auto-save, recovery, presets)")
    parser.add_argument("--no-autosave", action="store_true", help="no auto-save or crash-recovery journal")
    parser.add_argument("--lang", help="UI language (en, hu, de, ...); default: the system locale")
    parser.add_argument("--smoke", action="store_true", help="start, wait for the engine, then quit (CI check)")
    parser.add_argument("--screenshot", help="with --smoke: save a PNG of the window before quitting")
    args = parser.parse_args(argv)

    if not sys.platform.startswith("linux"):
        parser.error("CFA4 targets Linux (Ubuntu) only")
    try:
        from PySide6.QtCore import QSettings, QTimer
        from PySide6.QtWidgets import QApplication, QMessageBox
    except ImportError as exc:
        parser.error(f"Qt 6 / PySide6 missing: {exc}; install: python -m pip install -e '.[qt]'")

    from . import i18n
    from .engine import EngineClient, EngineError, locate_engine
    from .main_window import MainWindow

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
    settings = QSettings("CFA4", "CFA4-Qt")
    language = args.lang or str(settings.value("language") or "") or i18n.system_language()
    i18n.TR.set_language(language)

    rejected: list[str] = []
    engine_path = locate_engine(args.engine, rejected)
    if engine_path is None:
        message = i18n.tr(
            "The CFA4 engine (cfa4-cli) was not found. Build it with: cargo build --release -p filmcraft-cli"
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
    engine_args: list[str] = []
    if args.project:
        engine_args += ["--project", str(Path(args.project).expanduser().resolve())]
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
    window.show()

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
