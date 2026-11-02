"""Keep Qt Multimedia off a PipeWire that would crash the GUI (AGENTS.md §0: never crash).

Ubuntu 26.04 always ships PipeWire, and Qt Multimedia uses it when ``libpipewire-0.3`` loads. When
the library is there but PipeWire's client configuration is not (a partial install: a package
pulled in ``libpipewire-0.3-0`` alone, a container, a broken upgrade), libpipewire cannot create
its context and Qt's PipeWire backend crashes the process on the first audio device query
(playback, the meters, voice-over).

Qt Multimedia takes PipeWire only when ``QT_AUDIO_BACKEND`` is unset or ``pipewire``; any other
value makes it fall back in its own order (PulseAudio, which PipeWire's ``pipewire-pulse`` serves,
then ALSA in builds that have it), so naming ALSA would not reach it either. Before Qt Multimedia
loads, :func:`apply` therefore sets ``QT_AUDIO_BACKEND=pulseaudio`` on such a system, even over an
explicit ``pipewire``, and says why on stderr. Without a sound server the GUI then has no audio
devices (silent), never a crash. A working PipeWire and any other backend the user chose are kept.
"""

from __future__ import annotations

import ctypes.util
import os
from collections.abc import Callable, MutableMapping
from pathlib import Path

CLIENT_CONF = "client.conf"
DATA_DIR = Path("/usr/share/pipewire")
FALLBACK = "pulseaudio"


def _true(value: str) -> bool:
    return value.strip().lower() in ("1", "true", "yes", "on")


def pipewire_config_dirs(environ: MutableMapping[str, str]) -> list[Path]:
    """Where libpipewire looks for ``client.conf``: ``PIPEWIRE_CONFIG_DIR`` alone when set, else
    the user's, the administrator's and the package's directories."""
    if _true(environ.get("PIPEWIRE_NO_CONFIG", "")):
        # libpipewire then ignores PIPEWIRE_CONFIG_DIR and the user's and administrator's files
        return [DATA_DIR]
    explicit = environ.get("PIPEWIRE_CONFIG_DIR", "").strip()
    if explicit:
        return [Path(explicit)]
    config_home = environ.get("XDG_CONFIG_HOME", "").strip() or str(Path(environ.get("HOME", "~")).expanduser() / ".config")
    return [Path(config_home) / "pipewire", Path("/etc/pipewire"), DATA_DIR]


def client_config_paths(environ: MutableMapping[str, str]) -> list[Path]:
    """The client configuration files libpipewire would read: ``client.conf``, or the file that
    ``PIPEWIRE_CONFIG_NAME`` names (an absolute path is taken as it is), under each directory's
    ``PIPEWIRE_CONFIG_PREFIX`` subdirectory when one is set."""
    name = environ.get("PIPEWIRE_CONFIG_NAME", "").strip() or CLIENT_CONF
    if Path(name).is_absolute():
        return [Path(name)]
    prefix = environ.get("PIPEWIRE_CONFIG_PREFIX", "").strip()
    if prefix and Path(prefix).is_absolute():
        return [Path(prefix) / name]
    return [(d / prefix / name) if prefix else (d / name) for d in pipewire_config_dirs(environ)]


def choose(environ: MutableMapping[str, str], find_library: Callable[[str], str | None] | None = None) -> str | None:
    """The backend to force, or ``None`` when Qt's choice is safe (or the user chose another)."""
    find_library = find_library or ctypes.util.find_library
    chosen = environ.get("QT_AUDIO_BACKEND", "").strip()
    if chosen and chosen != "pipewire":
        return None  # Qt does not use PipeWire then
    if not find_library("pipewire-0.3"):
        return None
    if any(path.is_file() for path in client_config_paths(environ)):
        return None
    return FALLBACK


def apply(environ: MutableMapping[str, str] | None = None, log: Callable[[str], None] | None = None) -> str | None:
    """Set ``QT_AUDIO_BACKEND`` when PipeWire would crash Qt Multimedia; call before it loads."""
    env = os.environ if environ is None else environ
    backend = choose(env)
    if backend is not None:
        env["QT_AUDIO_BACKEND"] = backend
        if log is not None:
            log(f"cfa4-qt: PipeWire's client configuration ({CLIENT_CONF}) is missing, so Qt's PipeWire audio would "
                f"crash; using {backend} (or ALSA) instead")
    return backend


__all__ = ["apply", "choose", "client_config_paths", "pipewire_config_dirs"]
