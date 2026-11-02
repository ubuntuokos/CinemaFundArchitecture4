"""Keep Qt Multimedia off a PipeWire that would crash the GUI (AGENTS.md §0: never crash).

Ubuntu 26.04 always ships PipeWire, and Qt Multimedia uses it when ``libpipewire-0.3`` loads. When
libpipewire cannot create its context (above all when its client configuration is missing or
unreadable: a partial install that pulled in ``libpipewire-0.3-0`` alone, a container, a broken
upgrade), Qt's PipeWire backend does not check for that and crashes the process on the first
audio device query (playback, the meters, voice-over).

Rather than re-implement libpipewire's configuration lookup (directories, ``PIPEWIRE_NO_CONFIG``,
``PIPEWIRE_CONFIG_NAME`` / ``_PREFIX``, fallbacks, permissions), :func:`probe` asks libpipewire
itself, in a short child process with the GUI's environment, whether ``pw_context_new`` succeeds.

Qt Multimedia takes PipeWire only when ``QT_AUDIO_BACKEND`` is unset or ``pipewire``; any other
value makes it fall back in its own order (PulseAudio, which PipeWire's ``pipewire-pulse`` serves,
then ALSA in builds that have it), so naming ALSA would not reach it either. When the probe fails,
:func:`apply` therefore sets ``QT_AUDIO_BACKEND=pulseaudio`` before Qt Multimedia loads, even over
an explicit ``pipewire``, and says why on stderr. Without a sound server the GUI then has no audio
devices (silent), never a crash. A working PipeWire and any other backend the user chose are kept.
"""

from __future__ import annotations

import ctypes.util
import os
import subprocess
import sys
from collections.abc import Callable, Mapping, MutableMapping

FALLBACK = "pulseaudio"
PROBE_TIMEOUT_S = 10

# Run in a child: libpipewire logs to stderr there, and nothing it does can touch the GUI process.
PROBE = """
import ctypes, ctypes.util
lib = ctypes.CDLL(ctypes.util.find_library("pipewire-0.3"))
lib.pw_init(None, None)
lib.pw_main_loop_new.restype = ctypes.c_void_p
lib.pw_main_loop_get_loop.restype = ctypes.c_void_p
lib.pw_main_loop_get_loop.argtypes = [ctypes.c_void_p]
lib.pw_context_new.restype = ctypes.c_void_p
lib.pw_context_new.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t]
lib.pw_context_destroy.argtypes = [ctypes.c_void_p]
lib.pw_main_loop_destroy.argtypes = [ctypes.c_void_p]
loop = lib.pw_main_loop_new(None)
context = lib.pw_context_new(lib.pw_main_loop_get_loop(loop), None, 0) if loop else None
if context:
    lib.pw_context_destroy(context)
if loop:
    lib.pw_main_loop_destroy(loop)
print("pipewire-context-ok" if context else "pipewire-context-failed")
"""


def probe(environ: Mapping[str, str]) -> bool:
    """Whether libpipewire creates its context with this environment (so Qt's backend is safe)."""
    try:
        run = subprocess.run([sys.executable, "-I", "-c", PROBE], env=dict(environ), capture_output=True, text=True,
                             timeout=PROBE_TIMEOUT_S, check=False)
    except (OSError, subprocess.SubprocessError):
        return False
    return run.returncode == 0 and "pipewire-context-ok" in run.stdout


def choose(environ: MutableMapping[str, str], find_library: Callable[[str], str | None] | None = None,
           pipewire_works: Callable[[Mapping[str, str]], bool] | None = None) -> str | None:
    """The backend to force, or ``None`` when Qt's choice is safe (or the user chose another)."""
    find_library = find_library or ctypes.util.find_library
    pipewire_works = pipewire_works or probe
    chosen = environ.get("QT_AUDIO_BACKEND", "").strip()
    if chosen and chosen != "pipewire":
        return None  # Qt does not use PipeWire then
    if not find_library("pipewire-0.3"):
        return None
    if pipewire_works(environ):
        return None
    return FALLBACK


def apply(environ: MutableMapping[str, str] | None = None, log: Callable[[str], None] | None = None) -> str | None:
    """Set ``QT_AUDIO_BACKEND`` when PipeWire would crash Qt Multimedia; call before it loads."""
    env = os.environ if environ is None else environ
    backend = choose(env)
    if backend is not None:
        env["QT_AUDIO_BACKEND"] = backend
        if log is not None:
            log(f"cfa4-qt: PipeWire cannot start here (is its client.conf missing?), so Qt's PipeWire audio would "
                f"crash; using {backend} (or ALSA) instead")
    return backend


__all__ = ["apply", "choose", "probe"]
