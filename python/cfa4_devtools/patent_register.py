"""The patent review register (docs/patent-review.md, ADR 0003 §13): parsing and checks.

No network; reads the register and the crate manifests only. Used by the release gate
(`release_gate.py`: the `patent_review_recorded` gate cannot PASS while a base-package row is
pending) and by `python/tests/test_patent_register.py` (every codec / container crate has a row).
"""
from __future__ import annotations

import re
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REGISTER = ROOT / "docs" / "patent-review.md"
COLUMNS = ("Formátum", "Irány", "Komponens", "Csomag", "Eredmény", "Dátum", "Vizsgáló / forrás")
RESULTS = frozenset({"függőben", "szabad", "korlátozott", "tisztázatlan"})
# crates whose manifest description names decoding, encoding or (de)muxing: each needs a row
FORMAT_CRATE = re.compile(r"decod|encod|demux|muxer|container|reader and writer", re.IGNORECASE)
# crates the pattern matches that implement no media format of their own (they call the codec
# crates, or only describe frames); a reason each
NOT_A_FORMAT = {
    "export": "the export pipeline drives the encoder crates listed in the register",
    "frame": "video frames and audio buffers, no bitstream",
    "media": "decoding traits, probing and the frame cache; the formats are the codec crates'",
    "golden": "test-only golden images",
    "testkit": "test-only helpers",
}


class RegisterError(ValueError):
    pass


@dataclass(frozen=True)
class Row:
    format: str
    direction: str
    components: str
    package: str
    result: str
    date: str
    source: str

    @property
    def in_base(self) -> bool:
        return "alap" in self.package

    @property
    def pending(self) -> bool:
        return self.result == "függőben"


def parse(text: str) -> list[Row]:
    """Rows of every register table; malformed rows are errors, not skipped."""
    rows: list[Row] = []
    header: list[str] | None = None
    for number, line in enumerate(text.splitlines(), 1):
        if not line.startswith("|"):
            header = None
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if header is None:
            if tuple(cells) != COLUMNS:
                raise RegisterError(f"line {number}: table columns must be {' | '.join(COLUMNS)}")
            header = cells
            continue
        if all(set(c) <= set("-: ") for c in cells):
            continue
        if len(cells) != len(COLUMNS):
            raise RegisterError(f"line {number}: {len(cells)} cells, expected {len(COLUMNS)}")
        row = Row(*cells)
        if row.result not in RESULTS:
            raise RegisterError(f"line {number}: result {row.result!r} is not one of {sorted(RESULTS)}")
        if not row.pending:
            if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", row.date):
                raise RegisterError(f"line {number}: a recorded result needs its date (YYYY-MM-DD)")
            if not row.source:
                raise RegisterError(f"line {number}: a recorded result needs who reviewed it and the source")
        rows.append(row)
    if not rows:
        raise RegisterError("the register has no rows")
    return rows


def load(path: Path = REGISTER) -> list[Row]:
    return parse(path.read_text(encoding="utf-8"))


def pending_in_base(rows: list[Row]) -> list[str]:
    """`format (direction)` of every base-package row without a recorded result."""
    return [f"{r.format} ({r.direction})" for r in rows if r.in_base and r.pending]


def format_crates(root: Path = ROOT) -> list[str]:
    names = []
    for manifest in sorted(root.glob("crates/*/Cargo.toml")):
        package = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package", {})
        description = package.get("description", "")
        name = manifest.parent.name
        if isinstance(description, str) and FORMAT_CRATE.search(description) and name not in NOT_A_FORMAT:
            names.append(name)
    return names


def missing_crates(rows: list[Row], root: Path = ROOT) -> list[str]:
    named = " ".join(r.components for r in rows)
    return [n for n in format_crates(root) if f"`crates/{n}`" not in named]


def main() -> int:
    try:
        rows = load()
    except (OSError, RegisterError) as exc:
        print(f"FAIL patent register: {exc}", file=sys.stderr)
        return 2
    missing = missing_crates(rows)
    pending = pending_in_base(rows)
    print(f"{len(rows)} rows, {len(pending)} base-package rows pending, {len(missing)} format crates missing")
    for name in missing:
        print(f"missing row for crates/{name}", file=sys.stderr)
    return 1 if missing else 0


if __name__ == "__main__":
    raise SystemExit(main())
