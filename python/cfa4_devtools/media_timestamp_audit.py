"""CFA4 F7: bounded FFprobe container-level A/V timing evidence.

This does NOT measure acoustic/visual lip sync, CUDA codec use, decoded frame
content, or Qt6 NLE behavior. It never runs a GUI or modifies media.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any

MAX_PROBE_SECONDS = 30
MAX_PROBE_BYTES = 16 * 1024 * 1024
MAX_PACKETS = 15000


class MediaTimingError(ValueError):
    """FFprobe evidence is absent, malformed, or outside the configured gate."""


def seconds(value: Any, label: str) -> Decimal:
    try:
        t = Decimal(str(value))
    except (InvalidOperation, TypeError) as exc:
        raise MediaTimingError(f"{label}: missing or invalid seconds: {value!r}") from exc
    if not t.is_finite():
        raise MediaTimingError(f"{label}: non-finite time {value!r}")
    return t


def packets_for_stream(packets: list[dict[str, Any]], index: int) -> list[dict[str, Decimal]]:
    selected = []
    previous_dts = None
    for packet in packets:
        if packet.get("stream_index") != index:
            continue
        pts = seconds(packet.get("pts_time"), f"stream {index} PTS")
        dts = seconds(packet.get("dts_time"), f"stream {index} DTS")
        duration = seconds(packet.get("duration_time"), f"stream {index} packet duration")
        if duration <= 0:
            raise MediaTimingError(f"stream {index}: nonpositive packet duration")
        if previous_dts is not None and dts < previous_dts:
            raise MediaTimingError(f"stream {index}: non-monotonic decoding timestamps")
        previous_dts = dts
        selected.append({"pts": pts, "dts": dts, "duration": duration})
    if not selected:
        raise MediaTimingError(f"stream {index}: no packets in selected window")
    return selected


def timing_evidence(payload: dict[str, Any], *, max_start_ms: int = 250,
                    max_end_ms: int = 500) -> dict[str, Any]:
    """Interpret packet timestamps. Threshold PASS covers only container timing."""
    streams = payload.get("streams")
    packets = payload.get("packets")
    if not isinstance(streams, list) or not isinstance(packets, list):
        raise MediaTimingError("FFprobe must contain streams and packets arrays")
    if len(packets) > MAX_PACKETS:
        raise MediaTimingError("too many packets; limit sample to a short interval")
    candidates = {}
    for stream in streams:
        if not isinstance(stream, dict):
            raise MediaTimingError("invalid stream entry")
        kind = stream.get("codec_type")
        if kind in ("video", "audio") and kind not in candidates:
            if not isinstance(stream.get("index"), int):
                raise MediaTimingError(f"{kind}: missing stream index")
            candidates[kind] = stream
    if "video" not in candidates:
        raise MediaTimingError("no video stream; A/V evidence unavailable")
    if "audio" not in candidates:
        raise MediaTimingError("no audio stream; A/V sync cannot be measured")
    if max_start_ms < 0 or max_end_ms < 0:
        raise MediaTimingError("skew thresholds must be nonnegative")
    spans = {}
    for kind, stream in candidates.items():
        rows = packets_for_stream(packets, stream["index"])
        start = min(p["pts"] for p in rows)
        end = max(p["pts"] + p["duration"] for p in rows)
        if end <= start:
            raise MediaTimingError(f"{kind}: invalid packet presentation span")
        spans[kind] = {"index": stream["index"], "codec": stream.get("codec_name"),
                       "packets": len(rows), "start": start, "end": end,
                       "time_base": stream.get("time_base"),
                       "duration": stream.get("duration")}
    start_ms = abs(spans["audio"]["start"] - spans["video"]["start"]) * 1000
    end_ms = abs(spans["audio"]["end"] - spans["video"]["end"]) * 1000
    if start_ms > max_start_ms or end_ms > max_end_ms:
        raise MediaTimingError(
            f"container timestamp skew exceeds threshold: start={start_ms}ms "
            f"(limit={max_start_ms}), end={end_ms}ms (limit={max_end_ms})"
        )
    # No acoustic correlation or video/audio content matching was performed.
    return {
        "status": "PASS_CONTAINER_TIMESTAMPS_ONLY",
        "start_skew_ms": float(start_ms),
        "end_skew_ms": float(end_ms),
        "max_start_skew_ms": max_start_ms,
        "max_end_skew_ms": max_end_ms,
        "streams": {
            kind: {k: (str(v) if isinstance(v, Decimal) else v)
                   for k, v in values.items()}
            for kind, values in spans.items()
        },
        "not_proven": [
            "perceptual audio/video lip sync",
            "decoded frame/content fidelity",
            "CFA4 encoder/decoder or hardware execution",
            "full video length beyond the probed interval",
        ],
    }


def ffprobe_payload(path: Path, ffprobe: str, seconds_window: int) -> dict[str, Any]:
    if not 1 <= seconds_window <= MAX_PROBE_SECONDS:
        raise MediaTimingError(f"seconds window must be 1..{MAX_PROBE_SECONDS}")
    if not path.is_file():
        raise MediaTimingError(f"media file unavailable: {path}")
    command = [
        ffprobe, "-v", "error", "-show_streams", "-show_packets",
        "-show_entries",
        (
            "stream=index,codec_type,codec_name,time_base,start_time,duration,"
            "sample_rate,channels,color_primaries,color_transfer,color_space:"
            "packet=stream_index,pts_time,dts_time,duration_time"
        ),
        "-read_intervals", f"0%+{seconds_window}",
        "-of", "json", str(path),
    ]
    try:
        result = subprocess.run(command, capture_output=True, timeout=45, check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise MediaTimingError(f"FFprobe unavailable or timed out: {exc}") from exc
    if result.returncode:
        raise MediaTimingError(f"FFprobe failed: {result.stderr[-1200:].decode('utf-8', 'replace')}")
    if len(result.stdout) > MAX_PROBE_BYTES:
        raise MediaTimingError("FFprobe JSON exceeds bounded evidence size")
    try:
        payload = json.loads(result.stdout)
    except (ValueError, UnicodeDecodeError) as exc:
        raise MediaTimingError("FFprobe returned invalid JSON") from exc
    if not isinstance(payload, dict):
        raise MediaTimingError("FFprobe root is not an object")
    return payload


def main() -> int:
    parser = argparse.ArgumentParser(description="CFA4 F7 short-window A/V timestamp audit")
    parser.add_argument("media", type=Path)
    parser.add_argument("--ffprobe", default="ffprobe")
    parser.add_argument("--seconds", type=int, default=10)
    parser.add_argument("--start-limit-ms", type=int, default=250)
    parser.add_argument("--end-limit-ms", type=int, default=500)
    parser.add_argument("--output", type=Path, help="optional JSON evidence path")
    args = parser.parse_args()
    try:
        payload = ffprobe_payload(args.media, args.ffprobe, args.seconds)
        result = timing_evidence(payload, max_start_ms=args.start_limit_ms,
                                 max_end_ms=args.end_limit_ms)
    except MediaTimingError as exc:
        print(f"FAIL CFA4 F7: {exc}", file=sys.stderr)
        return 1
    result["source"] = str(args.media)
    result["window_seconds"] = args.seconds
    result["probe"] = "FFprobe packet PTS/DTS; no audio/video content correlation"
    content = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(content, encoding="utf-8")
    print(content)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
