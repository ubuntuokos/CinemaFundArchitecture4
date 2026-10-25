#!/usr/bin/env bash
# Run the original cfa4-ui test suite outside the CFA4 root dependency graph.
# Isolate test-only crates so the application Cargo.lock need not include them.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source_dir="$ROOT/vendor/cfa4-libs/ui"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT
cp -a "$source_dir/." "$tmpdir/"
cat >> "$tmpdir/Cargo.toml" <<'TOML'

[workspace]

[dev-dependencies]
egui = { version = "0.36", features = ["default_fonts"] }
serde_json = "1"
egui_extras = { version = "0.36", default-features = false, features = ["svg"] }
egui_kittest = { version = "0.36", features = ["wgpu"] }
image = { version = "0.25", default-features = false, features = ["png"] }
TOML
cargo test --manifest-path "$tmpdir/Cargo.toml" --lib --tests
