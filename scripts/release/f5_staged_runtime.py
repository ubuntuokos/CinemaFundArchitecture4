#!/usr/bin/env python3
"""CFA4 F5: non-invasive staged runtime audit on Ubuntu 26.04 AMD64.

Smart Cut (WP-10, docs/plans/CFA4-SMART-CUT-RENDER-R1.md): every format's packaged CLI cuts an
MPEG-2 transport stream on its GOP boundaries with CFA4's own Rust code, verifies the copy and
reads the result back; no package carries or depends on GOPchop.

Media QC (stage G, docs/plans/MEDIA-QC-2026-10-27.md): every format's packaged CLI checks an MP4
with the built-in `delivery-web` profile (`cfa4-cli qc`). A sound file decodes, a truncated one
and one that misses its render plan FAIL, and no file passes without a human review.

Does NOT certify actual system .deb install, Flatpak sandbox launch, GPU/GUI,
AppImage AppRun, legal redistribution, or complete dlopen dependency inventory.
All extracted files, trial prefix installs and imported bundle data are temporary.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

from verify_cfa4_packages import APP_ID, AuditError, repo_version, verify_package_set

MAX_OUTPUT = 64000


def invoke(args: list[str], *, cwd: Path | None = None, env: dict | None = None,
           timeout: int = 90) -> str:
    try:
        result = subprocess.run(
            args, cwd=cwd, env=env, capture_output=True, text=True,
            timeout=timeout, check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise AuditError(f"unable to run {args[0]}: {exc}") from exc
    if result.returncode:
        raise AuditError(
            f"{args[0]} exited {result.returncode}: "
            f"{(result.stderr + result.stdout)[-1500:]}"
        )
    return (result.stdout + result.stderr)[:MAX_OUTPUT]


def isolated_env(root: Path) -> dict[str, str]:
    home = root / "home"
    for name in ("config", "cache", "data"):
        (home / f".{name}").mkdir(parents=True, exist_ok=True)
    return {
        **os.environ,
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / ".config"),
        "XDG_CACHE_HOME": str(home / ".cache"),
        "XDG_DATA_HOME": str(home / ".data"),
        "XDG_STATE_HOME": str(home / ".state"),
    }


def library_inventory(exe: Path, *, env: dict[str, str]) -> dict:
    if not exe.is_file():
        raise AuditError(f"missing executable: {exe}")
    result = invoke(["ldd", str(exe)], env=env)
    missing = re.findall(r"^\s*(\S+)\s*=>\s*not found\b", result, flags=re.M)
    if missing:
        raise AuditError(f"missing ELF runtime dependencies for {exe.name}: {missing}")
    return {"executable": exe.name, "missing": [], "ldd": result.strip()}


def cli_smoke(exe: Path, version: str, env: dict[str, str]) -> dict:
    if not exe.is_file() or not os.access(exe, os.X_OK):
        raise AuditError(f"missing executable CFA4 CLI: {exe}")
    output = invoke([str(exe), "--version"], env=env, timeout=20).strip()
    if version not in output:
        raise AuditError(f"CFA4 CLI output lacks expected version {version}: {output!r}")
    return {"cli_version": output, "version_match": True}


# The Smart Cut smoke cuts frames [10, 50) of a 4 s, 25 fps MPEG-2 stream with closed GOPs of 10.
SMART_CUT_RANGE = (10, 50)
SMART_CUT_FRAMES = SMART_CUT_RANGE[1] - SMART_CUT_RANGE[0]
# The frame-accurate cut [5, 45): the pictures it can copy whole are copied, the boundary pictures
# re-encoded with CFA4's MPEG-2 encoder.
FRAME_ACCURATE_RANGE = (5, 45)
FRAME_ACCURATE_FRAMES = FRAME_ACCURATE_RANGE[1] - FRAME_ACCURATE_RANGE[0]
FRAME_ACCURATE_MIN_PSNR = 30.0


def fixture_ffmpeg() -> str:
    """FFmpeg makes the Smart Cut fixture: CFA4's own 9.0.2 build, else the one on PATH."""
    build = Path(os.environ.get("CFA4_FFMPEG_BUILD", "/opt/cfa4/ffmpeg-9.0.2")) / "bin/ffmpeg"
    if build.is_file() and os.access(build, os.X_OK):
        return str(build)
    found = shutil.which("ffmpeg")
    if not found:
        raise AuditError("the Smart Cut smoke needs ffmpeg for its MPEG-2 fixture "
                         "(CFA4_FFMPEG_BUILD/bin/ffmpeg or PATH)")
    return found


def make_smart_cut_fixture(ffmpeg: str, work: Path, env: dict[str, str]) -> Path:
    """An MPEG-2 transport stream: closed GOPs of 10 with two B pictures, and MP2 audio."""
    source = work / "smart-cut-source.ts"
    invoke([
        ffmpeg, "-v", "error", "-y",
        "-f", "lavfi", "-i", "testsrc2=size=176x144:rate=25",
        "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", "4",
        "-c:v", "mpeg2video", "-g", "10", "-bf", "2", "-flags", "+cgop",
        "-sc_threshold", "1000000000", "-c:a", "mp2", "-b:a", "128k",
        "-f", "mpegts", str(source),
    ], env=env, timeout=60)
    if not source.is_file() or source.stat().st_size == 0:
        raise AuditError("ffmpeg wrote no Smart Cut fixture")
    return source


def run_results(output: str) -> list[dict]:
    """The result lines of `cfa4-cli run`, in order."""
    results = []
    for line in output.splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                value = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(value, dict) and "line" in value:
                results.append(value)
    return results


def smart_cut_smoke(cli: Path, source: Path, work: Path, env: dict[str, str]) -> dict:
    """The packaged CLI cuts `source` on GOP boundaries and frame accurately (re-encoding the
    boundary pictures with CFA4's MPEG-2 encoder), verifies both cuts and reads them back."""
    work.mkdir(parents=True, exist_ok=True)
    output = work / "smart-cut.ts"
    accurate = work / "smart-cut-frame-accurate.ts"
    first, end = SMART_CUT_RANGE
    cut = {"path": str(source), "in": {"frame": first}, "out": {"frame": end},
           "mode": "gop-boundary", "output": str(output)}
    fa_first, fa_end = FRAME_ACCURATE_RANGE
    fa_cut = {"path": str(source), "in": {"frame": fa_first}, "out": {"frame": fa_end},
              "mode": "frame-accurate", "output": str(accurate), "confirm": True}
    steps = [
        {"id": "export.analyze", "params": {"path": str(source)}},
        {"id": "export.plan", "params": cut},
        {"id": "export.start", "params": cut},
        {"id": "export.analyze", "params": {"path": str(output)}},
        {"id": "export.plan", "params": fa_cut},
        {"id": "export.start", "params": fa_cut},
        {"id": "export.analyze", "params": {"path": str(accurate)}},
    ]
    script = work / "smart-cut.jsonl"
    script.write_text("".join(json.dumps(step) + "\n" for step in steps), encoding="utf-8")
    results = run_results(invoke([str(cli), "run", str(script), "--data-dir", str(work / "data")],
                                 env=env, timeout=180))
    if len(results) != len(steps) or not all(r.get("ok") for r in results):
        raise AuditError(f"Smart Cut smoke failed: {results}")
    analyzed, planned, done, back, fa_planned, fa_done, fa_back = (r.get("result") or {} for r in results)
    if analyzed.get("codec") != "mpeg2-video":
        raise AuditError(f"Smart Cut smoke: the fixture is not MPEG-2 video: {analyzed}")
    summary = planned.get("summary") or {}
    want = {"lossless": True, "requiresConfirmation": False,
            "copiedFrames": SMART_CUT_FRAMES, "reencodedFrames": 0}
    if any(summary.get(k) != v for k, v in want.items()):
        raise AuditError(f"Smart Cut smoke: unexpected plan {summary}")
    report = done.get("report") or {}
    tracks = (done.get("audio") or {}).get("tracks") or []
    if done.get("verified") is not True or report.get("samples") != SMART_CUT_FRAMES or len(tracks) != 1:
        raise AuditError(f"Smart Cut smoke: the copy was not verified: {done}")
    if not output.is_file() or output.stat().st_size != done.get("bytes"):
        raise AuditError("Smart Cut smoke: the output file differs from the reported copy")
    if back.get("codec") != "mpeg2-video" or back.get("frames") != SMART_CUT_FRAMES:
        raise AuditError(f"Smart Cut smoke: the output reads back wrong: {back}")
    frame_accurate = check_frame_accurate(accurate, fa_planned, fa_done, fa_back)
    return {
        "scope": "SMART_CUT_MPEG2_GOP_COPY_AND_FRAME_ACCURATE_IN_PACKAGED_CLI",
        "copied_frames": SMART_CUT_FRAMES,
        "audio_tracks": len(tracks),
        "output_bytes": done.get("bytes"),
        "verified": True,
        "frame_accurate": frame_accurate,
    }


def check_frame_accurate(output: Path, planned: dict, done: dict, back: dict) -> dict:
    """The frame-accurate cut: boundaries re-encoded, whole GOPs copied, verified and readable."""
    ready = (planned.get("boundaryReencode") or {}).get("ready")
    summary = planned.get("summary") or {}
    if ready is not True or summary.get("lossless") is not False or not summary.get("reencodedFrames"):
        raise AuditError(f"Smart Cut smoke: unexpected frame-accurate plan {planned}")
    report = done.get("report") or {}
    psnr = report.get("minPsnr")
    if (done.get("verified") is not True or report.get("samples") != FRAME_ACCURATE_FRAMES
            or not report.get("reencoded") or not report.get("copied")
            or not isinstance(psnr, (int, float)) or psnr < FRAME_ACCURATE_MIN_PSNR):
        raise AuditError(f"Smart Cut smoke: the frame-accurate cut was not verified: {done}")
    if not output.is_file() or output.stat().st_size != done.get("bytes"):
        raise AuditError("Smart Cut smoke: the frame-accurate output differs from the reported file")
    if back.get("codec") != "mpeg2-video" or back.get("frames") != FRAME_ACCURATE_FRAMES:
        raise AuditError(f"Smart Cut smoke: the frame-accurate output reads back wrong: {back}")
    return {"frames": FRAME_ACCURATE_FRAMES, "copied": report["copied"],
            "reencoded": report["reencoded"], "min_psnr": psnr}


# The QC fixture: 4 s at 25 fps, MPEG-2 video (no encoder licence question) and AAC in MP4.
QC_FRAMES = 100
# `cfa4-cli qc` exit codes (apps/cfa4-cli/src/qc.rs): the verdict, never 1 (error) or 2 (usage)
QC_EXIT = {"PASS": 0, "WARN": 3, "FAIL": 4, "INCOMPLETE": 5}
QC_REPORT_SCHEMA = "cfa4-qc-report/1"


def make_qc_fixture(ffmpeg: str, work: Path, env: dict[str, str]) -> Path:
    """An MP4 the QC reads sample by sample: MPEG-2 video and AAC audio."""
    source = work / "qc-source.mp4"
    invoke([
        ffmpeg, "-v", "error", "-y",
        "-f", "lavfi", "-i", "testsrc2=size=176x144:rate=25",
        "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", "4",
        "-c:v", "mpeg2video", "-g", "10", "-c:a", "aac", "-b:a", "128k",
        "-movflags", "+faststart", str(source),
    ], env=env, timeout=60)
    if not source.is_file() or source.stat().st_size == 0:
        raise AuditError("ffmpeg wrote no QC fixture")
    return source


def run_qc(cli: Path, media: Path, out: Path, env: dict[str, str],
           expect: Path | None = None) -> dict:
    """`cfa4-cli qc media --out out`: the report, checked against the exit code."""
    args = [str(cli), "qc", str(media), "--profile", "delivery-web", "--out", str(out)]
    if expect is not None:
        args += ["--expect", str(expect)]
    try:
        result = subprocess.run(args, env=env, capture_output=True, text=True, timeout=180,
                                check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise AuditError(f"QC smoke: unable to run {cli.name} qc: {exc}") from exc
    said = (result.stderr + result.stdout)[-1500:]
    if result.returncode not in QC_EXIT.values():
        raise AuditError(f"QC smoke: {cli.name} qc exited {result.returncode}: {said}")
    try:
        report = json.loads(out.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        raise AuditError(f"QC smoke: no readable report for {media.name}: {exc}") from exc
    if not isinstance(report, dict) or report.get("schema") != QC_REPORT_SCHEMA:
        raise AuditError(f"QC smoke: the report is not {QC_REPORT_SCHEMA}: {str(report)[:300]}")
    profile = report.get("profile") or {}
    if profile.get("id") != "delivery-web" or not re.fullmatch(r"[0-9a-f]{64}", str(profile.get("sha256"))):
        raise AuditError(f"QC smoke: the report names no built-in profile: {profile}")
    verdict = (report.get("summary") or {}).get("verdict")
    if QC_EXIT.get(verdict) != result.returncode:
        raise AuditError(f"QC smoke: verdict {verdict} but exit code {result.returncode}")
    # crates/format/src/atomic.rs writes `.<name>.<pid>-<n>.tmp` and renames it over the report
    leftovers = sorted(p.name for p in out.parent.iterdir()
                       if p.name.startswith(f".{out.name}.") and p.name.endswith(".tmp"))
    if leftovers:
        raise AuditError(f"QC smoke: the report was not written atomically: {leftovers}")
    return report


def qc_status(report: dict, check: str) -> str:
    for c in report.get("checks") or []:
        if isinstance(c, dict) and c.get("id") == check:
            return str(c.get("status"))
    return "MISSING"


def qc_smoke(cli: Path, source: Path, work: Path, env: dict[str, str]) -> dict:
    """The packaged CLI's QC: the profile is built in, the decoders run, a broken file and a
    render-plan mismatch FAIL, and nothing PASSes without a human review."""
    work.mkdir(parents=True, exist_ok=True)
    sound = run_qc(cli, source, work / "sound.cfa4qc.json", env)
    if qc_status(sound, "probe.open") != "PASS" or qc_status(sound, "integrity.decode") != "PASS":
        raise AuditError("QC smoke: the packaged decoders did not read the fixture: "
                         f"{(sound.get('summary') or {}).get('reasons')}")
    if sound["summary"]["verdict"] == "PASS":
        raise AuditError("QC smoke: a file without a human review PASSed")
    plan = work / "plan.json"
    plan.write_text(json.dumps({"frames": QC_FRAMES + 1}), encoding="utf-8")
    mismatch = run_qc(cli, source, work / "mismatch.cfa4qc.json", env, expect=plan)
    if qc_status(mismatch, "integrity.frameCount") != "FAIL" or mismatch["summary"]["verdict"] != "FAIL":
        raise AuditError("QC smoke: a frame count that misses the render plan did not FAIL")
    truncated = work / "truncated.mp4"
    data = source.read_bytes()
    truncated.write_bytes(data[: len(data) * 6 // 10])
    broken = run_qc(cli, truncated, work / "truncated.cfa4qc.json", env)
    if qc_status(broken, "integrity.decode") != "FAIL" or broken["summary"]["verdict"] != "FAIL":
        raise AuditError("QC smoke: a truncated file did not FAIL its decode check")
    return {
        "scope": "MEDIA_QC_DELIVERY_WEB_IN_PACKAGED_CLI",
        "profile_sha256": sound["profile"]["sha256"],
        "verdicts": {"sound": sound["summary"]["verdict"], "frame_count_mismatch": "FAIL",
                     "truncated": "FAIL"},
    }


def assert_no_gopchop(root: Path, what: str) -> None:
    """Smart Cut is CFA4's own code: no package ships GOPchop (WP-10 gate)."""
    found = sorted(str(p.relative_to(root)) for p in root.rglob("*") if "gopchop" in p.name.lower())
    if found:
        raise AuditError(f"{what} ships GOPchop files: {found[:10]}")


def stage_tarball(archive_path: Path, target: Path, version: str,
                  env: dict[str, str], fixture: Path, qc_fixture: Path) -> dict:
    stage = target / "tar-source"
    stage.mkdir()
    # Already inspected by verify_package_set; tarfile data filter also rejects
    # unexpected links, traversal and device nodes.
    with tarfile.open(archive_path, "r:gz") as archive:
        archive.extractall(stage, filter="data")
    unpacked = stage / f"cfa4-{version}-linux-x86_64"
    installed = target / "tar-prefix"
    invoke(["bash", str(unpacked / "install.sh"), "--prefix", str(installed)], env=env)
    if not (installed / "share/applications" / f"{APP_ID}.desktop").is_file():
        raise AuditError("tar prefix installer omitted CFA4 desktop entry")
    if not (installed / "share/metainfo" / f"{APP_ID}.metainfo.xml").is_file():
        raise AuditError("tar prefix installer omitted CFA4 AppStream data")
    binaries = [installed / "bin/cfa4", installed / "bin/cfa4-cli"]
    assert_no_gopchop(installed, "the tarball")
    return {
        "scope": "PREFIX_INSTALL_AND_CLI_SMOKE_NOT_GUI",
        **cli_smoke(binaries[1], version, env),
        "smart_cut": smart_cut_smoke(binaries[1], fixture, target / "tar-smart-cut", env),
        "qc": qc_smoke(binaries[1], qc_fixture, target / "tar-qc", env),
        "libraries": [library_inventory(path, env=env) for path in binaries],
        "cleanup": "isolated temporary prefix deleted by TemporaryDirectory",
    }


def stage_deb(package: Path, target: Path, version: str,
              env: dict[str, str], fixture: Path, qc_fixture: Path) -> dict:
    stage = target / "deb-unpacked"
    stage.mkdir()
    invoke(["dpkg-deb", "--extract", str(package), str(stage)], env=env)
    binaries = [stage / "usr/bin/cfa4", stage / "usr/bin/cfa4-cli"]
    assert_no_gopchop(stage, "the .deb")
    relations = invoke(["dpkg-deb", "--field", str(package), "Depends", "Recommends", "Suggests"], env=env)
    if "gopchop" in relations.lower():
        raise AuditError(f"the .deb depends on GOPchop: {relations.strip()}")
    return {
        "scope": "DEB_UNPACK_AND_CLI_SMOKE_NOT_DPKG_INSTALL",
        **cli_smoke(binaries[1], version, env),
        "smart_cut": smart_cut_smoke(binaries[1], fixture, target / "deb-smart-cut", env),
        "qc": qc_smoke(binaries[1], qc_fixture, target / "deb-qc", env),
        "libraries": [library_inventory(path, env=env) for path in binaries],
        "cleanup": "temporary extracted FHS tree removed",
    }


def stage_appimage(package: Path, target: Path, version: str,
                   env: dict[str, str], fixture: Path, qc_fixture: Path) -> dict:
    work = target / "appimage-unpacked"
    work.mkdir()
    invoke([str(package), "--appimage-extract"], cwd=work, env=env, timeout=120)
    appdir = work / "squashfs-root"
    if not (appdir / "AppRun").exists():
        raise AuditError("AppImage extract has no AppRun")
    binaries = [appdir / "usr/bin/cfa4", appdir / "usr/bin/cfa4-cli"]
    assert_no_gopchop(appdir, "the AppImage")
    return {
        "scope": "APPIMAGE_EXTRACT_AND_CLI_SMOKE_NOT_APPRUN",
        **cli_smoke(binaries[1], version, env),
        "smart_cut": smart_cut_smoke(binaries[1], fixture, target / "appimage-smart-cut", env),
        "qc": qc_smoke(binaries[1], qc_fixture, target / "appimage-qc", env),
        "libraries": [library_inventory(path, env=env) for path in binaries],
        "cleanup": "temporary AppDir extracted files removed",
    }


def stage_flatpak(package: Path, target: Path, env: dict[str, str]) -> dict:
    # Import to a private temporary OSTree repo, not ~/.local/share/flatpak or
    # the system installation. Neither app nor runtime is installed.
    repo = target / "private-flatpak-repo"
    invoke(["ostree", "init", f"--repo={repo}", "--mode=archive-z2"], env=env)
    invoke(["flatpak", "build-import-bundle", str(repo), str(package)], env=env,
           timeout=120)
    refs = invoke(["ostree", f"--repo={repo}", "refs"], env=env).splitlines()
    prefix = f"app/{APP_ID}/x86_64/"
    found = sorted(ref for ref in refs if ref.startswith(prefix))
    if len(found) != 1:
        raise AuditError(f"Flatpak bundle unexpected app refs: {refs}")
    return {
        "scope": "PRIVATE_BUNDLE_IMPORT_NOT_FLATPAK_INSTALL_OR_RUNTIME",
        "ref": found[0],
        "cleanup": "private temporary OSTree repo removed",
    }


def flatpak_payload(target: Path, ref: str, version: str, env: dict[str, str],
                    fixture: Path, qc_fixture: Path) -> dict:
    """The bundle's /app tree, checked out of the private repo: the same CLI and Smart Cut smoke
    on the host (not inside the Flatpak sandbox or runtime)."""
    repo = target / "private-flatpak-repo"
    tree = target / "flatpak-checkout"
    invoke(["ostree", f"--repo={repo}", "checkout", "--user-mode", ref, str(tree)], env=env,
           timeout=120)
    cli = tree / "files/bin/cfa4-cli"
    assert_no_gopchop(tree, "the Flatpak bundle")
    return {
        "scope": "CHECKOUT_AND_CLI_SMOKE_ON_HOST_NOT_FLATPAK_SANDBOX",
        **cli_smoke(cli, version, env),
        "smart_cut": smart_cut_smoke(cli, fixture, target / "flatpak-smart-cut", env),
        "qc": qc_smoke(cli, qc_fixture, target / "flatpak-qc", env),
    }


def check_platform() -> None:
    if sys.platform != "linux" or os.uname().machine != "x86_64":
        raise AuditError("F5 requires Ubuntu 26.04 x86_64")
    data = {}
    for line in Path("/etc/os-release").read_text().splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            data[k] = v.strip('"')
    if data.get("VERSION_ID") != "26.04" or (
        data.get("ID") != "ubuntu" and "ubuntu" not in data.get("ID_LIKE", "").split()
    ):
        raise AuditError("F5 requires Ubuntu 26.04 or an Ubuntu 26.04 flavour")


def audit(dist: Path, version: str) -> dict:
    check_platform()
    # Verify integrity of all four artifacts before executing any CFA4 binary.
    structure = verify_package_set(dist, version)
    basename = f"cfa4-{version}-linux-x86_64"
    with tempfile.TemporaryDirectory(prefix="cfa4-f5-staging-") as tmp:
        root = Path(tmp)
        env = isolated_env(root)
        ffmpeg = fixture_ffmpeg()
        fixture = make_smart_cut_fixture(ffmpeg, root, env)
        qc_fixture = make_qc_fixture(ffmpeg, root, env)
        flatpak = stage_flatpak(dist / f"{basename}.flatpak", root, env)
        flatpak["payload"] = flatpak_payload(root, flatpak["ref"], version, env, fixture, qc_fixture)
        result = {
            "schema": "CFA4_F5_STAGING_V1",
            "build_version": version,
            "structure": structure["status"],
            "tarball": stage_tarball(dist / f"{basename}.tar.gz", root, version, env, fixture, qc_fixture),
            "deb": stage_deb(dist / f"{basename}.deb", root, version, env, fixture, qc_fixture),
            "appimage": stage_appimage(dist / f"{basename}.AppImage", root, version, env, fixture, qc_fixture),
            "flatpak": flatpak,
            "status": "PASS_STAGING_ONLY",
            "not_proven": [
                "clean VM .deb install and removal",
                "Flatpak runtime install/sandbox launch/removal",
                "AppImage AppRun GUI launch",
                "KDE/GNOME/Cinnamon desktop integration",
                "GPU, dynamic-loaded libraries, licensing and distribution",
            ],
        }
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--report", type=Path, help="Optional JSON audit report path")
    args = parser.parse_args()
    try:
        result = audit(args.dist.resolve(), repo_version())
    except (AuditError, OSError, subprocess.CalledProcessError, tarfile.TarError) as exc:
        print(f"FAIL CFA4 F5 staged validation: {exc}", file=sys.stderr)
        return 1
    payload = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(payload, encoding="utf-8")
    print(payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
