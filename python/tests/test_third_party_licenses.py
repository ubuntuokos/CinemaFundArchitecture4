"""THIRD-PARTY-LICENSES.txt for the packages (cfa4_devtools.third_party_licenses, issue #220)."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest
from cfa4_devtools.third_party_licenses import NoticeError, collect, main, render, shipped_package_ids

MIT_SERDE = "MIT License\n\nCopyright (c) serde authors\n"
APACHE = "Apache License\nVersion 2.0, January 2004\n"


def _crate(tmp: Path, name: str, files: dict[str, str]) -> Path:
    crate = tmp / name
    crate.mkdir()
    (crate / "Cargo.toml").write_text(f'[package]\nname = "{name}"\n', encoding="utf-8")
    (crate / "src").mkdir()
    (crate / "src" / "LICENSE-not-top-level").write_text("ignored", encoding="utf-8")
    for file_name, text in files.items():
        (crate / file_name).write_text(text, encoding="utf-8")
    return crate / "Cargo.toml"


def _metadata(tmp: Path) -> dict[str, Any]:
    def pkg(name: str, manifest: Path, license: str | None = "MIT OR Apache-2.0", source: str | None = "registry") -> dict[str, Any]:
        return {"id": f"{name}-id", "name": name, "version": "1.0.0", "license": license, "source": source,
                "repository": f"https://example.invalid/{name}", "authors": [f"{name} authors"],
                "manifest_path": str(manifest), "license_file": None}

    packages = [
        pkg("cfa4", _crate(tmp, "cfa4", {"LICENSE": "GPL"}), "GPL-3.0-or-later", None),
        pkg("cfa4-cli", _crate(tmp, "cfa4-cli", {}), "GPL-3.0-or-later", None),
        pkg("cfa4-engine", _crate(tmp, "cfa4-engine", {}), "GPL-3.0-or-later", None),
        pkg("serde", _crate(tmp, "serde", {"LICENSE-MIT": MIT_SERDE, "LICENSE-APACHE": APACHE, "README.md": "x"})),
        pkg("zlib-rs", _crate(tmp, "zlib-rs", {"LICENSE": "zlib License\r\n", "NOTICE.txt": "Notice\n"}), "Zlib"),
        pkg("anyhow", _crate(tmp, "anyhow", {"LICENSE-APACHE": APACHE})),
        pkg("nofile", _crate(tmp, "nofile", {})),
        pkg("criterion", _crate(tmp, "criterion", {"LICENSE": "dev only"})),
        pkg("cc", _crate(tmp, "cc", {"LICENSE": "build only"})),
        pkg("unrelated", _crate(tmp, "unrelated", {"LICENSE": "not reachable"})),
    ]
    normal = [{"kind": None, "target": None}]

    def dep(name: str, kinds: list[dict[str, Any]] | None = None) -> dict[str, Any]:
        return {"pkg": f"{name}-id", "dep_kinds": kinds or normal}

    nodes = [
        {"id": "cfa4-id", "deps": [dep("cfa4-engine"), dep("criterion", [{"kind": "dev", "target": None}])]},
        {"id": "cfa4-cli-id", "deps": [dep("cfa4-engine"), dep("cc", [{"kind": "build", "target": None}])]},
        {"id": "cfa4-engine-id", "deps": [dep("serde"), dep("zlib-rs"), dep("anyhow"), dep("nofile")]},
        {"id": "serde-id", "deps": []},
        {"id": "zlib-rs-id", "deps": [dep("serde")]},
        {"id": "anyhow-id", "deps": []},
        {"id": "nofile-id", "deps": []},
        {"id": "criterion-id", "deps": []},
        {"id": "cc-id", "deps": []},
        {"id": "unrelated-id", "deps": []},
    ]
    return {
        "packages": packages,
        "workspace_members": ["cfa4-id", "cfa4-cli-id", "cfa4-engine-id", "criterion-id-not-member"],
        "resolve": {"nodes": nodes},
    }


def test_only_normal_dependencies_of_the_binaries_ship(tmp_path: Path) -> None:
    ids = shipped_package_ids(_metadata(tmp_path))
    assert ids == {"cfa4-id", "cfa4-cli-id", "cfa4-engine-id", "serde-id", "zlib-rs-id", "anyhow-id", "nofile-id"}


def test_workspace_code_is_left_out_and_texts_are_written_once(tmp_path: Path) -> None:
    crates, texts = collect(_metadata(tmp_path))
    assert [c.name for c in crates] == ["anyhow", "nofile", "serde", "zlib-rs"]
    by_name = {c.name: c for c in crates}
    # anyhow and serde share the Apache text: one copy, the same number
    assert by_name["anyhow"].files == [("LICENSE-APACHE", 1)]
    assert ("LICENSE-APACHE", 1) in by_name["serde"].files
    assert texts.count(APACHE) == 1
    assert by_name["nofile"].files == []
    # licence and notice files are found whatever their suffix; README and src/ are not
    assert [f for f, _ in by_name["zlib-rs"].files] == ["LICENSE", "NOTICE.txt"]
    assert "zlib License\n" in texts  # CRLF normalised


def test_render_is_deterministic_and_names_every_crate(tmp_path: Path) -> None:
    meta = _metadata(tmp_path)
    first = render(*collect(meta))
    meta["packages"].reverse()
    meta["resolve"]["nodes"].reverse()
    second = render(*collect(meta))
    assert first == second
    assert "Crates: 4. Licence texts: 4." in first
    assert "serde 1.0.0\n  Licence: MIT OR Apache-2.0\n  Source: https://example.invalid/serde" in first
    assert "nofile 1.0.0" in first and "ships no licence file" in first
    assert "Copyright (c) serde authors" in first
    assert "GPL-3.0-or-later" in first and "cfa4-engine" not in first


def test_missing_root_and_oversized_file_are_errors(tmp_path: Path) -> None:
    meta = _metadata(tmp_path)
    meta["workspace_members"].remove("cfa4-cli-id")
    with pytest.raises(NoticeError, match="cfa4-cli"):
        collect(meta)
    other = tmp_path / "other"
    other.mkdir()
    meta = _metadata(other)
    serde_dir = Path(next(p for p in meta["packages"] if p["name"] == "serde")["manifest_path"]).parent
    (serde_dir / "LICENSE-HUGE").write_bytes(b"x" * (300 * 1024))
    with pytest.raises(NoticeError, match="too large"):
        collect(meta)


def test_cli_writes_the_file_and_strict_fails_without_licence_files(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    meta = tmp_path / "metadata.json"
    crates = tmp_path / "crates"
    crates.mkdir()
    meta.write_text(json.dumps(_metadata(crates)), encoding="utf-8")
    out = tmp_path / "doc" / "THIRD-PARTY-LICENSES.txt"
    assert main([str(meta), "-o", str(out)]) == 0
    assert out.read_text(encoding="utf-8").startswith("CFA4 – third-party licence notices")
    assert "nofile 1.0.0" in capsys.readouterr().err
    assert main([str(meta), "-o", str(out), "--strict"]) == 1
    assert main([str(tmp_path / "missing.json"), "-o", str(out)]) == 1
