"""Pick a Qt Multimedia audio backend that cannot crash the GUI (AGENTS.md §0: never crash).

Ubuntu 26.04 always ships PipeWire, and Qt Multimedia uses it when ``libpipewire-0.3`` loads. When
the library is there but PipeWire's client configuration is not (a partial install: a package
pulled in ``libpipewire-0.3-0`` alone, a container, a broken upgrade), libpipewire cannot create
its context and Qt's PipeWire backend crashes the process on the first audio device query
(playback, the meters, voice-over). Before Qt Multimedia loads, :func:`apply` then sets
``QT_AUDIO_BACKEND`` to PulseAudio (PipeWire's own ``pipewire-pulse`` serves it), or ALSA without
libpulse. A backend the user chose in ``QT_AUDIO_BACKEND`` is always kept.
"""

from __future__ import annotations

import ctypes.util
import os
from collections.abc import Callable, MutableMapping
from pathlib import Path

CLIENT_CONF = "client.conf"


def pipewire_config_dirs(environ: MutableMapping[str, str]) -> list[Path]:
    """Where libpipewire looks for ``client.conf``: ``PIPEWIRE_CONFIG_DIR`` alone when set, else
    the user's, the administrator's and the package's directories."""
    explicit = environ.get("PIPEWIRE_CONFIG_DIR", "").strip()
    if explicit:
        return [Path(explicit)]
    config_home = environ.get("XDG_CONFIG_HOME", "").strip() or str(Path(environ.get("HOME", "~")).expanduser() / ".config")
    return [Path(config_home) / "pipewire", Path("/etc/pipewire"), Path("/usr/share/pipewire")]


def choose(environ: MutableMapping[str, str], find_library: Callable[[str], str | None] | None = None) -> str | None:
    """The backend to force, or ``None`` when Qt's own choice is safe (or the user made one)."""
    find_library = find_library or ctypes.util.find_library
    if environ.get("QT_AUDIO_BACKEND", "").strip():
        return None
    if not find_library("pipewire-0.3"):
        return None
    if any((d / CLIENT_CONF).is_file() for d in pipewire_config_dirs(environ)):
        return None
    return "pulseaudio" if find_library("pulse") else "alsa"


def apply(environ: MutableMapping[str, str] | None = None, log: Callable[[str], None] | None = None) -> str | None:
    """Set ``QT_AUDIO_BACKEND`` when PipeWire would crash Qt Multimedia; call before it loads."""
    env = os.environ if environ is None else environ
    backend = choose(env)
    if backend is not None:
        env["QT_AUDIO_BACKEND"] = backend
        if log is not None:
            log(f"cfa4-qt: PipeWire's client configuration ({CLIENT_CONF}) is missing; using the {backend} audio backend")
    return backend


__all__ = ["apply", "choose", "pipewire_config_dirs"]
