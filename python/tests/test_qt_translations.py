"""The Qt GUI is translated into every finished language (parity row 1.8).

The Qt GUI reads the shared catalogs (`crates/ui-egui/src/i18n/`) plus its own
(`python/cfa4_qt/i18n/`) for the strings only it has. Russian and Ukrainian are finished since
2026-11-09 and checked like the others. Hungarian must cover every string
(`test_hungarian_catalog_covers_the_qt_gui`); the others must cover at least COVERAGE of them, so a
new string can land before its translations do. No Qt import: this runs in the plain Python CI
job.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
from cfa4_qt.i18n import LANGUAGES, Translator, parse_catalog

QT = Path(__file__).resolve().parents[1] / "cfa4_qt"
PARTIAL = {"en"}
FINISHED = sorted(set(LANGUAGES) - PARTIAL)
COVERAGE = 0.98
PLACEHOLDER = re.compile(r"\{\w+\}")


def _qt_literals() -> set[str]:
    """Every literal passed to tr()/fmt()/N_() in the Qt GUI."""
    found: set[str] = set()
    for path in QT.rglob("*.py"):
        src = path.read_text(encoding="utf-8")
        found.update(m.group(1) for m in re.finditer(r'\b(?:tr|fmt|N_)\(\s*"((?:[^"\\]|\\.)*)"', src))
        found.update(m.group(1) for m in re.finditer(r"\b(?:tr|fmt|N_)\(\s*'((?:[^'\\]|\\.)*)'", src))
    return found


def test_the_qt_gui_has_strings_to_check():
    assert len(_qt_literals()) > 1000


@pytest.mark.parametrize("code", FINISHED)
def test_the_qt_strings_are_translated(code: str):
    tr = Translator(code)
    assert tr.language == code
    literals = _qt_literals()
    missing = sorted(s for s in literals if not tr.has(s))
    assert len(missing) <= (1 - COVERAGE) * len(literals), missing


@pytest.mark.parametrize("code", FINISHED)
def test_qt_translations_keep_their_placeholders(code: str):
    """A translation that drops or renames {name} would show the raw field or raise in fmt()."""
    tr = Translator(code)
    bad = [s for s in _qt_literals() if sorted(PLACEHOLDER.findall(s)) != sorted(PLACEHOLDER.findall(tr.tr(s)))]
    assert not bad, sorted(bad)


@pytest.mark.parametrize("path", sorted((QT / "i18n").glob("*.tsv")), ids=lambda p: p.stem)
def test_qt_catalogs_are_clean(path: Path):
    """Each Qt catalog says it is clean-room work and every line parses."""
    text = path.read_text(encoding="utf-8")
    assert "Clean-room" in text.split("\n\n", 1)[0]
    lines = [line for line in text.splitlines() if line.strip() and not line.startswith("#")]
    plain, contextual = parse_catalog(text)
    assert len(plain) + len(contextual) == len(lines), "malformed or duplicate lines"
