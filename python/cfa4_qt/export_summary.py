"""Translated export Summary lines.

The engine's ``export.resolve`` answers with English Summary lines built from comma-separated
fragments (``crates/export/src/settings.rs`` ``ExportSettings::summary``), for example
``1920x1080 (1.0), 25 fps, Progressive, High L4.1, CBR, 8.00 Mbps, keyframe every 50 frames``.
This module translates them fragment by fragment through the catalogs, with the language's
decimal separator in the numbers. A fragment it does not know (a codec or profile name such as
``ProRes 422 HQ``, or a fragment added to the engine later) is shown unchanged, so the Summary
is never wrong, at worst partly English. English shows the engine's text as it is.

Each pattern below matches one fragment the engine writes; its template is that fragment with
``{name}`` placeholders, and is the catalog key (``N_``) that translators see.
"""

from __future__ import annotations

import re

from .i18n import N_, TR

CONTEXT = "export.summary"
_NUM = r"\d+(?:\.\d+)?"

# fragments translated as a whole
WHOLE = (
    N_("No video"),
    N_("No audio"),
    N_("Progressive"),
    N_("Upper First"),
    N_("Lower First"),
    N_("HEVC Main / Main 10 for HDR"),
    N_("MPEG-2 4:2:2 Profile"),
    N_("MPEG-2 Main Profile 4:2:0"),
    N_("no B pictures"),
    N_("1 B picture between anchors"),
    N_("AV1 Main (10-bit for HDR)"),
    N_("10-bit RGB"),
    N_("12-bit X'Y'Z'"),
    N_("10-bit R'G'B' 4:4:4"),
    N_("JPEG 2000 IMF lossy"),
    N_("JPEG 2000 IMF reversible"),
    N_("16-bit half float"),
    N_("linear"),
    N_("uncompressed"),
    N_("Stereo"),
    N_("Mono"),
    N_("Film Scan 1"),
    N_("Film Scan 2"),
    N_("Film Scan 3"),
    # format and container names
    N_("Animated GIF"),
    N_("Waveform Audio"),
    N_("AAC Audio"),
    N_("Opus Audio"),
    N_("Dolby Digital (AC-3)"),
    N_("HLS (HTTP Live Streaming)"),
    N_("DCP (Digital Cinema Package)"),
    N_("fragmented MP4 segments"),
    N_("Program Stream"),
    N_("Transport Stream"),
    N_("BDAV Transport Stream"),
    N_("Elementary Stream"),
    N_("Image sequence"),
)

# words whose plain catalog entry means something else (Low: a deep sound); translated in
# CONTEXT only
CONTEXT_WORDS = ("High", "Medium", "Low")

# (fragment pattern, template); placeholders are the named groups
PATTERNS: tuple[tuple[re.Pattern[str], str], ...] = tuple(
    (re.compile(p), t)
    for p, t in (
        (rf"(?P<w>\d+)x(?P<h>\d+) \((?P<par>{_NUM})\)", N_("{w}x{h} ({par})")),
        (rf"(?P<fps>{_NUM}) fps", N_("{fps} fps")),
        (r"(?P<profile>Baseline|Main|High) auto level", N_("{profile} auto level")),
        (r"(?P<n>[12]) pass", N_("{n} pass")),
        (r"CRF \(constant quality\) (?P<crf>\d+)", N_("CRF (constant quality) {crf}")),
        (rf"(?P<v>{_NUM}) Mbps", N_("{v} Mbps")),
        (rf"Target (?P<v>{_NUM}) Mbps", N_("Target {v} Mbps")),
        (rf"Max (?P<v>{_NUM}) Mbps", N_("Max {v} Mbps")),
        (rf"average (?P<v>{_NUM}) Mbps", N_("average {v} Mbps")),
        (rf"CBR (?P<v>{_NUM}) Mbps", N_("CBR {v} Mbps")),
        (rf"VBR Target (?P<v>{_NUM}) Mbps", N_("VBR Target {v} Mbps")),
        (r"keyframe every (?P<n>\d+) frames", N_("keyframe every {n} frames")),
        (r"constant quantiser (?P<q>\d+)", N_("constant quantiser {q}")),
        (r"(?P<n>\d+) B pictures between anchors", N_("{n} B pictures between anchors")),
        (r"quantizer (?P<q>\d+)", N_("quantizer {q}")),
        (r"quality (?P<q>\d+)", N_("quality {q}")),
        (r"(?P<word>.+) compression", N_("{word} compression")),
        (r"(?P<sampling>YUV 4:2:2|RGB 4:4:4|RGBA 4:4:4:4) (?P<bits>\d+)-bit", N_("{sampling} {bits}-bit")),
        (r"FLAC \(lossless\) (?P<bits>\d+)-bit", N_("FLAC (lossless) {bits}-bit")),
        (r"level (?P<n>\d+)", N_("level {n}")),
        (r"Uncompressed (?P<bits>\d+)-bit PCM", N_("Uncompressed {bits}-bit PCM")),
        (r"Uncompressed (?P<bits>\d+)-bit float PCM", N_("Uncompressed {bits}-bit float PCM")),
        (rf"(?P<aspect>Flat|Scope|Full) (?P<ratio>{_NUM}):1", N_("{aspect} {ratio}:1")),
    )
)

# placeholders holding a word (translated) rather than a number (decimal separator)
_WORD_GROUPS = {"word", "profile", "sampling", "aspect"}
_MAX_LINE = 4096


def _word(text: str) -> str:
    if text in CONTEXT_WORDS:
        return TR.tr_ctx(CONTEXT, text)
    return TR.tr(text)


def fragment(text: str) -> str:
    """One fragment, translated; unknown fragments unchanged."""
    if text in WHOLE or text in CONTEXT_WORDS:
        return _word(text)
    for pattern, template in PATTERNS:
        m = pattern.fullmatch(text)
        if m is None:
            continue
        if not TR.has(template):
            # no translation in this language: the English fragment, numbers as they are
            return text
        out = TR.tr(template)
        for key, value in m.groupdict().items():
            out = out.replace("{" + key + "}", _word(value) if key in _WORD_GROUPS else TR.decimal(value))
        return out
    return text


def line(text: str) -> str:
    """A Video or Audio Summary line (or a DCP aspect such as ``SMPTE, Flat 1.85:1``)."""
    if TR.language == "en" or not text or len(text) > _MAX_LINE:
        return text
    return ", ".join(fragment(part) for part in text.split(", "))


def format_line(text: str) -> str:
    """The Format line: ``H.264 (MP4)``, ``MPEG-2 (Program Stream)``, ``Animated GIF``."""
    if TR.language == "en" or not text or len(text) > _MAX_LINE:
        return text
    if text in WHOLE:
        return _word(text)
    cut = text.rfind(" (")
    if cut <= 0 or not text.endswith(")"):
        return text
    label, container = text[:cut], text[cut + 2 : -1]
    return f"{_word(label) if label in WHOLE else label} ({line(container)})"


def size(text: str) -> str:
    """An estimated size such as ``1.2 GB``."""
    number, sep, unit = text.partition(" ")
    return f"{TR.decimal(number)}{sep}{unit}" if sep else text
