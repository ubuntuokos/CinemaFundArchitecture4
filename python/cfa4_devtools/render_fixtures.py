"""CFA4-RENDER-INTEGRATION-PLAN-001 F0V: reference fixture generator and verifier.

The plan's reference package (SDR 1080p, DCI-class 4K 12-bit, PQ 4K30 10-bit, VFR, alpha, EXR
sequence, 5.1 / 7.1 audio, flash-and-beep sync, and the four retouching fixtures) is generated
procedurally with ``ffmpeg`` lavfi sources, so the media is original work and is never committed
(``docs/testing.md``). This module follows the existing fixture convention of ``crates/testkit``:
files go to ``<workspace>/target/fixtures/render-f0v/`` (``CFA4_FIXTURES_DIR`` overrides the root)
and ``CFA4_FFMPEG`` / ``CFA4_FFPROBE`` select the tools.

The committed manifest (``docs/audit/render-f0v/F0V-fixtures-manifest.json``) holds, per fixture,
the generator arguments and the *expected stream properties*. Byte hashes are deliberately not
committed: they change with the encoder build. ``generate`` writes the hashes of this machine to
``MANIFEST.local.json`` next to the files so a report can quote them.

    python3 python/cfa4_devtools/render_fixtures.py list
    python3 python/cfa4_devtools/render_fixtures.py generate [name ...]
    python3 python/cfa4_devtools/render_fixtures.py verify          # properties vs. manifest
    python3 python/cfa4_devtools/render_fixtures.py write-manifest   # regenerate the committed file

A fixture whose encoder is missing is reported as ``SKIPPED`` (never PASS); ``CFA4_REQUIRE_ORACLES=1``
turns a skip into a failure, like the Rust oracles. Standard library only.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

SCHEMA = "cfa4.render-fixtures/1"
ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "docs" / "audit" / "render-f0v" / "F0V-fixtures-manifest.json"
SOURCE = "procedural (ffmpeg lavfi), original work, generated locally, never committed"
LICENCE = "CFA4 original (GPL-3.0-or-later); no third-party media"


@dataclass(frozen=True)
class Fixture:
    name: str
    output: str  # file name, or a ``%04d`` pattern for a sequence
    purpose: str
    args: tuple[str, ...]  # everything between ``-y`` and the output path
    expect: dict[str, Any] = field(default_factory=dict)  # ffprobe field -> value
    needs_encoder: str = ""
    acs: tuple[str, ...] = ()
    gap: str = ""  # an honest limit of this stand-in


def _v(**kw: Any) -> dict[str, Any]:
    return {f"v.{k}": v for k, v in kw.items()}


def _a(**kw: Any) -> dict[str, Any]:
    return {f"a.{k}": v for k, v in kw.items()}


FIXTURES: tuple[Fixture, ...] = (
    Fixture(
        "sdr_1080p25",
        "sdr_1080p25.mp4",
        "1080p25 8-bit SDR baseline",
        (
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1920x1080:r=25:d=2",
            "-c:v",
            "libx264",
            "-crf",
            "16",
            "-pix_fmt",
            "yuv420p",
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
        ),
        _v(codec_name="h264", width=1920, height=1080, pix_fmt="yuv420p", r_frame_rate="25/1", nb_frames=50),
        "libx264",
        ("AC-02", "AC-04", "AC-16"),
    ),
    Fixture(
        "dci_4k24_12bit_p3",
        "dci_4k24_12bit_p3.mkv",
        "4K DCI 24 fps 12-bit, DCI-P3 tagged (RGB lossless)",
        (
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=4096x2160:r=24:d=1,format=gbrp12le",
            "-c:v",
            "ffv1",
            "-level",
            "3",
            "-color_primaries",
            "smpte432",
            "-color_trc",
            "smpte428",
        ),
        _v(codec_name="ffv1", width=4096, height=2160, pix_fmt="gbrp12le", r_frame_rate="24/1"),
        "ffv1",
        ("AC-04", "AC-18"),
        "RGB 12-bit stand-in; the X'Y'Z' J2K master and the J2K oracle belong to CINECERT and stay BLOCKED/PENDING (AC-12)",
    ),
    Fixture(
        "pq_4k30_10bit",
        "pq_4k30_10bit.mp4",
        "4K30 10-bit PQ (SMPTE ST 2084), BT.2020",
        (
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=3840x2160:r=30:d=1,format=yuv420p10le",
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
            "-crf",
            "18",
            "-x265-params",
            "log-level=error",
            "-colorspace",
            "bt2020nc",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "smpte2084",
        ),
        _v(
            codec_name="hevc",
            width=3840,
            height=2160,
            pix_fmt="yuv420p10le",
            r_frame_rate="30/1",
            color_transfer="smpte2084",
        ),
        "libx265",
        ("AC-04", "AC-18"),
    ),
    Fixture(
        "vfr_synthetic",
        "vfr_synthetic.mkv",
        "variable frame rate with jittered timestamps",
        (
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1280x720:r=50:d=2,setpts='(N*0.04+0.012*sin(N*1.3))/TB'",
            "-fps_mode",
            "passthrough",
            "-c:v",
            "libx264",
            "-crf",
            "20",
            "-pix_fmt",
            "yuv420p",
        ),
        _v(codec_name="h264", width=1280, height=720),
        "libx264",
        ("AC-05", "AC-12"),
        "synthetic stand-in; the real phone clip stays BLOCKED/PENDING (AC-12)",
    ),
    Fixture(
        "alpha_prores4444",
        "alpha_prores4444.mov",
        "ProRes 4444 with a half-transparent right half (ffprobe reports the decoder pixel format yuva444p12le)",
        (
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=25:d=1,format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='if(lt(X,W/2),255,96)',format=yuva444p10le",
            "-c:v",
            "prores_ks",
            "-profile:v",
            "4444",
            "-alpha_bits",
            "16",
        ),
        _v(codec_name="prores", width=640, height=360, pix_fmt="yuva444p12le"),
        "prores_ks",
        ("AC-04",),
    ),
    Fixture(
        "exr_sequence",
        "exr_sequence_%04d.exr",
        "16-frame OpenEXR sequence, 32-bit float planar RGB",
        (
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=24:d=1,format=gbrpf32le",
            "-frames:v",
            "16",
            "-c:v",
            "exr",
            "-pix_fmt",
            "gbrpf32le",
        ),
        {"frames": 16},
        "exr",
        ("AC-04", "AC-14"),
    ),
    Fixture(
        "audio_51_24bit",
        "audio_51_24bit.wav",
        "5.1, 24-bit / 48 kHz, one tone per channel",
        (
            "-f",
            "lavfi",
            "-i",
            "sine=f=220:r=48000:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=330:r=48000:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:r=48000:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=550:r=48000:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=660:r=48000:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=770:r=48000:d=2",
            "-filter_complex",
            "[0][1][2][3][4][5]amerge=inputs=6,pan=5.1|FL=c0|FR=c1|FC=c2|LFE=c3|BL=c4|BR=c5",
            "-c:a",
            "pcm_s24le",
        ),
        _a(codec_name="pcm_s24le", sample_rate="48000", channels=6, channel_layout="5.1"),
        "pcm_s24le",
        ("AC-13",),
    ),
    Fixture(
        "audio_71_24bit",
        "audio_71_24bit.wav",
        "7.1, 24-bit / 48 kHz, one tone per channel",
        (
            "-f",
            "lavfi",
            "-i",
            "sine=f=220:r=48000:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=330:r=48000:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:r=48000:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=550:r=48000:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=660:r=48000:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=770:r=48000:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=880:r=48000:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=990:r=48000:d=2",
            "-filter_complex",
            "[0][1][2][3][4][5][6][7]amerge=inputs=8,pan=7.1|FL=c0|FR=c1|FC=c2|LFE=c3|BL=c4|BR=c5|SL=c6|SR=c7",
            "-c:a",
            "pcm_s24le",
        ),
        _a(codec_name="pcm_s24le", sample_rate="48000", channels=8, channel_layout="7.1"),
        "pcm_s24le",
        ("AC-13",),
    ),
    Fixture(
        "sync_flash_beep",
        "sync_flash_beep.mkv",
        "white flash on frame 0 of every second; 1 kHz, 10 ms beep at the same instants",
        (
            "-f",
            "lavfi",
            "-i",
            "color=black:s=640x360:r=25:d=4,format=yuv420p,geq=lum='if(eq(mod(N,25),0),235,16)':cb=128:cr=128",
            "-f",
            "lavfi",
            "-i",
            "aevalsrc='if(lt(mod(t,1),0.01),sin(2*PI*1000*t),0)':s=48000:d=4",
            "-c:v",
            "libx264",
            "-crf",
            "12",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "pcm_s24le",
        ),
        {**_v(codec_name="h264", width=640, height=360, r_frame_rate="25/1"), **_a(sample_rate="48000")},
        "libx264",
        ("AC-05",),
    ),
    Fixture(
        "retouch_interlaced",
        "retouch_interlaced.mkv",
        "interlaced, top field first, 50 fields/s",
        (
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=720x576:r=50:d=2,interlace=scan=tff",
            "-c:v",
            "libx264",
            "-flags",
            "+ilme+ildct",
            "-crf",
            "16",
            "-pix_fmt",
            "yuv420p",
        ),
        _v(codec_name="h264", width=720, height=576),
        "libx264",
        ("AC-04",),
    ),
    Fixture(
        "retouch_telecined",
        "retouch_telecined.mkv",
        "24p film material telecined with the 2:3 pattern",
        (
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=720x480:r=24000/1001:d=2,telecine=pattern=23",
            "-c:v",
            "libx264",
            "-flags",
            "+ilme+ildct",
            "-crf",
            "16",
            "-pix_fmt",
            "yuv420p",
        ),
        _v(codec_name="h264", width=720, height=480),
        "libx264",
        ("AC-04",),
    ),
    Fixture(
        "retouch_noisy",
        "retouch_noisy.mkv",
        "temporal and spatial noise on a test pattern",
        (
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1280x720:r=25:d=2,noise=alls=40:allf=t+u",
            "-c:v",
            "libx264",
            "-crf",
            "10",
            "-pix_fmt",
            "yuv420p",
        ),
        _v(codec_name="h264", width=1280, height=720),
        "libx264",
        ("AC-04",),
    ),
    Fixture(
        "retouch_banded",
        "retouch_banded.mkv",
        "a slow 8-bit gradient that shows banding",
        (
            "-f",
            "lavfi",
            "-i",
            "gradients=s=1280x720:r=25:d=2:c0=0x202020:c1=0x484848:nb_colors=2:speed=0.0001,format=yuv420p",
            "-c:v",
            "libx264",
            "-crf",
            "8",
            "-pix_fmt",
            "yuv420p",
        ),
        _v(codec_name="h264", width=1280, height=720),
        "libx264",
        ("AC-04",),
    ),
)


def fixtures_root() -> Path:
    env = os.environ.get("CFA4_FIXTURES_DIR")
    return (Path(env) if env else ROOT / "target" / "fixtures") / "render-f0v"


def tool(name: str) -> str | None:
    override = os.environ.get(f"CFA4_{name.upper()}")
    if override:
        return override if Path(override).is_file() else None
    return shutil.which(name)


def has_encoder(ffmpeg: str, encoder: str) -> bool:
    if not encoder:
        return True
    done = subprocess.run([ffmpeg, "-hide_banner", "-encoders"], capture_output=True, text=True, check=False)
    return any(line.split()[1:2] == [encoder] for line in done.stdout.splitlines() if line.startswith(" "))


def probe(ffprobe: str, path: Path) -> dict[str, Any]:
    done = subprocess.run(
        [ffprobe, "-v", "error", "-show_streams", "-of", "json", str(path)], capture_output=True, text=True, check=True
    )
    streams = json.loads(done.stdout).get("streams", [])
    out: dict[str, Any] = {}
    for kind, prefix in (("video", "v"), ("audio", "a")):
        first = next((s for s in streams if s.get("codec_type") == kind), None)
        if first:
            out.update({f"{prefix}.{k}": v for k, v in first.items()})
    return out


def is_sequence(fx: Fixture) -> bool:
    return "%04d" in fx.output


def generate_one(fx: Fixture, ffmpeg: str, root: Path) -> str:
    if not has_encoder(ffmpeg, fx.needs_encoder):
        return "SKIPPED"
    target = root / fx.output
    if is_sequence(fx):  # a stale longer sequence must not survive a regeneration
        for old in root.glob(fx.output.replace("%04d", "[0-9][0-9][0-9][0-9]")):
            old.unlink()
    cmd = [ffmpeg, "-hide_banner", "-loglevel", "error", "-y", *fx.args, str(target)]
    done = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if done.returncode != 0:
        sys.stderr.write(f"{fx.name}: ffmpeg failed: {done.stderr.strip()[:400]}\n")
        return "FAIL"
    return "OK"


def check_one(fx: Fixture, ffprobe: str, root: Path) -> list[str]:
    problems: list[str] = []
    if is_sequence(fx):
        files = sorted(root.glob(fx.output.replace("%04d", "[0-9][0-9][0-9][0-9]")))
        if len(files) != fx.expect.get("frames"):
            problems.append(f"frames: expected {fx.expect.get('frames')}, found {len(files)}")
        return problems
    path = root / fx.output
    if not path.is_file():
        return ["missing"]
    actual = probe(ffprobe, path)
    for key, want in fx.expect.items():
        got = actual.get(key)
        if str(got) != str(want):
            problems.append(f"{key}: expected {want}, got {got}")
    return problems


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def manifest() -> dict[str, Any]:
    return {
        "schema": SCHEMA,
        "source": SOURCE,
        "licence": LICENCE,
        "location": "target/fixtures/render-f0v/ (CFA4_FIXTURES_DIR overrides the root); never committed",
        "hashes": "per machine in MANIFEST.local.json after `generate`; not committed because they depend on the encoder build",
        "fixtures": [
            {
                "name": f.name,
                "output": f.output,
                "purpose": f.purpose,
                "acs": list(f.acs),
                "needs_encoder": f.needs_encoder or None,
                "expect": f.expect,
                "limit": f.gap or None,
                "ffmpeg_args": list(f.args),
            }
            for f in FIXTURES
        ],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else None)
    parser.add_argument("command", choices=["list", "generate", "verify", "write-manifest"])
    parser.add_argument("names", nargs="*")
    args = parser.parse_args(argv)
    chosen = [f for f in FIXTURES if not args.names or f.name in args.names]
    if args.command == "list":
        for f in chosen:
            print(f"{f.name:22} {f.output:30} {', '.join(f.acs):20} {f.purpose}")
        return 0
    if args.command == "write-manifest":
        MANIFEST.parent.mkdir(parents=True, exist_ok=True)
        MANIFEST.write_text(json.dumps(manifest(), indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
        print(MANIFEST.relative_to(ROOT))
        return 0
    ffmpeg, ffprobe = tool("ffmpeg"), tool("ffprobe")
    require = os.environ.get("CFA4_REQUIRE_ORACLES") == "1"
    if not ffmpeg or not ffprobe:
        print("SKIPPED: ffmpeg/ffprobe not found")
        return 1 if require else 0
    root = fixtures_root()
    root.mkdir(parents=True, exist_ok=True)
    bad = False
    local: dict[str, str] = {}
    for fx in chosen:
        if args.command == "generate":
            status = generate_one(fx, ffmpeg, root)
            if status == "OK":
                problems = check_one(fx, ffprobe, root)
                status = "OK" if not problems else "FAIL " + "; ".join(problems)
                if not is_sequence(fx) and (root / fx.output).is_file():
                    local[fx.name] = sha256_of(root / fx.output)
        else:
            if not has_encoder(ffmpeg, fx.needs_encoder):
                status = "SKIPPED"
            else:
                problems = check_one(fx, ffprobe, root)
                status = "OK" if not problems else "FAIL " + "; ".join(problems)
        print(f"{status:8} {fx.name}")
        bad |= status.startswith("FAIL") or (require and status == "SKIPPED")
    if args.command == "generate" and local:
        (root / "MANIFEST.local.json").write_text(json.dumps(local, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
