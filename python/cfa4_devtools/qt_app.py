"""Experimental Linux Qt 6 shell. NOT the complete video editor.

The Rust cfa4-cli is only queried for its version, without a shell or remote calls.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path


def locate_engine(explicit: str | None = None) -> Path | None:
    """Find a local CLI without installing or downloading software."""
    if explicit:
        candidate = Path(explicit).expanduser()
        return candidate if candidate.is_file() else None
    # Prefer the installed CFA4 command.
    executable = shutil.which("cfa4-cli")
    if executable:
        return Path(executable)
    root = Path(__file__).resolve().parents[2]
    for relative in ("target/release/cfa4-cli", "target/debug/cfa4-cli"):
        candidate = root / relative
        if candidate.is_file():
            return candidate
    return None


def engine_version(executable: Path | None) -> str:
    if executable is None:
        return "Rust CLI: not built (cargo build --release -p cfa4-cli)"
    try:
        process = subprocess.run(
            [str(executable), "--version"], capture_output=True, text=True, timeout=8, check=False
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return f"Rust CLI unavailable: {exc}"
    if process.returncode != 0:
        return f"Rust CLI error ({process.returncode}): {(process.stderr or '').strip()}"
    return (process.stdout or "").strip() or "Rust CLI returned an empty version"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", help="Path to the local cfa4-cli executable")
    parser.add_argument("--smoke", action="store_true", help="Create Qt widgets and exit without interaction")
    args = parser.parse_args(argv)

    if not sys.platform.startswith("linux"):
        parser.error("CFA4 Qt development targets Linux only")

    # Optional dependency: the Rust workspace does not link Qt or depend on Python.
    try:
        from PySide6.QtCore import QTimer
        from PySide6.QtWidgets import QApplication, QLabel, QMainWindow, QPushButton, QVBoxLayout, QWidget
    except ImportError as exc:
        parser.error(f"Qt 6/PySide6 missing: {exc}; install: python -m pip install -e '.[qt]'")

    app = QApplication([sys.argv[0]])
    window = QMainWindow()
    window.setWindowTitle("CFA4 — Qt 6 development shell")
    container = QWidget()
    layout = QVBoxLayout(container)
    layout.addWidget(QLabel("CFA4 · Linux · Rust / Python / Qt 6"))
    layout.addWidget(QLabel("Experimental Qt UI only. Existing Rust editor is not yet ported to Qt."))
    status = QLabel()
    status.setWordWrap(True)
    engine = locate_engine(args.engine)

    def refresh() -> None:
        status.setText(engine_version(engine))

    button = QPushButton("Check local Rust engine")
    button.clicked.connect(refresh)
    layout.addWidget(status)
    layout.addWidget(button)
    window.setCentralWidget(container)
    window.resize(650, 200)
    refresh()
    window.show()
    if args.smoke:
        QTimer.singleShot(0, app.quit)
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
