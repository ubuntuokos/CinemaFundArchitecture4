"""Translation catalogs for the Qt GUI.

Same TSV format as the shared catalogs (`context <TAB> source <TAB> translation`, documented at
the top of ``crates/ui-egui/src/i18n/es.tsv``). The Qt GUI reads the shared catalogs, so menu
and command labels translate the same way in both frontends while egui still exists, plus its own
catalogs in ``python/cfa4_qt/i18n/`` (Hungarian, and Qt-only strings). A missing or malformed
catalog never breaks the UI: the English source text is shown instead.
"""

from __future__ import annotations

import locale
import os
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
CATALOG_DIRS = (HERE / "i18n", ROOT / "crates" / "ui-egui" / "src" / "i18n")

# language code -> name shown in the View ▸ Language menu (in that language)
LANGUAGES = {
    "en": "English",
    "hu": "Magyar",
    "de": "Deutsch",
    "es": "Español",
    "fr": "Français",
    "it": "Italiano",
    "pt-br": "Português (Brasil)",
    "id": "Bahasa Indonesia",
    "ja": "日本語",
    "ko": "한국어",
    "zh-cn": "简体中文",
    "ru": "Русский",
    "uk": "Українська",
}


def _unescape(text: str) -> str:
    out: list[str] = []
    chars = iter(text)
    for c in chars:
        if c != "\\":
            out.append(c)
            continue
        nxt = next(chars, None)
        if nxt == "n":
            out.append("\n")
        elif nxt == "t":
            out.append("\t")
        elif nxt == "\\" or nxt is None:
            out.append("\\")
        else:
            out.append("\\" + nxt)
    return "".join(out)


def parse_catalog(text: str) -> tuple[dict[str, str], dict[tuple[str, str], str]]:
    """Plain and contextual entries of one catalog file. Malformed lines are skipped."""
    plain: dict[str, str] = {}
    contextual: dict[tuple[str, str], str] = {}
    for line in text.splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        cols = line.split("\t")
        if len(cols) != 3 or not cols[1] or not cols[2]:
            continue
        context, source, translation = (_unescape(c) for c in cols)
        if context:
            contextual[(context, source)] = translation
        else:
            plain[source] = translation
    return plain, contextual


class Translator:
    def __init__(self, language: str = "en") -> None:
        self.language = "en"
        self._plain: dict[str, str] = {}
        self._contextual: dict[tuple[str, str], str] = {}
        self.set_language(language)

    def set_language(self, language: str) -> None:
        language = language if language in LANGUAGES else "en"
        self.language = language
        self._plain, self._contextual = {}, {}
        if language == "en":
            return
        # later directories never override earlier ones: the Qt catalog wins over the shared one
        for directory in reversed(CATALOG_DIRS):
            path = directory / f"{language}.tsv"
            try:
                text = path.read_text(encoding="utf-8")
            except (OSError, UnicodeDecodeError):
                continue
            plain, contextual = parse_catalog(text)
            self._plain.update(plain)
            self._contextual.update(contextual)

    def tr(self, text: str) -> str:
        return self._plain.get(text, text)

    def has(self, text: str) -> bool:
        """Does the catalog translate ``text``? (Also true for entries equal to the source.)"""
        return text in self._plain

    def tr_ctx(self, context: str, text: str) -> str:
        return self._contextual.get((context, text)) or self.tr(text)

    def fmt(self, template: str, **args: object) -> str:
        """Translate ``template`` and fill its ``{name}`` placeholders; unknown ones stay."""
        out = self.tr(template)
        for key, value in args.items():
            out = out.replace("{" + key + "}", str(value))
        return out


def system_language() -> str:
    """The best catalog for the user's locale (``CFA4_LANG`` overrides)."""
    wanted = (
        os.environ.get("CFA4_LANG")
        or os.environ.get("LC_ALL")
        or os.environ.get("LC_MESSAGES")
        or os.environ.get("LANG")
    )
    if not wanted:
        try:
            wanted = locale.getlocale()[0] or ""
        except ValueError:
            wanted = ""
    tag = wanted.split(".")[0].replace("_", "-").lower()
    if tag in LANGUAGES:
        return tag
    base = tag.split("-")[0]
    if base == "pt":
        return "pt-br"
    if base == "zh":
        return "zh-cn"
    return base if base in LANGUAGES else "en"


TR = Translator("en")


def N_(text: str) -> str:
    """Marks a literal for translation where it is translated later (``tr(label)``); returns it
    unchanged. The catalog-coverage test collects ``N_("…")`` literals as well as ``tr("…")``."""
    return text


def tr(text: str) -> str:
    return TR.tr(text)


def fmt(template: str, **args: object) -> str:
    return TR.fmt(template, **args)
