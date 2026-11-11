"""Translation catalogs for the Qt GUI.

Every CFA4 translation lives in the repository's top-level ``i18n/<code>.tsv``, one catalog per
language shared with egui (owner decision 2026-11-09), so menu and command labels translate the
same way in both frontends. Format: ``context <TAB> source <TAB> translation``, documented at the
top of ``i18n/es.tsv``. An installed package has no checkout, so packaging/linux/package.sh copies
the catalogs into ``cfa4_qt/i18n``. A missing or malformed catalog never breaks the UI: the English
source text is shown instead.
"""

from __future__ import annotations

import locale
import os
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
# the catalogs: an installed package's own copy (packaging/linux/package.sh), else the checkout's
# top-level i18n/ (a checkout has no python/cfa4_qt/i18n/)
PACKAGED_CATALOGS = HERE / "i18n"
CATALOG_DIR = PACKAGED_CATALOGS if PACKAGED_CATALOGS.is_dir() else ROOT / "i18n"

# language code -> name shown in the View ▸ Language menu (in that language)
LANGUAGES = {
    "en": "English (US)",
    "en-gb": "English (UK)",
    "en-au": "English (Australia)",
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
    "pl": "Polski",
    "tr": "Türkçe",
    "nl": "Nederlands",
    "cs": "Čeština",
    "vi": "Tiếng Việt",
    "zh-hant": "繁體中文",
    "el": "Ελληνικά",
    "sv": "Svenska",
    "sr-latn": "Srpski (latinica)",
    "bg": "Български",
    "ka": "ქართული",
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
        # a regional variant reads its base catalog first (Australian English on top of British)
        for code in (BASE_CATALOG.get(language), language):
            if code is None:
                continue
            try:
                text = (CATALOG_DIR / f"{code}.tsv").read_text(encoding="utf-8")
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

    def decimal(self, value: str) -> str:
        """``value`` (a plain decimal such as ``4.00``) with the language's decimal separator:
        ``4,00`` in Hungarian. Anything that is not a plain decimal comes back unchanged."""
        if self.language in DECIMAL_COMMA and _DECIMAL.fullmatch(value):
            return value.replace(".", ",")
        return value


# regional variants that read another catalog first, their own entries winning
BASE_CATALOG = {"en-au": "en-gb"}
# system locales with no catalog of their own that are shown in a close variant
REGIONAL_FALLBACK = {"en-nz": "en-au", "en-ie": "en-gb"}

# languages that write 4,00 instead of 4.00
DECIMAL_COMMA = frozenset({"hu", "ka", "bg", "sr-latn", "sv", "el", "vi", "cs", "nl", "tr", "pl", "de", "es", "fr", "it", "pt-br", "id", "ru", "uk"})
_DECIMAL = re.compile(r"-?\d+\.\d+")


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
    if tag in REGIONAL_FALLBACK:
        return REGIONAL_FALLBACK[tag]
    base = tag.split("-")[0]
    if base == "pt":
        return "pt-br"
    if base == "sr":
        # Serbian (sr-Latn, sr_RS@latin, sr-RS…): the Latin-script catalog
        return "sr-latn"
    if base == "zh":
        # Traditional script or a Traditional region (zh-TW, zh-HK, zh-MO, zh-Hant…); else Simplified
        traditional = {"hant", "tw", "hk", "mo"}
        return "zh-hant" if traditional & set(tag.split("-")[1:]) else "zh-cn"
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


def decimal(value: str) -> str:
    return TR.decimal(value)
