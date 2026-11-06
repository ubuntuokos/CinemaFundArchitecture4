"""An installed Qt GUI translates as a checkout does (parity row 1.10).

A package has no ``crates/`` tree, so packaging/linux/package.sh copies egui's shared catalogs
into ``cfa4_qt/i18n/shared``; ``cfa4_qt.i18n`` reads them from there when the checkout's are not
beside it. No Qt import: this runs in the plain Python CI job.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def test_installed_gui_reads_the_shared_catalogs_from_its_package(tmp_path: Path):
    installed = tmp_path / "usr/lib/cfa4/python"
    shutil.copytree(ROOT / "python/cfa4_qt", installed / "cfa4_qt", ignore=shutil.ignore_patterns("__pycache__"))
    shared = installed / "cfa4_qt/i18n/shared"
    shared.mkdir()
    shutil.copy(ROOT / "crates/ui-egui/src/i18n/de.tsv", shared)
    probe = "from cfa4_qt.i18n import CATALOG_DIRS, Translator; print(CATALOG_DIRS[1]); print(Translator('de').tr('About CFA4'))"
    out = subprocess.run(
        [sys.executable, "-I", "-c", f"import sys; sys.path.insert(0, {str(installed)!r}); {probe}"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        timeout=60,
        check=True,
    ).stdout.splitlines()
    assert out == [str(shared), "Über CFA4"], out


def test_a_checkout_reads_egui_catalogs_in_place():
    from cfa4_qt.i18n import CATALOG_DIRS

    assert CATALOG_DIRS[1] == ROOT / "crates/ui-egui/src/i18n"
