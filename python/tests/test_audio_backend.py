"""The Qt GUI never crashes on a PipeWire library without its client configuration
(``cfa4_qt.audio_backend``; AGENTS.md §0)."""

from __future__ import annotations

import ctypes.util
import os
import subprocess
import sys
from pathlib import Path

import pytest
from cfa4_qt.audio_backend import apply, choose, client_config_paths, pipewire_config_dirs

ROOT = Path(__file__).resolve().parents[2]


def libs(*names: str):
    return lambda name: f"lib{name}.so" if name in names else None


def test_config_dirs_follow_libpipewire(tmp_path: Path) -> None:
    assert pipewire_config_dirs({"PIPEWIRE_CONFIG_DIR": str(tmp_path), "XDG_CONFIG_HOME": "/x"}) == [tmp_path]
    assert pipewire_config_dirs({"XDG_CONFIG_HOME": "/x", "HOME": "/h"}) == [
        Path("/x/pipewire"), Path("/etc/pipewire"), Path("/usr/share/pipewire")]
    assert pipewire_config_dirs({"HOME": "/h"})[0] == Path("/h/.config/pipewire")
    # PIPEWIRE_NO_CONFIG: libpipewire reads only its packaged files
    assert pipewire_config_dirs({"PIPEWIRE_NO_CONFIG": "true", "PIPEWIRE_CONFIG_DIR": str(tmp_path)}) == [
        Path("/usr/share/pipewire")]



def test_client_config_paths_follow_the_name_and_prefix_overrides(tmp_path: Path) -> None:
    base = {"PIPEWIRE_CONFIG_DIR": str(tmp_path)}
    assert client_config_paths(base) == [tmp_path / "client.conf"]
    assert client_config_paths({**base, "PIPEWIRE_CONFIG_NAME": "cfa4.conf"}) == [tmp_path / "cfa4.conf"]
    assert client_config_paths({**base, "PIPEWIRE_CONFIG_PREFIX": "p"}) == [tmp_path / "p" / "client.conf"]
    assert client_config_paths({**base, "PIPEWIRE_CONFIG_NAME": "/x/y.conf"}) == [Path("/x/y.conf")]
    # a valid custom configuration keeps PipeWire
    (tmp_path / "cfa4.conf").write_text("context.properties = {}\n", encoding="utf-8")
    assert choose({**base, "PIPEWIRE_CONFIG_NAME": "cfa4.conf"}, libs("pipewire-0.3")) is None
    assert choose(dict(base), libs("pipewire-0.3")) == "pulseaudio"


def test_choose_moves_qt_off_pipewire_only_without_its_client_config(tmp_path: Path) -> None:
    empty = {"PIPEWIRE_CONFIG_DIR": str(tmp_path)}
    # no libpipewire: Qt never picks PipeWire
    assert choose(dict(empty), libs("pulse")) is None
    # libpipewire without client.conf: off PipeWire (Qt then tries PulseAudio, then ALSA)
    assert choose(dict(empty), libs("pipewire-0.3", "pulse")) == "pulseaudio"
    assert choose(dict(empty), libs("pipewire-0.3")) == "pulseaudio"
    # even when the user named PipeWire: it would crash (AGENTS §0)
    assert choose({**empty, "QT_AUDIO_BACKEND": "pipewire"}, libs("pipewire-0.3")) == "pulseaudio"
    # another backend the user chose is kept: Qt does not touch PipeWire then
    assert choose({**empty, "QT_AUDIO_BACKEND": "alsa"}, libs("pipewire-0.3")) is None
    # with its configuration PipeWire is safe, also when named
    (tmp_path / "client.conf").write_text("context.properties = {}\n", encoding="utf-8")
    assert choose(dict(empty), libs("pipewire-0.3", "pulse")) is None
    assert choose({**empty, "QT_AUDIO_BACKEND": "pipewire"}, libs("pipewire-0.3")) is None


def test_apply_sets_the_variable_and_says_why(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.setattr(ctypes.util, "find_library", libs("pipewire-0.3"))
    env = {"PIPEWIRE_CONFIG_DIR": str(tmp_path)}
    said: list[str] = []
    assert apply(env, said.append) == "pulseaudio"
    assert env["QT_AUDIO_BACKEND"] == "pulseaudio" and "client.conf" in said[0]
    assert apply(env, said.append) is None and len(said) == 1


@pytest.mark.skipif(not ctypes.util.find_library("pipewire-0.3"), reason="libpipewire-0.3 is not installed")
def test_qt_audio_query_survives_pipewire_without_config(tmp_path: Path) -> None:
    """Regression: with libpipewire-0.3 but no client.conf, ``QMediaDevices.defaultAudioOutput()``
    segfaulted in Qt's PipeWire backend."""
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
