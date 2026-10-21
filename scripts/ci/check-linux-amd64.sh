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
  scripts/dev/test-vendored-ui.sh \
  scripts/ci/install-ffmpeg-9.0.2.sh; do
  bash -n "$script"
done
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
assert "cargo test --workspace --exclude craft-ui --release --locked --timings" in workflow

for filename in ("packaging/linux/package.sh", "packaging/linux/flatpak-bundle.sh"):
    src = (root / filename).read_text()
    assert '$(uname -m)' in src and "x86_64" in src, filename
    assert not any(arch in src for arch in ("aarch64", "arm64", "riscv64", "android")), filename

for filename in ("python/cfa4_devtools/health.py", "python/tests/test_devtools.py",
                 "packaging/linux/test_install.py"):
    ast.parse((root / filename).read_text(), filename=filename)
tomllib.loads((root / "pyproject.toml").read_text())
tomllib.loads((root / "rust-toolchain.toml").read_text())
print("PASS: Linux x86_64/AMD64 policy and lightweight configuration checks")
PY
