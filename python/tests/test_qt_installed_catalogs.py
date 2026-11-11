"""An installed Qt GUI translates as a checkout does (parity row 1.10).

A package has no checkout, so packaging/linux/package.sh copies the top-level ``i18n/`` catalogs
into ``cfa4_qt/i18n``; ``cfa4_qt.i18n`` reads them from there when the checkout's are not beside
it. No Qt import: this runs in the plain Python CI job.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def test_installed_gui_reads_the_catalogs_from_its_package(tmp_path: Path):
    installed = tmp_path / "usr/lib/cfa4/python"
    shutil.copytree(ROOT / "python/cfa4_qt", installed / "cfa4_qt", ignore=shutil.ignore_patterns("__pycache__"))
    catalogs = installed / "cfa4_qt/i18n"
    catalogs.mkdir()
    shutil.copy(ROOT / "i18n/de.tsv", catalogs)
    shutil.copy(ROOT / "i18n/hu.tsv", catalogs)
    probe = (
        "from cfa4_qt.i18n import CATALOG_DIR, Translator; print(CATALOG_DIR); "
        "print(Translator('de').tr('About CFA4')); print(Translator('hu').tr('All files'))"
    )
    out = subprocess.run(
        [sys.executable, "-I", "-c", f"import sys; sys.path.insert(0, {str(installed)!r}); {probe}"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        timeout=60,
        check=True,
    ).stdout.splitlines()
    assert out == [str(catalogs), "Über CFA4", "Minden fájl"], out


def test_a_checkout_reads_the_top_level_catalogs_in_place():
    from cfa4_qt.i18n import CATALOG_DIR

    assert CATALOG_DIR == ROOT / "i18n"
