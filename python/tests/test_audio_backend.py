"""The Qt GUI never crashes on a PipeWire that cannot start (``cfa4_qt.audio_backend``; AGENTS.md §0)."""

from __future__ import annotations

import ctypes.util
import os
import subprocess
import sys
from pathlib import Path

import pytest
from cfa4_qt.audio_backend import apply, choose, probe

ROOT = Path(__file__).resolve().parents[2]
HAS_PIPEWIRE = bool(ctypes.util.find_library("pipewire-0.3"))


def libs(*names: str):
    return lambda name: f"lib{name}.so" if name in names else None


def works(answer: bool):
    return lambda _env: answer


def test_choose_moves_qt_off_pipewire_only_when_it_cannot_start() -> None:
    # no libpipewire: Qt never picks PipeWire (and nothing is probed)
    assert choose({}, libs("pulse"), works(False)) is None
    # a PipeWire that starts is kept, also when named
    assert choose({}, libs("pipewire-0.3"), works(True)) is None
    assert choose({"QT_AUDIO_BACKEND": "pipewire"}, libs("pipewire-0.3"), works(True)) is None
    # one that cannot start: off PipeWire (Qt then tries PulseAudio, then ALSA), even when named
    assert choose({}, libs("pipewire-0.3"), works(False)) == "pulseaudio"
    assert choose({"QT_AUDIO_BACKEND": "pipewire"}, libs("pipewire-0.3"), works(False)) == "pulseaudio"
    # another backend the user chose is kept: Qt does not touch PipeWire then
    assert choose({"QT_AUDIO_BACKEND": "alsa"}, libs("pipewire-0.3"), works(False)) is None


def test_apply_sets_the_variable_and_says_why(monkeypatch) -> None:
    monkeypatch.setattr(ctypes.util, "find_library", libs("pipewire-0.3"))
    monkeypatch.setattr("cfa4_qt.audio_backend.probe", works(False))
    env: dict[str, str] = {}
    said: list[str] = []
    assert apply(env, said.append) == "pulseaudio"
    assert env["QT_AUDIO_BACKEND"] == "pulseaudio" and "client.conf" in said[0]
    assert apply(env, said.append) is None and len(said) == 1


@pytest.mark.skipif(not HAS_PIPEWIRE, reason="libpipewire-0.3 is not installed")
def test_probe_asks_libpipewire_itself(tmp_path: Path) -> None:
    """The probe follows libpipewire's own lookup: a missing, a custom-named and an unreadable
    configuration, without CFA4 re-implementing it."""
    base = {k: v for k, v in os.environ.items() if not k.startswith("PIPEWIRE_")}
    assert not probe({**base, "PIPEWIRE_CONFIG_DIR": str(tmp_path / "none")})
    conf = tmp_path / "conf"
    conf.mkdir()
    (conf / "cfa4.conf").write_text("context.properties = {}\n", encoding="utf-8")
    assert probe({**base, "PIPEWIRE_CONFIG_DIR": str(conf), "PIPEWIRE_CONFIG_NAME": "cfa4.conf"})
    (conf / "client.conf").write_text("context.properties = {}\n", encoding="utf-8")
    assert probe({**base, "PIPEWIRE_CONFIG_DIR": str(conf)})
    if os.geteuid() != 0:  # root reads any file
        (conf / "client.conf").chmod(0)
        assert not probe({**base, "PIPEWIRE_CONFIG_DIR": str(conf)})


@pytest.mark.skipif(not HAS_PIPEWIRE, reason="libpipewire-0.3 is not installed")
def test_qt_audio_query_survives_pipewire_without_config(tmp_path: Path) -> None:
    """Regression: with libpipewire-0.3 but no client.conf, ``QMediaDevices.defaultAudioOutput()``
    segfaulted in Qt's PipeWire backend, also with ``QT_AUDIO_BACKEND=pipewire``."""
    pytest.importorskip("PySide6.QtMultimedia")
    code = (
        "import sys; from cfa4_qt.audio_backend import apply; apply(log=print)\n"
        "from PySide6.QtWidgets import QApplication; app = QApplication([])\n"
        "from PySide6.QtMultimedia import QMediaDevices; QMediaDevices.defaultAudioOutput(); print('alive')\n"
    )
    for named in (None, "pipewire"):  # Qt's own choice, and the user naming PipeWire
        env = {k: v for k, v in os.environ.items() if k != "QT_AUDIO_BACKEND"}
        env.update(PIPEWIRE_CONFIG_DIR=str(tmp_path / "none"), QT_QPA_PLATFORM="offscreen", PYTHONPATH=str(ROOT / "python"))
        if named:
            env["QT_AUDIO_BACKEND"] = named
        run = subprocess.run([sys.executable, "-c", code], env=env, capture_output=True, text=True, timeout=120, check=False)
        assert run.returncode == 0 and "alive" in run.stdout, (named, run.stdout + run.stderr)
