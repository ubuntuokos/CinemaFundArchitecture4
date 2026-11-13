"""CFA4 OTIO-1.2: OpenTimelineIO samples, written by the official library, for the CFA4 importer.

The reverse direction of OTIO-1.1 (``otio_oracle``): the official ``opentimelineio`` package writes
one small file per situation the CFA4 OTIO reader handles, covering every schema it reads
(``Timeline.1``, ``Stack.1``, ``Track.1``, ``Clip.1``/``Clip.2``, ``Gap.1``, ``Transition.1``,
``Marker.1``/``Marker.2``, ``ExternalReference.1``, ``MissingReference.1``,
``GeneratorReference.1``, ``LinearTimeWarp.1``, ``FreezeFrame.1``, ``Effect.1``,
``SerializableCollection.1``, ``ImageSequenceReference.1``, a bare ``Track.1`` as the whole document,
and a clip with several media references and an active key). The library is a development tool only, like ffmpeg: never linked
into or shipped with CFA4 (docs/plans/OTIO-INTEGRATION-2026-10-24.md).

Next to the files it writes ``samples.json``: per file the timelines OTIO itself sees (tracks,
clip and nested-stack names, each one's position and length as OTIO's ``range_in_parent`` computes
them, in exact engine ticks, transition counts and positions, and marker positions; per clip the
media it points at, enabled, speed, reverse and hold; per timeline the frame rate and start
timecode) and the import warnings CFA4 is expected to give. The Rust test ``oracle_samples`` imports every file and checks it against this::

    python -m cfa4_devtools.otio_samples target/fixtures/otio-samples
    CFA4_OTIO_SAMPLES_DIR=target/fixtures/otio-samples \\
        cargo test -p cfa4-interchange --test otio oracle_samples

Exit status: 0 the samples were written, 2 they could not be (``opentimelineio`` missing, the
directory not writable).
"""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Callable
from fractions import Fraction
from pathlib import Path
from typing import Any

from cfa4_devtools.otio_oracle import OracleError, load_otio

#: Engine time base (``cfa4_time::TICKS_PER_SECOND``): every common frame and sample rate divides it.
TICKS_PER_SECOND = 254_016_000_000
NTSC = 30000 / 1001
FILM = 24000 / 1001
NTSC_60 = 60000 / 1001
#: The base directory the Rust test passes to the importer: relative media paths resolve under it.
BASE_DIR = "/media"
#: OTIO generator kinds CFA4 knows (`crates/interchange/src/otio.rs`); any other imports as Black Video.
GENERATORS = {
    "solidcolor": "ColorMatte",
    "solid_color": "ColorMatte",
    "smptebars": "BarsAndTone",
    "bars": "BarsAndTone",
}


def ticks_of(rational_time: Any) -> int:
    """Engine ticks of an OTIO RationalTime, rounded to the nearest tick."""
    rate = Fraction(rational_time.rate)
    if rate <= 0:
        raise ValueError(f"non-positive rate {rational_time.rate!r}")
    return round(Fraction(rational_time.value) / rate * TICKS_PER_SECOND)


def file_path(url: str) -> str:
    """The path CFA4 opens for a media URL: ``file://`` URLs decoded, relative paths under BASE_DIR."""
    from urllib.parse import unquote, urlparse

    if url.startswith("file://"):
        return unquote(urlparse(url).path)
    return url if url.startswith("/") else f"{BASE_DIR}/{url}"


def expect_media(otio: Any, clip: Any) -> dict[str, Any]:
    """The project item a clip must point at, from its active media reference."""
    ref = clip.media_reference
    if isinstance(ref, otio.schema.ExternalReference) and ref.target_url.strip():
        return {"ref": "file", "path": file_path(ref.target_url), "offline": False}
    if isinstance(ref, otio.schema.ImageSequenceReference) and ref.target_url_base.strip():
        # how CFA4 keeps an image sequence is OTIO-2.3; here only that it is a file and online
        return {"ref": "file", "path": None, "offline": False}
    if isinstance(ref, otio.schema.GeneratorReference):
        return {
            "ref": "generator",
            "generator": GENERATORS.get(ref.generator_kind.lower(), "BlackVideo"),
            "offline": False,
        }
    return {"ref": "file", "path": None, "offline": True}


def expect_time_effects(otio: Any, clip: Any) -> dict[str, Any]:
    speed, reverse, hold = 1.0, False, False
    for effect in clip.effects:
        if isinstance(effect, otio.schema.FreezeFrame):
            hold = True
        elif isinstance(effect, otio.schema.LinearTimeWarp):
            if effect.time_scalar == 0:
                hold = True
            else:
                speed, reverse = abs(effect.time_scalar), effect.time_scalar < 0
    return {"speed": speed, "reverse": reverse, "frame_hold": hold}


def _first_rate(otio: Any, composition: Any) -> float | None:
    """The rate CFA4 falls back to without a global start time: the first ``source_range`` found."""
    for child in composition:
        if getattr(child, "source_range", None) is not None:
            rate: float = child.source_range.duration.rate
            return rate
        if isinstance(child, otio.schema.Track | otio.schema.Stack):
            inner = _first_rate(otio, child)
            if inner is not None:
                return inner
    return None


def expect_timeline(otio: Any, timeline: Any) -> dict[str, Any]:
    """What CFA4 should import from one timeline: tracks grouped by kind (video first), as CFA4 orders them."""
    tracks = []
    markers = [ticks_of(m.marked_range.start_time) for m in timeline.tracks.markers]
    for track in timeline.tracks:
        if not isinstance(track, otio.schema.Track):
            continue  # CFA4 reports anything else directly in the stack and skips it
        items, transitions, transition_ranges = [], 0, []
        markers += [ticks_of(m.marked_range.start_time) for m in track.markers]
        children = list(track)
        cut = 0  # ticks: where the previous slot ends
        for index, child in enumerate(children):
            if isinstance(child, otio.schema.Transition):
                transitions += 1
                inn, out = ticks_of(child.in_offset), ticks_of(child.out_offset)
                transition_ranges.append([cut - inn, inn + out])
                continue
            placed = child.range_in_parent()
            cut = ticks_of(placed.end_time_exclusive())
            if isinstance(child, otio.schema.Gap):
                continue
            item: dict[str, Any] = {
                "name": child.name,
                "start_ticks": ticks_of(placed.start_time),
                "duration_ticks": ticks_of(placed.duration),
                "enabled": bool(child.enabled),
                # A nested stack's own markers belong to the nested sequence, not to its instance.
                "markers": [],
                "media": {"ref": "sequence"},
            }
            if isinstance(child, otio.schema.Clip):
                # clip markers are in media time, from the start of the media's available range
                avail = child.media_reference.available_range if child.media_reference is not None else None
                origin = ticks_of(avail.start_time) if avail is not None else 0
                item["markers"] = [ticks_of(m.marked_range.start_time) - origin for m in child.markers]
                item["media"] = expect_media(otio, child)
                item.update(expect_time_effects(otio, child))
            items.append(item)
        kind = "Audio" if track.kind == otio.schema.TrackKind.Audio else "Video"
        tracks.append(
            {
                "kind": kind,
                "name": track.name,
                "items": items,
                "transitions": transitions,
                "transition_ranges": transition_ranges,
            }
        )
    tracks.sort(key=lambda t: t["kind"] != "Video")  # stable: keeps each kind's order
    start = timeline.global_start_time
    # CFA4 keeps the timeline's and the tracks' markers together on the sequence, by start
    return {
        "name": timeline.name,
        "rate": start.rate if start is not None else _first_rate(otio, timeline.tracks),
        "start_timecode": round(start.value) if start is not None else 0,
        "markers": sorted(markers),
        "tracks": tracks,
    }


# --- building blocks -----------------------------------------------------------------------------


def _rt(otio: Any, value: float, rate: float) -> Any:
    return otio.opentime.RationalTime(value, rate)


def _tr(otio: Any, start: float, duration: float, rate: float) -> Any:
    return otio.opentime.TimeRange(_rt(otio, start, rate), _rt(otio, duration, rate))


def _file(otio: Any, path: str, start: float, duration: float, rate: float) -> Any:
    return otio.schema.ExternalReference(target_url=path, available_range=_tr(otio, start, duration, rate))


def _clip(otio: Any, name: str, ref: Any, start: float, duration: float, rate: float) -> Any:
    return otio.schema.Clip(name=name, media_reference=ref, source_range=_tr(otio, start, duration, rate))


def _gap(otio: Any, duration: float, rate: float) -> Any:
    return otio.schema.Gap(source_range=_tr(otio, 0, duration, rate))


def _marker(otio: Any, name: str, start: float, duration: float, rate: float) -> Any:
    return otio.schema.Marker(
        name=name, marked_range=_tr(otio, start, duration, rate), color=otio.schema.MarkerColor.GREEN
    )


def _timeline(otio: Any, name: str, rate: float, tracks: list[Any]) -> Any:
    tl = otio.schema.Timeline(name=name, global_start_time=_rt(otio, 0, rate))
    for t in tracks:
        tl.tracks.append(t)
    return tl


# --- the samples ---------------------------------------------------------------------------------


def edit(otio: Any, rate: float = 25, name: str = "Edit") -> Any:
    """Gaps, a dissolve between two file clips, markers on clip, track and timeline, linked audio."""
    v = otio.schema.Track(name="V1", kind=otio.schema.TrackKind.Video)
    v.append(_gap(otio, 10, rate))
    one = _clip(otio, "one", _file(otio, "/media/one.mov", 0, 500, rate), 100, 50, rate)
    one.markers.append(_marker(otio, "clip note", 110, 0, rate))
    v.append(one)
    v.append(
        otio.schema.Transition(
            name="Cross Dissolve",
            transition_type=otio.schema.TransitionTypes.SMPTE_Dissolve,
            in_offset=_rt(otio, 6, rate),
            out_offset=_rt(otio, 6, rate),
        )
    )
    v.append(_clip(otio, "two", _file(otio, "/media/two.mov", 0, 500, rate), 20, 40, rate))
    v.markers.append(_marker(otio, "track note", 30, 5, rate))
    a = otio.schema.Track(name="A1", kind=otio.schema.TrackKind.Audio)
    a.append(_gap(otio, 10, rate))
    a.append(_clip(otio, "one", _file(otio, "/media/one.mov", 0, 500, rate), 100, 50, rate))
    tl = _timeline(otio, name, rate, [v, a])
    tl.tracks.markers.append(_marker(otio, "timeline note", 0, 0, rate))
    return tl


def references(otio: Any, rate: float = NTSC) -> Any:
    """Every media reference kind; the ones CFA4 cannot use come in offline or as Black Video."""
    v = otio.schema.Track(name="V1", kind=otio.schema.TrackKind.Video)
    v.append(_clip(otio, "file", _file(otio, "/media/file.mov", 0, 300, rate), 0, 30, rate))
    v.append(_clip(otio, "missing", otio.schema.MissingReference(), 0, 30, rate))
    gen = otio.schema.GeneratorReference(generator_kind="SMPTEBars", available_range=_tr(otio, 0, 300, rate))
    v.append(_clip(otio, "bars", gen, 0, 30, rate))
    v.append(_clip(otio, "black", otio.schema.GeneratorReference(generator_kind="black"), 0, 30, rate))
    v.append(_clip(otio, "noise", otio.schema.GeneratorReference(generator_kind="Noise"), 0, 30, rate))
    v.append(_clip(otio, "no path", otio.schema.ExternalReference(target_url=""), 0, 30, rate))
    return _timeline(otio, "References", rate, [v])


def time_effects(otio: Any, rate: float = 24) -> Any:
    """Speed, reverse and freeze as LinearTimeWarp and FreezeFrame; an effect CFA4 does not know."""
    v = otio.schema.Track(name="V1", kind=otio.schema.TrackKind.Video)
    media = "/media/fx.mov"
    fast = _clip(otio, "fast", _file(otio, media, 0, 1000, rate), 0, 20, rate)
    fast.effects.append(otio.schema.LinearTimeWarp(time_scalar=2.0))
    rev = _clip(otio, "reverse", _file(otio, media, 0, 1000, rate), 200, 20, rate)
    rev.effects.append(otio.schema.LinearTimeWarp(time_scalar=-1.0))
    hold = _clip(otio, "hold", _file(otio, media, 0, 1000, rate), 300, 20, rate)
    hold.effects.append(otio.schema.FreezeFrame())
    blur = _clip(otio, "blur", _file(otio, media, 0, 1000, rate), 400, 20, rate)
    blur.effects.append(otio.schema.Effect(name="Blur", effect_name="Blur"))
    off = _clip(otio, "disabled", _file(otio, media, 0, 1000, rate), 500, 20, rate)
    off.enabled = False
    for c in (fast, rev, hold, blur, off):
        v.append(c)
    return _timeline(otio, "Time effects", rate, [v])


def nested(otio: Any, rate: float = 25) -> Any:
    """A Stack inside a track (a nested sequence), trimmed, after a gap."""
    inner_v = otio.schema.Track(name="V1", kind=otio.schema.TrackKind.Video)
    inner_v.append(_clip(otio, "inner", _file(otio, "/media/inner.mov", 0, 500, rate), 0, 100, rate))
    inner = otio.schema.Stack(name="Inner", source_range=_tr(otio, 25, 50, rate))
    inner.append(inner_v)
    inner.markers.append(_marker(otio, "inner note", 30, 0, rate))
    v = otio.schema.Track(name="V1", kind=otio.schema.TrackKind.Video)
    v.append(_gap(otio, 5, rate))
    v.append(inner)
    v.append(_clip(otio, "after", _file(otio, "/media/after.mov", 0, 500, rate), 0, 25, rate))
    return _timeline(otio, "Outer", rate, [v])


def transitions(otio: Any, rate: float = FILM) -> Any:
    """Transitions at both ends of a clip, a named custom one, two video and two audio tracks."""
    v1 = otio.schema.Track(name="V1", kind=otio.schema.TrackKind.Video)
    v1.append(
        otio.schema.Transition(
            name="Fade In",
            transition_type=otio.schema.TransitionTypes.SMPTE_Dissolve,
            in_offset=_rt(otio, 0, rate),
            out_offset=_rt(otio, 12, rate),
        )
    )
    v1.append(_clip(otio, "a", _file(otio, "/media/a.mov", 0, 500, rate), 0, 48, rate))
    v1.append(
        otio.schema.Transition(
            name="Wipe",
            transition_type=otio.schema.TransitionTypes.Custom,
            in_offset=_rt(otio, 0, rate),
            out_offset=_rt(otio, 8, rate),
        )
    )
    v1.append(_clip(otio, "b", _file(otio, "/media/b.mov", 0, 500, rate), 10, 48, rate))
    v2 = otio.schema.Track(name="V2", kind=otio.schema.TrackKind.Video)
    v2.append(_gap(otio, 24, rate))
    v2.append(_clip(otio, "title", otio.schema.GeneratorReference(generator_kind="SMPTEBars"), 0, 24, rate))
    a1 = otio.schema.Track(name="A1", kind=otio.schema.TrackKind.Audio)
    a1.append(_clip(otio, "a", _file(otio, "/media/a.mov", 0, 500, rate), 0, 48, rate))
    a2 = otio.schema.Track(name="A2", kind=otio.schema.TrackKind.Audio)
    a2.append(_gap(otio, 12, rate))
    a2.append(_clip(otio, "music", _file(otio, "/media/music.wav", 0, 5000, rate), 0, 84, rate))
    # interleaved kinds: CFA4 groups the tracks by kind, keeping each kind's order
    return _timeline(otio, "Transitions", rate, [v1, a1, v2, a2])


def start_timecode(otio: Any, rate: float) -> Any:
    """The edit at one hour of timecode; media whose available range does not start at 0."""
    tl = edit(otio, rate, f"Hour {rate:.3f}")
    tl.global_start_time = _rt(otio, round(rate) * 3600, rate)
    for track in tl.tracks:
        for child in track:
            if isinstance(child, otio.schema.Clip):
                avail = child.media_reference.available_range
                child.media_reference.available_range = _tr(otio, 1000, avail.duration.value, rate)
                child.source_range = _tr(
                    otio, 1000 + child.source_range.start_time.value, child.source_range.duration.value, rate
                )
                for m in child.markers:
                    m.marked_range = _tr(
                        otio, 1000 + m.marked_range.start_time.value, m.marked_range.duration.value, rate
                    )
    return tl


def media_paths(otio: Any, rate: float = 24) -> Any:
    """An image sequence, several media references with an active key, URL and relative paths."""
    frames = otio.schema.ImageSequenceReference(
        target_url_base="file:///frames/",
        name_prefix="shot.",
        name_suffix=".exr",
        start_frame=1,
        frame_step=1,
        rate=rate,
        frame_zero_padding=4,
        available_range=_tr(otio, 0, 48, rate),
    )
    multi = otio.schema.Clip(name="multi", source_range=_tr(otio, 0, 24, rate))
    multi.set_media_references(
        {
            "DEFAULT_MEDIA": otio.schema.ExternalReference(target_url="file:///proxy/multi.mov"),
            "HIGH": otio.schema.ExternalReference(target_url="file:///full/multi.mov"),
        },
        "HIGH",
    )
    colour = otio.schema.GeneratorReference(generator_kind="SolidColor", parameters={"color": [1.0, 0.0, 0.0, 1.0]})
    v = otio.schema.Track(name="V1", kind=otio.schema.TrackKind.Video)
    v.append(_clip(otio, "frames", frames, 0, 48, rate))
    v.append(multi)
    v.append(_clip(otio, "url", otio.schema.ExternalReference(target_url="file:///footage/shot%20a.mov"), 0, 24, rate))
    v.append(_clip(otio, "relative", otio.schema.ExternalReference(target_url="clips/b.mov"), 0, 24, rate))
    v.append(_clip(otio, "red", colour, 0, 24, rate))
    return _timeline(otio, "Media paths", rate, [v])


def bare_track(otio: Any, rate: float = 30) -> Any:
    """A Track as the whole document: CFA4 makes one sequence named after it."""
    v = otio.schema.Track(name="Solo", kind=otio.schema.TrackKind.Video)
    v.append(_gap(otio, 15, rate))
    v.append(_clip(otio, "only", _file(otio, "/media/only.mov", 0, 500, rate), 0, 45, rate))
    return v


def clip_in_stack(otio: Any, rate: float = 24) -> Any:
    """A clip directly in the top-level stack, which CFA4 reports and skips."""
    v = otio.schema.Track(name="V1", kind=otio.schema.TrackKind.Video)
    v.append(_clip(otio, "kept", _file(otio, "/media/kept.mov", 0, 500, rate), 0, 24, rate))
    tl = _timeline(otio, "Clip in stack", rate, [v])
    tl.tracks.append(_clip(otio, "stray", _file(otio, "/media/stray.mov", 0, 500, rate), 0, 24, rate))
    return tl


def collection(otio: Any) -> Any:
    """Two timelines at different rates in a SerializableCollection."""
    coll = otio.schema.SerializableCollection(name="Bundle")
    coll.append(edit(otio, 25, "First"))
    coll.append(edit(otio, 50, "Second"))
    return coll


#: file name -> (builder, target schema versions, expected CFA4 import warnings as message substrings)
SAMPLES: dict[str, tuple[Callable[[Any], Any], dict[str, int] | None, list[str]]] = {
    "edit_25.otio": (edit, None, []),
    "edit_legacy_clip1_marker1.otio": (edit, {"Clip": 1, "Marker": 1}, []),
    "references_2997.otio": (
        references,
        None,
        ['OTIO generator "noise" is not supported', "OTIO media reference without a path"],
    ),
    "time_effects_24.otio": (time_effects, None, ['OTIO effect "Blur" is not supported']),
    "nested_25.otio": (nested, None, []),
    "transitions_23976.otio": (transitions, None, []),
    "collection.otio": (collection, None, []),
    "start_timecode_30.otio": (lambda otio: start_timecode(otio, 30), None, []),
    "start_timecode_5994.otio": (lambda otio: start_timecode(otio, NTSC_60), None, []),
    "start_timecode_60.otio": (lambda otio: start_timecode(otio, 60), None, []),
    "media_paths_24.otio": (media_paths, None, []),
    "bare_track_30.otio": (bare_track, None, []),
    "clip_in_stack.otio": (clip_in_stack, None, ["OTIO Clip directly in a stack is not supported"]),
}


def _marker_v1(value: Any) -> Any:
    """``Marker.2`` -> ``Marker.1`` (OTIO 0.18 has no downgrader for it): version 1 keeps the range
    under ``range``, version 2 renamed it to ``marked_range``; OTIO's upgrader maps it back on read."""
    if isinstance(value, dict):
        out = {k: _marker_v1(v) for k, v in value.items()}
        if out.get("OTIO_SCHEMA") == "Marker.2":
            out["OTIO_SCHEMA"] = "Marker.1"
            out["range"] = out.pop("marked_range", None)
            out.pop("comment", None)
        return out
    if isinstance(value, list):
        return [_marker_v1(v) for v in value]
    return value


def serialise(otio: Any, obj: Any, versions: dict[str, int] | None) -> str:
    """OTIO JSON at the given schema versions (``Marker: 1`` is converted here, the rest by OTIO)."""
    versions = dict(versions or {})
    legacy_markers = versions.pop("Marker", None) == 1
    text: str = otio.adapters.otio_json.write_to_string(obj, target_schema_versions=versions or None)
    if legacy_markers:
        text = json.dumps(_marker_v1(json.loads(text)), indent=4)
    return text


def timelines_of(otio: Any, obj: Any) -> list[Any]:
    if isinstance(obj, otio.schema.Timeline):
        return [obj]
    if isinstance(obj, otio.schema.SerializableCollection):
        return [t for child in obj for t in timelines_of(otio, child)]
    if isinstance(obj, otio.schema.Track):
        # CFA4 reads a bare track as a timeline of that one track, named after it
        tl = otio.schema.Timeline(name=obj.name)
        tl.tracks.append(obj.clone())
        tl.global_start_time = None
        return [tl]
    raise TypeError(f"unexpected sample object {type(obj).__name__}")


def write(directory: Path) -> dict[str, Any]:
    otio = load_otio()
    try:
        directory.mkdir(parents=True, exist_ok=True)
    except OSError as exc:
        raise OracleError(f"{directory}: {exc}") from exc
    samples = []
    for file, (build, versions, warnings) in SAMPLES.items():
        obj = build(otio)
        text = serialise(otio, obj, versions)
        # what OTIO reads back from the file it wrote is what CFA4 must import
        read = otio.adapters.read_from_string(text)
        try:
            (directory / file).write_text(text, encoding="utf-8")
        except OSError as exc:
            raise OracleError(f"{directory / file}: {exc}") from exc
        samples.append(
            {
                "file": file,
                "timelines": [expect_timeline(otio, t) for t in timelines_of(otio, read)],
                "warnings": warnings,
            }
        )
    manifest = {
        "version": 1,
        "oracle_version": getattr(otio, "__version__", "unknown"),
        "ticks_per_second": TICKS_PER_SECOND,
        "samples": samples,
    }
    try:
        (directory / "samples.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False), encoding="utf-8")
    except OSError as exc:
        raise OracleError(f"{directory / 'samples.json'}: {exc}") from exc
    return manifest


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("directory", type=Path, help="directory to write the .otio samples and samples.json to")
    args = parser.parse_args(argv)
    try:
        manifest = write(args.directory)
    except OracleError as exc:
        print(f"otio_samples: {exc}", file=sys.stderr)
        return 2
    print(f"otio_samples: {len(manifest['samples'])} samples written to {args.directory}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
