"""CFA4-RENDER-INTEGRATION-PLAN-001 F0a: the render inventory generator and its committed output."""

from __future__ import annotations

from pathlib import Path

from cfa4_devtools import render_inventory as ri

ROOT = Path(__file__).resolve().parents[2]


def _crate(root: Path, directory: str, name: str, deps: list[str], files: dict[str, str]) -> None:
    base = root / "crates" / directory
    (base / "src").mkdir(parents=True)
    dep_lines = "\n".join(f"{d} = {{ workspace = true }}" for d in deps)
    (base / "Cargo.toml").write_text(
        f'[package]\nname = "{name}"\ndescription = "{name} test"\n\n[dependencies]\n{dep_lines}\n', encoding="utf-8"
    )
    for rel, text in files.items():
        (base / "src" / rel).write_text(text, encoding="utf-8")


def test_edges_types_and_entry_points(tmp_path: Path) -> None:
    _crate(tmp_path, "time", "cfa4-time", [], {"lib.rs": "pub struct Tick(pub i64);\n"})
    _crate(
        tmp_path,
        "render",
        "cfa4-render",
        ["cfa4-time", "rayon"],
        {
            "lib.rs": "use cfa4_time::Tick;\n// cfa4_time::Ignored\npub struct Image;\npub fn render_sequence() {}\n",
            "tests.rs": "use cfa4_time::FromTest;\n",
        },
    )
    inv = ri.build(tmp_path)
    assert inv["crate_edges"] == [{"from": "cfa4-render", "to": "cfa4-time"}]
    assert inv["use_edges"] == [{"from": "cfa4-render/lib.rs", "to": "cfa4-time", "item": "Tick"}]
    assert {(t["name"], t["class"]) for t in inv["types"]} == {("Tick", "time"), ("Image", "frame")}
    assert inv["entry_points"] == [{"crate": "cfa4-render", "module": "lib.rs", "fn": "render_sequence"}]


def test_output_is_deterministic(tmp_path: Path) -> None:
    _crate(tmp_path, "time", "cfa4-time", [], {"lib.rs": "pub struct Tick(pub i64);\n"})
    first = ri.outputs(tmp_path)
    assert first == ri.outputs(tmp_path)


def test_committed_inventory_is_current() -> None:
    assert ri.main(["--root", str(ROOT), "--check"]) == 0


def test_environment_snapshot_is_read_only_json() -> None:
    from cfa4_devtools import render_environment as re_env

    snap = re_env.snapshot(ROOT)
    assert snap["schema"] == "cfa4.render-environment/1"
    assert snap["toolchain"]["msrv"] and snap["toolchain"]["rust_toolchain_toml"]
    assert len(snap["source"]["lockfiles_sha256"]["Cargo.lock"]) == 64
