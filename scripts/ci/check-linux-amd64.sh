#!/usr/bin/env bash
# Fast, non-compiling CFA4 Linux AMD64 policy gate.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "CFA4 only supports Linux x86_64 (AMD64)." >&2
  exit 2
fi
for script in \
  packaging/linux/package.sh \
  packaging/linux/flatpak-bundle.sh \
  packaging/linux/install.sh \
  scripts/dev/bootstrap-ubuntu.sh \
  scripts/dev/verify-linux.sh \
  scripts/dev/audit-gpu-hardware.sh \
  scripts/dev/audit-ffmpeg-hwaccel.sh \
  scripts/dev/recheck-libplacebo.sh \
  scripts/ci/read-ffmpeg-audit-sha256.sh \
  scripts/ci/test-ffmpeg-audit-sha256.sh \
  scripts/ci/test-gpu-hardware-log.sh \
  scripts/dev/test-vendored-ui.sh \
  scripts/ci/install-ffmpeg-9.0.2.sh; do
  bash -n "$script"
done
# Strict GPU hardware log parser regression (only shell + temporary logs,
# no cargo, no GPU and no expensive full Rust/Qt/FFmpeg tests).
bash scripts/ci/test-gpu-hardware-log.sh
bash scripts/ci/test-ffmpeg-audit-sha256.sh
python3 - <<'PY'
from pathlib import Path
import ast
import tomllib

root = Path.cwd()
workflow = (root / ".github/workflows/ci-linux.yml").read_text()
assert "  platform-scope:" in workflow
assert workflow.count("github.event.label.name == 'cfa4-full-validation'") == 7
assert "types: [opened, synchronize, reopened, labeled]" in workflow
assert workflow.count("    needs: platform-scope") == 7
assert workflow.count("    runs-on: ubuntu-26.04") == 8
assert "audit-gpu-hardware.sh" not in workflow, "physical GPU gate must not run on hosted CI"
assert "CFA4_REQUIRE_GPU_FX" in (root / "crates/gpu/src/fx_tests.rs").read_text()
assert "CFA4_GPU_HARDWARE_ADAPTER:" in (root / "crates/gpu/src/blend_tests.rs").read_text()
ffmpeg_hw = (root / "scripts/dev/audit-ffmpeg-hwaccel.sh").read_text()
for marker in ("CFA4_FFMPEG_BIN", "CFA4_FFPROBE_BIN", "nvenc_", "nvdec_", "av1_nvdec",
               "av1_software_sample", "scale_vulkan", "libplacebo", "avgblur_opencl",
               "UNSUPPORTED_HARDWARE", "CFA4_VULKAN_DEVICE", "-buildconf", "-hwaccel_output_format"):
    assert marker in ffmpeg_hw, f"FFmpeg hardware audit missing {marker}"
# Do not regress to 128x72: Ampere NVENC rejects that tiny frame, and its
# AV1 NVDEC reports min_height=128. Validate executable fixture dimensions.
import re
source_w = int(re.search(r"^SOURCE_W=(\d+)$", ffmpeg_hw, re.M).group(1))
source_h = int(re.search(r"^SOURCE_H=(\d+)$", ffmpeg_hw, re.M).group(1))
assert source_w >= 320 and source_h >= 240 and source_w % 2 == 0 and source_h % 2 == 0
assert "testsrc2=size=${SOURCE_W}x${SOURCE_H}" in ffmpeg_hw
assert ffmpeg_hw.count('"$SOURCE_W" "$SOURCE_H"') >= 4
assert "libplacebo=w=${DOWNSCALE_W}:h=${DOWNSCALE_H}:format=yuv420p" in ffmpeg_hw
assert ffmpeg_hw.count("check_framehash") >= 4
# libplacebo owns its Vulkan context; avoid importing the incompatible
# FFmpeg Vulkan queue flags into older libplacebo API versions.
libplacebo_branch = ffmpeg_hw.split("    if has_filter libplacebo; then", 1)[1].split("\n    else\n        record libplacebo REQUIRED NOT_BUILT", 1)[0]
assert "-filter_hw_device vk" not in libplacebo_branch.split("if run_step libplacebo REQUIRED", 1)[1]
assert "Device Name: $GPU_NAME" in libplacebo_branch
recheck_lp = (root / "scripts/dev/recheck-libplacebo.sh").read_text()
for marker in ("-vf \"libplacebo=w=320:h=180:format=yuv420p\"", "source.mkv",
               "Device Name: $GPU_NAME", "libplacebo.framehash", '"$actual" == 8'):
    assert marker in recheck_lp, f"libplacebo recheck script missing {marker}"
assert "-filter_hw_device" not in recheck_lp.split("if ! \"$FFMPEG\"", 1)[1]
assert "cargo build" not in recheck_lp and "apt-get" not in recheck_lp
assert "bash scripts/ci/read-ffmpeg-audit-sha256.sh" in recheck_lp
assert "sed -n 's/^FFmpeg SHA256" not in recheck_lp
assert "hwdownload,format=nv12" in ffmpeg_hw
assert "cargo build" not in ffmpeg_hw and "apt-get" not in ffmpeg_hw
assert "audit-ffmpeg-hwaccel.sh" not in workflow, "do not run physical NVENC on hosted CI"
assert "cargo test --workspace --exclude craft-ui --release --locked --timings" in workflow

for filename in ("packaging/linux/package.sh", "packaging/linux/flatpak-bundle.sh",
                 "packaging/linux/install.sh"):
    src = (root / filename).read_text()
    assert '$(uname -m)' in src and "x86_64" in src, filename
    assert not any(arch in src for arch in ("aarch64", "arm64", "riscv64", "android")), filename

assert 'arch: amd64' in (root / "packaging/linux/nfpm.yaml").read_text()
assert 'CARGO_BUILD_TARGET' in (root / "packaging/linux/package.sh").read_text()
package_script = (root / "packaging/linux/package.sh").read_text()
assert 'CFA4_APPIMAGETOOL_SHA256' in package_script
assert 'sha256sum -- "$TOOL"' in package_script
assert 'appimagetool SHA-256 mismatch' in package_script
assert 'releases/download/continuous/appimagetool' not in package_script
assert 'curl -fsSL -o "$TOOL"' not in package_script

for filename in ("python/cfa4_devtools/health.py", "python/tests/test_devtools.py",
                 "packaging/linux/test_install.py"):
    ast.parse((root / filename).read_text(), filename=filename)
tomllib.loads((root / "pyproject.toml").read_text())
tomllib.loads((root / "rust-toolchain.toml").read_text())
print("PASS: Linux x86_64/AMD64 policy and lightweight configuration checks")
PY
# All stdlib, offline and non-compiling: validates Cargo Git sources, lockfile
# vendor paths, source manifests and individual Git-blob provenance hashes.
PYTHONPATH=python python3 -m cfa4_devtools.dependency_audit
