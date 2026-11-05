"""CFA4 OTIO-1.2: OpenTimelineIO sample files for every schema CFA4 reads, written by the official library.

This is the reverse direction of ``otio_oracle`` (OTIO-1.1): the official ``opentimelineio`` package
writes the documents, and the Rust test ``oracle_samples`` imports them into CFA4
(docs/plans/OTIO-INTEGRATION-2026-10-24.md)::

    python -m cfa4_devtools.otio_samples target/fixtures/otio-samples
    CFA4_OTIO_SAMPLES_DIR=target/fixtures/otio-samples \\
        cargo test -p cfa4-interchange --test otio oracle_samples

Next to the ``.otio`` files it writes ``manifest.json``: for each file the sequences CFA4 must make
and, per track, every clip and nested sequence with the position and length that OTIO itself
computes (``range_in_parent``) in exact engine ticks, the transitions, markers, time effects and the
media each clip points at. ``warnings`` lists the report warnings the import must give (each entry is
a substring of exactly one warning); any other warning fails the test.

Covered schemas: Timeline, Stack (top level and nested), SerializableCollection, a bare Track,
Track (Video and Audio), Clip, Gap, Transition (at a cut, a fade in and a fade out), Marker (stack,
track and clip), ExternalReference (absolute ``file://`` URL, relative path, empty path),
ImageSequenceReference, GeneratorReference (solid colour, SMPTE bars, black, unknown),
MissingReference, several media references with an active key, LinearTimeWarp (speed, reverse,
hold), FreezeFrame and an unknown Effect; eight frame rates including the NTSC ones.

Like the oracle, the library is a development tool only: never imported by or shipped with CFA4.
Exit status: 0 written, 2 cannot run (``opentimelineio`` missing, unusable directory).
"""
from __future__ import annotations

import argparse
import json
import sys
from fractions import Fraction
from pathlib import Path
from typing import Any

from cfa4_devtools.otio_oracle import OracleError, load_otio

#: Engine time base (``cfa4_time::TICKS_PER_SECOND``).
TICKS_PER_SECOND = 254_016_000_000
#: Base directory the Rust test passes to the importer, for relative media paths.
BASE_DIR = "/proj"

#: (file tag, rate) for the per-rate timelines; NTSC rates as OTIO writes them (floats).
RATES: tuple[tuple[str, float], ...] = (
    ("23976", 24000 / 1001),
    ("24", 24.0),
    ("25", 25.0),
    ("2997", 30000 / 1001),
    ("30", 30.0),
    ("50", 50.0),
    ("5994", 60000 / 1001),
    ("60", 60.0),
)


def ticks(seconds: Fraction) -> int:
    """Engine ticks of an exact number of seconds, rounded to the nearest tick."""
    return round(seconds * TICKS_PER_SECOND)


def seconds_of(rational_time: Any) -> Fraction:
    rate = Fraction(rational_time.rate)
    if rate <= 0:
        raise ValueError(f"non-positive rate {rational_time.rate!r}")
    return Fraction(rational_time.value) / rate


class Samples:
    """Builds the sample documents and, from OTIO's own timing, what CFA4 must import."""

    def __init__(self, otio: Any) -> None:
        self.otio = otio
        self.schema = otio.schema
        self.opentime = otio.opentime

    # -- small builders ---------------------------------------------------------------------------

    def rt(self, value: float, rate: float) -> Any:
        return self.opentime.RationalTime(value, rate)

    def tr(self, start: float, duration: float, rate: float) -> Any:
        return self.opentime.TimeRange(self.rt(start, rate), self.rt(duration, rate))

    def external(self, url: str, rate: float, available: tuple[float, float] | None = None) -> Any:
        ref = self.schema.ExternalReference(target_url=url)
        if available is not None:
            ref.available_range = self.tr(available[0], available[1], rate)
        return ref

    def clip(self, name: str, ref: Any, start: float, duration: float, rate: float) -> Any:
        return self.schema.Clip(name=name, media_reference=ref, source_range=self.tr(start, duration, rate))

    def gap(self, duration: float, rate: float) -> Any:
        return self.schema.Gap(source_range=self.tr(0, duration, rate))

    def transition(self, name: str, in_frames: float, out_frames: float, rate: float,
                   kind: str | None = None) -> Any:
        return self.schema.Transition(
            name=name,
            transition_type=kind or self.schema.TransitionTypes.SMPTE_Dissolve,
            in_offset=self.rt(in_frames, rate),
            out_offset=self.rt(out_frames, rate),
        )

    def marker(self, name: str, at: float, rate: float, color: str = "RED", length: float = 0) -> Any:
        return self.schema.Marker(name=name, marked_range=self.tr(at, length, rate), color=color)

    def generator(self, kind: str, **parameters: Any) -> Any:
        return self.schema.GeneratorReference(generator_kind=kind, parameters=parameters)

    # -- the samples ------------------------------------------------------------------------------

    def per_rate(self, tag: str, rate: float) -> Any:
        """Every editorial schema on one timeline: gaps, three kinds of transition, markers on the
        stack, a track and a clip, absolute and relative media, a generator, a disabled clip."""
        tl = self.schema.Timeline(name=f"Rate {tag}")
        tl.global_start_time = self.rt(round(rate) * 3600, rate)
        tl.tracks.markers.append(self.marker("scene", 10, rate, "GREEN"))
        v1 = self.schema.Track(name="V1", kind=self.schema.TrackKind.Video)
        a = self.clip("shot_a", self.external("file:///footage/shot%20a.mov", rate, (1000, 500)), 1010, 48, rate)
        a.markers.append(self.marker("kingfisher", 1020, rate, "RED", 2))
        b = self.clip("shot_b", self.external("clips/shot_b.mov", rate), 0, 36, rate)
        c = self.clip("shot_c", self.external("file:///footage/shot_c.mov", rate), 7, 20, rate)
        v1.extend([
            self.gap(12, rate),
            self.transition("Fade In", 0, 6, rate),
            a,
            self.transition("", 6, 6, rate),
            b,
            self.gap(5, rate),
            c,
            self.transition("Fade Out", 4, 0, rate),
        ])
        v2 = self.schema.Track(name="Titles", kind=self.schema.TrackKind.Video)
        v2.markers.append(self.marker("title in", 30, rate, "BLUE"))
        v2.extend([self.gap(30, rate), self.clip("red", self.generator("SolidColor", color=[1.0, 0.0, 0.0, 1.0]), 0, 24, rate)])
        a1 = self.schema.Track(name="A1", kind=self.schema.TrackKind.Audio)
        off = self.clip("room tone", self.external("file:///footage/room.wav", rate), 0, 30, rate)
        off.enabled = False
        a1.extend([self.gap(12, rate), self.clip("shot_a", self.external("file:///footage/shot%20a.mov", rate, (1000, 500)), 1010, 48, rate), off])
        tl.tracks.extend([v1, v2, a1])
        return tl

    def media_references(self) -> Any:
        rate = 24.0
        tl = self.schema.Timeline(name="Media references")
        v1 = self.schema.Track(name="V1", kind=self.schema.TrackKind.Video)
        frames = self.schema.ImageSequenceReference(
            target_url_base="file:///frames/", name_prefix="shot.", name_suffix=".exr", start_frame=1,
            frame_step=1, rate=rate, frame_zero_padding=4, available_range=self.tr(0, 48, rate))
        multi = self.schema.Clip(name="multi", source_range=self.tr(0, 24, rate))
        multi.set_media_references({
            "DEFAULT_MEDIA": self.external("file:///proxy/multi.mov", rate),
            "HIGH": self.external("file:///full/multi.mov", rate),
        }, "HIGH")
        v1.extend([
            self.clip("frames", frames, 0, 48, rate),
            self.clip("bars", self.generator("SMPTEBars"), 0, 24, rate),
            self.clip("black", self.generator("black"), 0, 12, rate),
            self.clip("noise", self.generator("Noise"), 0, 12, rate),
            self.clip("missing", self.schema.MissingReference(name="lost_shot.mov"), 0, 24, rate),
            multi,
            self.clip("no path", self.external("", rate), 0, 12, rate),
        ])
        tl.tracks.append(v1)
        return tl

    def time_effects(self) -> Any:
        rate = 25.0
        tl = self.schema.Timeline(name="Time effects")
        v1 = self.schema.Track(name="V1", kind=self.schema.TrackKind.Video)
        fast = self.clip("fast", self.external("file:///f/a.mov", rate), 100, 50, rate)
        fast.effects.append(self.schema.LinearTimeWarp(time_scalar=2.0))
        back = self.clip("backwards", self.external("file:///f/b.mov", rate), 100, 25, rate)
        back.effects.append(self.schema.LinearTimeWarp(time_scalar=-1.0))
        held = self.clip("held", self.external("file:///f/c.mov", rate), 40, 25, rate)
        held.effects.append(self.schema.LinearTimeWarp(time_scalar=0.0))
        frozen = self.clip("frozen", self.external("file:///f/d.mov", rate), 60, 25, rate)
        frozen.effects.append(self.schema.FreezeFrame())
        blurred = self.clip("blurred", self.external("file:///f/e.mov", rate), 0, 25, rate)
        blurred.effects.append(self.schema.Effect(effect_name="Blur"))
        v1.extend([fast, back, held, frozen, blurred])
        tl.tracks.append(v1)
        return tl

    def nested(self) -> Any:
        rate = 24.0
        tl = self.schema.Timeline(name="Nested")
        inner = self.schema.Stack(name="inner", source_range=self.tr(12, 48, rate))
        iv = self.schema.Track(name="V1", kind=self.schema.TrackKind.Video)
        iv.append(self.clip("inside", self.external("file:///n/inside.mov", rate), 0, 72, rate))
        ia = self.schema.Track(name="A1", kind=self.schema.TrackKind.Audio)
        ia.append(self.clip("inside audio", self.external("file:///n/inside.wav", rate), 0, 72, rate))
        inner.extend([iv, ia])
        unnamed = self.schema.Stack(source_range=self.tr(0, 24, rate))
        uv = self.schema.Track(name="V1", kind=self.schema.TrackKind.Video)
        uv.append(self.clip("deep", self.external("file:///n/deep.mov", rate), 0, 24, rate))
        unnamed.append(uv)
        v1 = self.schema.Track(name="V1", kind=self.schema.TrackKind.Video)
        v1.extend([
            self.clip("before", self.external("file:///n/before.mov", rate), 0, 24, rate),
            inner,
            unnamed,
            self.clip("after", self.external("file:///n/after.mov", rate), 0, 24, rate),
        ])
        tl.tracks.append(v1)
        return tl

    def collection(self) -> Any:
        rate = 24.0
        coll = self.schema.SerializableCollection(name="Reels")
        for name in ("Reel 1", "Reel 2"):
            tl = self.schema.Timeline(name=name)
            track = self.schema.Track(name="V1", kind=self.schema.TrackKind.Video)
            track.append(self.clip(f"{name} shot", self.external(f"file:///reels/{name.replace(' ', '_')}.mov", rate), 0, 24, rate))
            tl.tracks.append(track)
            coll.append(tl)
        return coll

    def bare_track(self) -> Any:
        rate = 30.0
        track = self.schema.Track(name="Solo", kind=self.schema.TrackKind.Video)
        track.extend([self.gap(15, rate), self.clip("only", self.external("file:///solo/only.mov", rate), 0, 45, rate)])
        return track

    def clip_in_stack(self) -> Any:
        rate = 24.0
        tl = self.schema.Timeline(name="Clip in stack")
        track = self.schema.Track(name="V1", kind=self.schema.TrackKind.Video)
        track.append(self.clip("kept", self.external("file:///s/kept.mov", rate), 0, 24, rate))
        tl.tracks.extend([track, self.clip("stray", self.external("file:///s/stray.mov", rate), 0, 24, rate)])
        return tl

    def all(self) -> list[tuple[str, Any, list[str]]]:
        """(file name, document, expected warning substrings)."""
        out: list[tuple[str, Any, list[str]]] = [(f"rate_{tag}.otio", self.per_rate(tag, rate), []) for tag, rate in RATES]
        out += [
            ("media_references.otio", self.media_references(), [
                'OTIO generator "noise" is not supported',
                "OTIO media reference without a path",
            ]),
            ("time_effects.otio", self.time_effects(), ['OTIO effect "Blur" is not supported']),
            ("nested.otio", self.nested(), []),
            ("collection.otio", self.collection(), []),
            ("bare_track.otio", self.bare_track(), []),
            ("clip_in_stack.otio", self.clip_in_stack(), ["OTIO Clip directly in a stack is not supported"]),
        ]
        return out

    # -- what CFA4 must import, computed by OTIO --------------------------------------------------

    def expect_media(self, clip: Any) -> dict[str, Any]:
        ref = clip.media_reference  # the active reference
        s = self.schema
        if isinstance(ref, s.ExternalReference) and ref.target_url.strip():
            return {"ref": "file", "path": file_path(ref.target_url), "offline": False}
        if isinstance(ref, s.ImageSequenceReference) and ref.target_url_base.strip():
            # How CFA4 keeps an image sequence is OTIO-2.3; here only that it is a file reference.
            return {"ref": "file", "path": None, "offline": False}
        if isinstance(ref, s.GeneratorReference):
            known = {"solidcolor": "ColorMatte", "solid_color": "ColorMatte", "smptebars": "BarsAndTone",
                     "bars": "BarsAndTone", "black": "BlackVideo"}
            return {"ref": "generator", "generator": known.get(ref.generator_kind.lower(), "BlackVideo"), "offline": False}
        return {"ref": "file", "path": None, "offline": True}

    def expect_track(self, track: Any) -> dict[str, Any]:
        items: list[dict[str, Any]] = []
        transitions: list[dict[str, Any]] = []
        children = list(track)
        pos = Fraction(0)
        for index, child in enumerate(children):
            if isinstance(child, self.schema.Transition):
                inn, out = seconds_of(child.in_offset), seconds_of(child.out_offset)
                prev_clip = index > 0 and isinstance(children[index - 1], (self.schema.Clip, self.schema.Stack))
                next_clip = index + 1 < len(children) and isinstance(children[index + 1], (self.schema.Clip, self.schema.Stack))
                if prev_clip or next_clip:
                    transitions.append({"start_ticks": ticks(pos - inn), "duration_ticks": ticks(inn + out)})
                continue
            placed = child.range_in_parent()
            start, duration = seconds_of(placed.start_time), seconds_of(placed.duration)
            if start != pos:
                raise ValueError(f"{track.name}: OTIO places {child.name!r} at {start}, expected {pos}")
            pos = start + duration
            if isinstance(child, self.schema.Gap):
                continue
            entry: dict[str, Any] = {
                "name": child.name or ("Clip" if isinstance(child, self.schema.Clip) else "Nested Sequence"),
                "start_ticks": ticks(start),
                "duration_ticks": ticks(duration),
                "enabled": bool(child.enabled),
            }
            if isinstance(child, self.schema.Stack):
                entry["media"] = {"ref": "sequence", "offline": False}
                entry["markers"] = []
            else:
                entry["media"] = self.expect_media(child)
                avail = child.media_reference.available_range if child.media_reference is not None else None
                origin = seconds_of(avail.start_time) if avail is not None else Fraction(0)
                entry["markers"] = [{"name": m.name, "start_ticks": ticks(seconds_of(m.marked_range.start_time) - origin)} for m in child.markers]
                entry.update(self.expect_time_effects(child))
            items.append(entry)
        kind = "Audio" if track.kind == self.schema.TrackKind.Audio else "Video"
        return {"kind": kind, "name": track.name, "items": items, "transitions": transitions}

    def expect_time_effects(self, clip: Any) -> dict[str, Any]:
        speed, reverse, hold = 1.0, False, False
        for effect in clip.effects:
            if isinstance(effect, self.schema.FreezeFrame):
                hold = True
            elif isinstance(effect, self.schema.LinearTimeWarp):
                if effect.time_scalar == 0:
                    hold = True
                else:
                    speed, reverse = abs(effect.time_scalar), effect.time_scalar < 0
        return {"speed": speed, "reverse": reverse, "frame_hold": hold}

    def expect_sequence(self, name: str, stack: Any, global_start: Any | None) -> dict[str, Any]:
        tracks = [t for t in stack if isinstance(t, self.schema.Track)]
        markers = [{"name": m.name, "start_ticks": ticks(seconds_of(m.marked_range.start_time))} for m in stack.markers]
        for t in tracks:
            markers += [{"name": m.name, "start_ticks": ticks(seconds_of(m.marked_range.start_time))} for m in t.markers]
        markers.sort(key=lambda m: m["start_ticks"])
        rate = global_start.rate if global_start is not None else first_rate(self.schema, stack)
        return {
            "name": name,
            "rate": rate,
            "start_timecode": round(global_start.value) if global_start is not None else 0,
            "markers": markers,
            "tracks": [self.expect_track(t) for t in tracks],
        }

    def expect(self, document: Any) -> list[dict[str, Any]]:
        s = self.schema
        if isinstance(document, s.SerializableCollection):
            return [seq for child in document for seq in self.expect(child)]
        if isinstance(document, s.Timeline):
            return [self.expect_sequence(document.name or "Timeline", document.tracks, document.global_start_time)]
        if isinstance(document, s.Track):
            stack = s.Stack()
            stack.append(document.clone())
            return [self.expect_sequence(document.name or "Timeline", stack, None)]
        raise TypeError(f"no sample expectation for {type(document).__name__}")


def first_rate(schema: Any, composition: Any) -> float:
    """The rate CFA4 falls back to: the first ``source_range`` duration found, depth first."""
    for child in composition:
        if child.source_range is not None:
            return child.source_range.duration.rate
        if isinstance(child, (schema.Track, schema.Stack)):
            try:
                return first_rate(schema, child)
            except LookupError:
                continue
    raise LookupError("no source_range")


def file_path(url: str) -> str:
    """The path CFA4 opens for a media URL: ``file://`` URLs decoded, relative paths under BASE_DIR."""
    from urllib.parse import unquote, urlparse

    if url.startswith("file://"):
        return unquote(urlparse(url).path)
    if url.startswith("/"):
        return url
    return f"{BASE_DIR}/{url}"


def write(directory: Path) -> dict[str, Any]:
    otio = load_otio()
    samples = Samples(otio)
    try:
        directory.mkdir(parents=True, exist_ok=True)
    except OSError as exc:
        raise OracleError(f"{directory}: {exc}") from exc
    entries = []
    for name, document, warnings in samples.all():
        otio.adapters.write_to_file(document, str(directory / name))
        entries.append({"file": name, "warnings": warnings, "sequences": samples.expect(document)})
    manifest = {
        "version": 1,
        "kind": "otio-samples",
        "oracle": "opentimelineio",
        "oracle_version": getattr(otio, "__version__", "unknown"),
        "ticks_per_second": TICKS_PER_SECOND,
        "base_dir": BASE_DIR,
        "samples": entries,
    }
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False), encoding="utf-8")
    return manifest


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("directory", type=Path, help="directory to write manifest.json and the .otio samples to")
    args = parser.parse_args(argv)
    try:
        manifest = write(args.directory)
    except OracleError as exc:
        print(f"otio_samples: {exc}", file=sys.stderr)
        return 2
    print(f"otio_samples: wrote {len(manifest['samples'])} samples to {args.directory} "
          f"(opentimelineio {manifest['oracle_version']})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
