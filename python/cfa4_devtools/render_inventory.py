"""CFA4-RENDER-INTEGRATION-PLAN-001 F0a: machine-generated render call graph and type map.

Reads only the source tree (Cargo.toml manifests and ``.rs`` files of the render-related crates),
never builds, runs or installs anything, and writes a deterministic JSON and Markdown inventory:

* the workspace-internal crate dependency graph of the render area,
* module-level use edges (``crate/module.rs`` -> ``cfa4_<crate>::<item>``) between those crates,
* the public types of the area, classified (frame/image, time, job/queue, audio, colour, render),
* the public render / export / preview / proxy entry-point functions.

It is a static, module-level graph, not a function-level call graph: a reference to another
crate's item is an edge, whether or not it is called on a given path. The inventory is
regenerated with one command and ``--check`` fails when the committed copy is stale::

    python3 python/cfa4_devtools/render_inventory.py            # write docs/audit/render-f0a/
    python3 python/cfa4_devtools/render_inventory.py --check    # exit 1 if out of date

Standard library only (Python >= 3.12, ``tomllib``), so it runs on the Ubuntu system Python
without a virtual environment.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

SCHEMA = "cfa4.render-inventory/1"

# The render area of the plan (F0a task 4: renderer, timeline, render queue, proxy, preview,
# audio, GPU paths) plus the crates its exit criteria measure against. Directory names.
AREA: tuple[str, ...] = (
    "time",
    "geom",
    "color",
    "frame",
    "media",
    "project",
    "audio-dsp",
    "route",
    "render",
    "gpu",
    "export",
    "smart-edit",
    "qc",
    "metrics",
    "golden",
    "testkit",
    "ffmpeg",
    "gstreamer",
    "engine",
)
APPS: tuple[str, ...] = ("cfa4-cli",)

TYPE_CLASSES: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("job", re.compile(r"Job|Queue|Progress|Cancel|Exporter|Task|Status|Step|Worker|Pace")),
    ("time", re.compile(r"Tick|FrameRate|TimeRange|Timecode|TimeDisplay|Clock|Rate$")),
    ("audio", re.compile(r"Audio|Sample|Channel|Loudness|Mixer|Bus|Pan")),
    ("frame", re.compile(r"Frame|Image|Pixel|Plane|Chroma|Layout|Region|Raster|Surface|Texture")),
    ("colour", re.compile(r"Colou?r|Primaries|Transfer|Matrix|Lut|Hdr|Tone")),
    ("render", re.compile(r"Render|Composit|Blend|Plan|Preview|Proxy|Effect|Transition|Source")),
)
ENTRY = re.compile(r"render|export|preview|proxy|encode|mux|mix|composite")

PUB_TYPE = re.compile(r"^pub (struct|enum|trait|type) ([A-Z][A-Za-z0-9_]*)", re.M)
PUB_FN = re.compile(r"^pub fn ([a-z_][a-z0-9_]*)", re.M)
USE_REF = re.compile(r"\bcfa4_([a-z0-9_]+)::(?:\{)?([A-Za-z_][A-Za-z0-9_]*)")


@dataclass
class Crate:
    package: str
    directory: str
    description: str
    deps: list[str] = field(default_factory=list)
    dev_deps: list[str] = field(default_factory=list)


def _workspace_deps(table: dict[str, Any]) -> list[str]:
    return sorted(name for name in table if name.startswith("cfa4-"))


def read_crate(root: Path, rel: str) -> Crate | None:
    manifest = root / rel / "Cargo.toml"
    if not manifest.is_file():
        return None
    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    pkg = data.get("package", {})
    return Crate(
        package=str(pkg.get("name", "")),
        directory=rel,
        description=str(pkg.get("description", "")),
        deps=_workspace_deps(data.get("dependencies", {})),
        dev_deps=_workspace_deps(data.get("dev-dependencies", {})),
    )


def _is_test_file(rel: str) -> bool:
    name = rel.rsplit("/", 1)[-1]
    return (
        name == "tests.rs"
        or name.endswith("_tests.rs")
        or "/tests/" in rel
        or "/benches/" in rel
        or "/examples/" in rel
    )


def _strip_comments(text: str) -> str:
    return "\n".join(line for line in text.splitlines() if not line.lstrip().startswith("//"))


def classify(name: str) -> str:
    for label, pattern in TYPE_CLASSES:
        if pattern.search(name):
            return label
    return "other"


def build(root: Path) -> dict[str, Any]:
    crates: dict[str, Crate] = {}
    for rel in [f"crates/{d}" for d in AREA] + [f"apps/{a}" for a in APPS]:
        crate = read_crate(root, rel)
        if crate is not None:
            crates[crate.package] = crate
    area = set(crates)
    by_ident = {name.removeprefix("cfa4-").replace("-", "_"): name for name in area}

    crate_edges = sorted({(c.package, d) for c in crates.values() for d in c.deps if d in area})
    dev_edges = sorted({(c.package, d) for c in crates.values() for d in c.dev_deps if d in area})

    use_edges: set[tuple[str, str, str]] = set()
    types: list[dict[str, str]] = []
    entries: list[dict[str, str]] = []
    for crate in sorted(crates.values(), key=lambda c: c.package):
        src = root / crate.directory / "src"
        for path in sorted(src.rglob("*.rs")):
            rel = path.relative_to(root).as_posix()
            if _is_test_file(rel):
                continue
            text = _strip_comments(path.read_text(encoding="utf-8", errors="replace"))
            module = path.relative_to(src).as_posix()
            for ident, item in USE_REF.findall(text):
                target = by_ident.get(ident)
                if target is not None and target != crate.package:
                    use_edges.add((f"{crate.package}/{module}", target, item))
            for kind, name in PUB_TYPE.findall(text):
                types.append(
                    {"crate": crate.package, "module": module, "kind": kind, "name": name, "class": classify(name)}
                )
            for name in PUB_FN.findall(text):
                if ENTRY.search(name):
                    entries.append({"crate": crate.package, "module": module, "fn": name})

    return {
        "schema": SCHEMA,
        "scope": {"crates": sorted(area), "excludes": "test, bench and example files; comment lines"},
        "crates": [
            {"package": c.package, "directory": c.directory, "description": c.description}
            for c in sorted(crates.values(), key=lambda c: c.package)
        ],
        "crate_edges": [{"from": a, "to": b} for a, b in crate_edges],
        "dev_crate_edges": [{"from": a, "to": b} for a, b in dev_edges],
        "use_edges": [{"from": a, "to": b, "item": i} for a, b, i in sorted(use_edges)],
        "types": sorted(types, key=lambda t: (t["class"], t["crate"], t["module"], t["name"])),
        "entry_points": sorted(entries, key=lambda e: (e["crate"], e["module"], e["fn"])),
    }


def _node(name: str) -> str:
    return name.replace("-", "_")


def render_markdown(inv: dict[str, Any]) -> str:
    out: list[str] = [
        "# Render-terület: géppel generált call graph és típustérkép (F0a)",
        "",
        "> **Generált fájl, kézzel ne szerkeszd.** Forrás: `python/cfa4_devtools/render_inventory.py`",
        "> (`CFA4-RENDER-INTEGRATION-PLAN-001`, F0a 4. feladat). Újragenerálás:",
        "> `python3 python/cfa4_devtools/render_inventory.py`; frissesség: `--check`.",
        "> Statikus, modulszintű gráf: egy másik crate elemére való hivatkozás él, függetlenül attól,",
        "> hogy egy adott úton meghívódik-e. Tesztek, benchek, példák és kommentsorok kimaradnak.",
        "",
        "## 1. Crate-függőségek (workspace-en belül, a render-területen)",
        "",
        "```mermaid",
        "graph LR",
    ]
    for e in inv["crate_edges"]:
        out.append(f"  {_node(e['from'])} --> {_node(e['to'])}")
    out += ["```", "", "| Crate | Könyvtár | Leírás |", "|---|---|---|"]
    for c in inv["crates"]:
        out.append(f"| `{c['package']}` | `{c['directory']}` | {c['description']} |")

    out += ["", "## 2. Modulszintű hivatkozások crate-ek között", ""]
    grouped: dict[str, dict[str, list[str]]] = {}
    for e in inv["use_edges"]:
        grouped.setdefault(e["from"], {}).setdefault(e["to"], []).append(e["item"])
    out += ["| Modul | Hivatkozott crate | Elemek |", "|---|---|---|"]
    for module in sorted(grouped):
        for target in sorted(grouped[module]):
            items = ", ".join(f"`{i}`" for i in sorted(set(grouped[module][target])))
            out.append(f"| `{module}` | `{target}` | {items} |")

    out += ["", "## 3. Belépési pontok (nyilvános render/export/preview/proxy függvények)", ""]
    out += ["| Crate | Modul | Függvény |", "|---|---|---|"]
    for e in inv["entry_points"]:
        out.append(f"| `{e['crate']}` | `{e['module']}` | `{e['fn']}` |")

    out += ["", "## 4. Nyilvános típusok osztályonként", ""]
    labels = {
        "frame": "Képkocka, kép, pixel",
        "time": "Idő",
        "job": "Job, sor, folyamat, megszakítás",
        "audio": "Hang",
        "colour": "Szín",
        "render": "Render, kompozíció, előnézet, proxy",
    }
    other = sum(1 for t in inv["types"] if t["class"] == "other")
    out += [f"A többi {other} nyilvános típus („other” osztály) csak a `call-graph.json`-ban szerepel.", ""]
    for cls, label in labels.items():
        rows = [t for t in inv["types"] if t["class"] == cls]
        if not rows:
            continue
        out += [f"### {label} ({len(rows)})", "", "| Crate | Modul | Fajta | Típus |", "|---|---|---|---|"]
        for t in rows:
            out.append(f"| `{t['crate']}` | `{t['module']}` | {t['kind']} | `{t['name']}` |")
        out.append("")
    return "\n".join(out).rstrip() + "\n"


def outputs(root: Path) -> dict[Path, str]:
    inv = build(root)
    base = root / "docs" / "audit" / "render-f0a"
    return {
        base / "call-graph.json": json.dumps(inv, indent=1, ensure_ascii=False, sort_keys=True) + "\n",
        base / "call-graph.md": render_markdown(inv),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else None)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2], help="repository root")
    parser.add_argument("--check", action="store_true", help="fail if the committed inventory is out of date")
    args = parser.parse_args(argv)
    files = outputs(args.root)
    if args.check:
        stale = [p for p, text in files.items() if not p.is_file() or p.read_text(encoding="utf-8") != text]
        for p in stale:
            print(f"stale: {p.relative_to(args.root)}", file=sys.stderr)
        return 1 if stale else 0
    for path, text in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        print(path.relative_to(args.root))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
