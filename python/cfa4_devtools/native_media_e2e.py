"""R0 #16 Linux/AMD64 native CFA4 editor E2E audit.

This drives the real Rust/egui editor through its LOCAL loopback control server.
The PySide6 Qt6 shell is smoke-tested separately; it is NOT the NLE.
No downloads, package installs, automatic compilation or cloud calls.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import socket
import subprocess
import sys
import time
from datetime import UTC, datetime
from pathlib import Path
from typing import Any


def run(argv: list[str], *, timeout: int = 120, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(argv, text=True, capture_output=True, timeout=timeout, check=False, env=env)


def parse_command_json(proc: subprocess.CompletedProcess[str], what: str) -> dict[str, Any]:
    if proc.returncode:
        raise RuntimeError(f"{what}: exit={proc.returncode}; stderr={proc.stderr[-2500:]}")
    try:
        value = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"{what}: non-JSON output: {proc.stdout[-1200:]}") from exc
    if not isinstance(value, dict):
        raise TypeError(f"{what}: expected JSON object, got {type(value).__name__}")
    return value


def check_ffprobe(ffprobe: str, path: Path, codec: str, *, frames: int = 8) -> dict[str, Any]:
    cmd = [
        ffprobe, "-v", "error", "-count_frames", "-select_streams", "v:0",
        "-show_entries", "stream=codec_name,width,height,nb_read_frames",
        "-of", "json", str(path),
    ]
    result = parse_command_json(run(cmd, timeout=30), f"ffprobe {path.name}")
    streams = result.get("streams", [])
    if len(streams) != 1 or not isinstance(streams[0], dict):
        raise RuntimeError(f"{path.name}: expected exactly one video stream: {streams}")
    stream = streams[0]
    actual = (stream.get("codec_name"), stream.get("width"), stream.get("height"))
    if actual != (codec, 640, 360):
        raise RuntimeError(f"{path.name}: unexpected codec/size {actual}; expected {(codec, 640, 360)}")
    if int(stream.get("nb_read_frames", -1)) != frames:
        raise RuntimeError(f"{path.name}: expected {frames} decodable frames, got {stream.get('nb_read_frames')}")
    return stream


def reserve_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as server:
        server.bind(("127.0.0.1", 0))
        return int(server.getsockname()[1])


def await_control(port: int, process: subprocess.Popen[Any], timeout: float = 60.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"native editor exited before control server came up (code {process.returncode})")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.3):
                return
        except OSError:
            time.sleep(0.25)
    raise RuntimeError(f"native editor did not open its loopback control port within {timeout:g} seconds")


def bridge(cli: Path, port: int, command: str, params: dict[str, Any], *, timeout: int = 120) -> dict[str, Any]:
    proc = run(
        [str(cli), "--bridge", f"127.0.0.1:{port}", "exec", command, json.dumps(params, separators=(",", ":"))],
        timeout=timeout,
        env=dict(os.environ, CFA4_CLI_TRACE="1"),
    )
    return parse_command_json(proc, f"native editor command {command}")


def hardware_stats(perf: dict[str, Any]) -> tuple[int, int, int]:
    return (
        int(perf["decode"]["hardware"]["frames"]),
        int(perf["export"]["hardware"]["frames"]),
        int(perf["export"]["hardware"]["sessions"]),
    )


def preflight(cli: Path, editor: Path, sources: Path, out: Path, ffprobe: str) -> dict[str, Any]:
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise RuntimeError("R0 #16 is strictly Linux AMD64")
    os_release = Path("/etc/os-release").read_text(encoding="utf-8")
    if 'ID=ubuntu' not in os_release or 'VERSION_ID="26.04"' not in os_release and "VERSION_ID=26.04" not in os_release:
        raise RuntimeError("Ubuntu 26.04 is required for the authoritative R0 test")
    if os.environ.get("XDG_SESSION_TYPE") != "wayland":
        raise RuntimeError("native GUI acceptance requires an actual Wayland session")
    if not os.environ.get("WAYLAND_DISPLAY"):
        raise RuntimeError("WAYLAND_DISPLAY missing: headless Qt/GUI smoke is not a real Wayland test")
    if sys.version_info[:2] != (3, 14):
        raise RuntimeError(f"R0 Python must be 3.14, observed {sys.version.split()[0]}")
    rustc = run(["rustc", "--version"], timeout=10)
    if rustc.returncode or not rustc.stdout.split() or not rustc.stdout.split()[1].startswith("1.95."):
        raise RuntimeError(f"R0 Rust toolchain must be 1.95.x, observed {rustc.stdout.strip() or rustc.stderr.strip()}")
    for name, path in (("cfa4-cli", cli), ("cfa4 editor", editor)):
        if not path.is_file() or not os.access(path, os.X_OK):
            raise RuntimeError(f"{name} binary missing: {path}; build only these two binaries once if needed")
    for source in ("h264.mp4", "hevc.mp4", "av1-software.mkv"):
        if not (sources / source).is_file():
            raise RuntimeError(f"required existing FFmpeg #14 fixture missing: {sources / source}")
    nv = run(["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"], timeout=15)
    if nv.returncode or "RTX A1000" not in nv.stdout:
        raise RuntimeError(f"missing physical NVIDIA RTX A1000: {nv.stderr}")
    ff = run(["ffmpeg", "-version"], timeout=10)
    if ff.returncode or "ffmpeg version 9.0.2" not in ff.stdout.splitlines()[0]:
        raise RuntimeError("the FFmpeg 9.0.2 oracle executable is required")
    probe = run([ffprobe, "-version"], timeout=10)
    if probe.returncode or "ffprobe version 9.0.2" not in probe.stdout.splitlines()[0]:
        raise RuntimeError("ffprobe 9.0.2 is required")
    import_path = Path(__file__).resolve().parents[2]
    qt_env = dict(os.environ, PYTHONPATH=str(import_path / "python"), QT_QPA_PLATFORM="wayland")
    qt_versions = run(
        [sys.executable, "-c", "import PySide6; from PySide6.QtCore import qVersion; print(PySide6.__version__); print(qVersion())"],
        timeout=12,
    )
    if qt_versions.returncode:
        raise RuntimeError(f"PySide6/Qt6 Python binding unavailable: {qt_versions.stderr[-1500:]}")
    versions_qt = qt_versions.stdout.strip().splitlines()
    if len(versions_qt) != 2 or any(tuple(int(x) for x in v.split(".")[:2]) < (6, 10) for v in versions_qt):
        raise RuntimeError(f"R0 requires PySide6 and Qt >=6.10: {versions_qt}")
    qt = run([sys.executable, "-m", "cfa4_devtools.qt_app", "--smoke", "--engine", str(cli)], timeout=35, env=qt_env)
    if qt.returncode:
        raise RuntimeError(f"experimental Qt6 shell did not launch on Wayland: {qt.stderr[-1500:]}")
    out.mkdir(parents=True, exist_ok=True)
    # Keep reproducible build/runtime metadata without assuming the FFmpeg CLI
    # and CFA4 Rust binaries are linked to identical libav* libraries.
    configuration = run(["ffmpeg", "-buildconf"], timeout=12)
    if configuration.returncode:
        raise RuntimeError(f"FFmpeg buildconf retrieval failed: {configuration.stderr}")
    (out / "ffmpeg-buildconf.txt").write_text(configuration.stdout, encoding="utf-8")
    linkage = {}
    for name, binary in (("editor", editor), ("cli", cli)):
        linked = run(["ldd", str(binary)], timeout=12)
        linkage[name] = {"exit": linked.returncode, "stdout": linked.stdout, "stderr": linked.stderr}
    (out / "runtime-linkage.json").write_text(json.dumps(linkage, indent=2), encoding="utf-8")
    sha = run(["git", "rev-parse", "HEAD"], timeout=8)
    if sha.returncode:
        raise RuntimeError("cannot identify the exact CFA4 Git commit")
    versions = {
        "gitSha": sha.stdout.strip(),
        "python": sys.version.split()[0],
        "rustc": rustc.stdout.strip(),
        "session": "wayland",
        "nvidia": nv.stdout.strip(),
        "ffmpeg": ff.stdout.splitlines()[0],
        "ffprobe": probe.stdout.splitlines()[0],
        "qtShell": "PASS: experimental shell only – not a complete Qt6 NLE",
        "qt": {"pyside6": versions_qt[0], "qt": versions_qt[1]},
        "libavRuntimeLinkage": "RECORDED_NOT_YET_RECONCILED: inspect runtime-linkage.json vs ffmpeg-buildconf.txt",
        "appFrontend": "Rust/egui native editor (not Qt6)",
    }
    return versions


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sources", type=Path, default=Path("target/audit/ffmpeg/20261022T153037Z"))
    parser.add_argument("--cli", type=Path, default=Path("target/release/cfa4-cli"))
    parser.add_argument("--editor", type=Path, default=Path("target/release/cfa4"))
    parser.add_argument("--output", type=Path, default=None)
    args = parser.parse_args(argv)
    root = Path(__file__).resolve().parents[2]
    sources = (root / args.sources).resolve()
    cli = (root / args.cli).resolve()
    editor = (root / args.editor).resolve()
    stamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
    out = (root / (args.output or Path("target/audit/native-media") / stamp)).resolve()
    out.mkdir(parents=True, exist_ok=True)
    evidence: dict[str, Any] = {
        "audit": "CFA4 R0 issue 16 native Rust/egui editor E2E",
        "result": "FAIL",
        "checks": {},
        "mediaSources": str(sources),
        "output": str(out),
    }

    def checkpoint(name: str, value: Any) -> None:
        evidence["checks"][name] = value
        (out / "results.json").write_text(json.dumps(evidence, indent=2, ensure_ascii=False), encoding="utf-8")

    try:
        versions = preflight(cli, editor, sources, out, shutil.which("ffprobe") or "ffprobe")
        checkpoint("preflight", versions)
        for name, codec in (("h264.mp4", "h264"), ("hevc.mp4", "hevc"), ("av1-software.mkv", "av1")):
            checkpoint(f"input_{codec}", check_ffprobe("ffprobe", sources / name, codec))
        # Own project/data directory: never touch the user's actual CFA4 profile.
        port = reserve_port()
        editor_env = dict(os.environ, WGPU_BACKEND="vulkan", CFA4_CONTROL_TRACE="1")
        with (out / "native-editor.log").open("w", encoding="utf-8") as log:
            proc = subprocess.Popen(
                [str(editor), "--empty", "--no-recover", "--control", str(port), "--data-dir", str(out / "data")],
                stdout=log, stderr=subprocess.STDOUT, env=editor_env,
            )
            try:
                await_control(port, proc)
                checkpoint("native_control_port", "PASS: loopback listener open; UI responsiveness not yet verified")
                checkpoint("ui_control_readiness", {"status": "RUNNING", "method": "perf.stats"})
                before_payload = bridge(cli, port, "perf.stats", {})
                before = hardware_stats(before_payload)
                checkpoint("ui_control_readiness", {"status": "PASS", "method": "perf.stats"})
                checkpoint("native_window_control", "PASS: native Rust/egui Wayland editor answered a control request")
                items = []
                for source_name, source_codec in (("h264.mp4", "h264"), ("hevc.mp4", "hevc"), ("av1-software.mkv", "av1")):
                    source = sources / source_name
                    # Persist the active path before the bridge call, so a stalled
                    # import is identifiable in results.json after the timeout.
                    checkpoint(f"import_{source_codec}", {"status": "RUNNING", "path": str(source)})
                    imported_one = bridge(cli, port, "file.import", {"paths": [str(source)]})
                    checkpoint(f"import_{source_codec}", imported_one)
                    if imported_one.get("errors"):
                        raise RuntimeError(f"{source_codec}: native CFA4 importer returned errors: {imported_one['errors']}")
                    imported_items = imported_one.get("items")
                    if not isinstance(imported_items, list) or len(imported_items) != 1 or not isinstance(imported_items[0], int):
                        raise RuntimeError(f"{source_codec}: expected one imported media item: {imported_one}")
                    items.extend(imported_items)
                imported = {"items": items, "errors": []}
                checkpoint("import", imported)
                for source_codec, item, output_codec in (
                    ("h264", items[0], "h264"),
                    ("hevc", items[1], "hevc"),
                ):
                    seq = bridge(cli, port, "file.newSequenceFromClip", {"items": [item]})
                    checkpoint(f"sequence_{source_codec}", seq)
                    if not isinstance(seq.get("sequence"), int):
                        raise TypeError(f"{source_codec}: native sequence not created")
                    path = out / f"cfa4-editor-{source_codec}-to-{output_codec}.mp4"
                    result = bridge(
                        cli, port, "file.exportMedia",
                        {"path": str(path), "format": output_codec, "range": "entire",
                         "hardwareEncoding": "auto", "audio": False, "wait": True},
                        timeout=120,
                    )
                    checkpoint(f"export_{output_codec}", result)
                    checkpoint(f"probe_export_{output_codec}", check_ffprobe("ffprobe", path, output_codec))
                    # Reimport from CFA4, not just the external ffprobe oracle.
                    reimport = bridge(cli, port, "file.import", {"paths": [str(path)]})
                    checkpoint(f"reimport_{output_codec}", reimport)
                    if reimport.get("errors") or len(reimport.get("items", [])) != 1:
                        raise RuntimeError(f"{output_codec}: CFA4 cannot reimport its own NVENC export")
                project = out / "native-roundtrip.fcproj"
                checkpoint("save", bridge(cli, port, "file.saveAs", {"path": str(project)}))
                if not project.is_file():
                    raise RuntimeError("native CFA4 project save did not write a file")
                checkpoint("reopen", bridge(cli, port, "file.open", {"path": str(project)}))
                checkpoint("inspect", bridge(cli, port, "sequence.inspect", {}))
                after = hardware_stats(bridge(cli, port, "perf.stats", {}))
                checkpoint("hardware_counters", {"before": before, "after": after})
                if after[0] <= before[0]:
                    raise RuntimeError("native CFA4 GPU decode produced zero NVDEC frames (FFmpeg CLI counters do not count)")
                if after[1] <= before[1] or after[2] <= before[2]:
                    raise RuntimeError("native CFA4 export did not exercise its NVENC hardware encoder")
                # Native NVDEC only implements H264 and HEVC today. AV1 import
                # does not imply AV1 hardware decode; this is a separate gap.
                checkpoint("av1_native_nvdec", "NOT_IMPLEMENTED: CFA4 Linux NVDEC handles H.264/HEVC only")
                checkpoint("qt6_full_editor", "NOT_IMPLEMENTED: Qt6 is only an experimental shell")
                evidence["result"] = "PASS_NATIVE_ENGINE_PARTIAL_R0"
                return 0
            finally:
                proc.terminate()
                try:
                    proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=5)
    except subprocess.TimeoutExpired as exc:
        def captured(value: str | bytes | None) -> str:
            if value is None:
                return ""
            if isinstance(value, bytes):
                return value.decode("utf-8", errors="replace")
            return value

        checkpoint("error", {
            "message": str(exc),
            "stdout": captured(exc.stdout)[-4000:],
            "stderr": captured(exc.stderr)[-4000:],
        })
        print(f"CFA4 native media E2E FAIL: {exc}", file=sys.stderr)
        return 1
    except (OSError, RuntimeError, ValueError, KeyError) as exc:
        checkpoint("error", str(exc))
        print(f"CFA4 native media E2E FAIL: {exc}", file=sys.stderr)
        return 1
    finally:
        (out / "results.json").write_text(json.dumps(evidence, indent=2, ensure_ascii=False), encoding="utf-8")
        print(f"Native CFA4 E2E evidence: {out}")


if __name__ == "__main__":
    raise SystemExit(main())
