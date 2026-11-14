"""CFA4-PY-TYPING-001 T0: the mypy exception list in pyproject.toml may only shrink."""

from __future__ import annotations

import importlib.util
import tomllib
from pathlib import Path
from types import ModuleType

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ci" / "check-mypy-ratchet.py"


def _ratchet() -> ModuleType:
    spec = importlib.util.spec_from_file_location("check_mypy_ratchet", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _pyproject(tmp_path: Path, name: str, modules: list[str] | None) -> Path:
    text = '[tool.mypy]\nstrict = true\n'
    if modules is not None:
        listed = ", ".join(f'"{m}"' for m in modules)
        text += f"\n[[tool.mypy.overrides]]\nmodule = [{listed}]\nignore_errors = true\n"
    path = tmp_path / name
    path.write_text(text, encoding="utf-8")
    return path


def test_the_list_may_shrink(tmp_path: Path) -> None:
    head = _pyproject(tmp_path, "head.toml", ["a"])
    base = _pyproject(tmp_path, "base.toml", ["a", "b"])
    assert _ratchet().main([str(head), str(base)]) == 0


def test_the_list_may_not_grow(tmp_path: Path) -> None:
    head = _pyproject(tmp_path, "head.toml", ["a", "c"])
    base = _pyproject(tmp_path, "base.toml", ["a", "b"])
    assert _ratchet().main([str(head), str(base)]) == 1


def test_the_first_list_starts_it(tmp_path: Path) -> None:
    head = _pyproject(tmp_path, "head.toml", ["a", "b"])
    base = _pyproject(tmp_path, "base.toml", None)
    assert _ratchet().main([str(head), str(base)]) == 0


def test_an_unreadable_file_is_an_error(tmp_path: Path) -> None:
    head = _pyproject(tmp_path, "head.toml", ["a"])
    assert _ratchet().main([str(head), str(tmp_path / "missing.toml")]) == 2


def test_the_repository_list_names_existing_modules() -> None:
    """Every listed module is a file under python/ (a stale entry would hide nothing and never shrink)."""
    with (ROOT / "pyproject.toml").open("rb") as f:
        listed = _ratchet().exceptions(tomllib.load(f))
    missing = []
    for module in sorted(listed):
        parts = module.split(".")
        candidates = [ROOT / "python" / Path(*parts).with_suffix(".py"),
                      ROOT / "python" / Path(*parts) / "__init__.py",
                      ROOT / "python" / "tests" / f"{module}.py"]
        if not any(c.is_file() for c in candidates):
            missing.append(module)
    assert missing == []
