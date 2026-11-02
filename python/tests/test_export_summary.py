"""The export Summary lines (engine English) shown translated in the Qt GUI; no Qt needed."""

from __future__ import annotations

import re
from collections.abc import Iterator
from pathlib import Path

import pytest
from cfa4_qt import export_summary as es
from cfa4_qt.i18n import TR, Translator

ROOT = Path(__file__).resolve().parents[2]


@pytest.fixture
def hu() -> Iterator[None]:
    TR.set_language("hu")
    try:
        yield
    finally:
        TR.set_language("en")


# engine text (crates/export/src/settings.rs ExportSettings::summary) -> Hungarian
VIDEO = [
    (
        "1920x1080 (1.0), 25 fps, Progressive, High L4.1, CBR, 8.00 Mbps, keyframe every 50 frames",
        "1920×1080 (1,0), 25 fps, Progresszív, High L4.1, CBR, 8,00 Mbps, kulcskocka 50 képkockánként",
    ),
    (
        "1280x720 (1.0), 29.97 fps, Upper First, Main auto level, VBR, 2 pass, Target 4.50 Mbps, Max 6.75 Mbps, keyframe every 60 frames",
        "1280×720 (1,0), 29,97 fps, Felső mező először, Main, automatikus szint, VBR, 2 menet, cél 4,50 Mbps, max. 6,75 Mbps, kulcskocka 60 képkockánként",
    ),
    (
        "3840x2160 (1.0), 50 fps, Progressive, HEVC Main / Main 10 for HDR, CRF (constant quality) 23, keyframe every 100 frames",
        "3840×2160 (1,0), 50 fps, Progresszív, HEVC Main / Main 10 (HDR-hez), CRF (állandó minőség) 23, kulcskocka 100 képkockánként",
    ),
    (
        "720x576 (1.07), 25 fps, Lower First, MPEG-2 Main Profile 4:2:0, average 6.00 Mbps, keyframe every 12 frames, 2 B pictures between anchors",
        "720×576 (1,07), 25 fps, Alsó mező először, MPEG-2 Main profil, 4:2:0, átlag 6,00 Mbps, kulcskocka 12 képkockánként, 2 B-kép a horgonyképek között",
    ),
    (
        "1920x1080 (1.0), 25 fps, Progressive, MPEG-2 4:2:2 Profile, constant quantiser 2, keyframe every 15 frames, no B pictures",
        "1920×1080 (1,0), 25 fps, Progresszív, MPEG-2 4:2:2 profil, állandó kvantáló 2, kulcskocka 15 képkockánként, B-képek nélkül",
    ),
    # #278 (MPEG-2 CBR / VBV-constrained VBR)
    ("CBR 4.00 Mbps, 1 B picture between anchors", "CBR 4,00 Mbps, 1 B-kép a horgonyképek között"),
    ("VBR Target 4.00 Mbps, Max 6.00 Mbps", "VBR, cél 4,00 Mbps, max. 6,00 Mbps"),
    ("AV1 Main (10-bit for HDR), VBR, 1 pass", "AV1 Main (HDR-hez 10 bites), VBR, 1 menet"),
    ("VP9 Profile 0, quantizer 31", "VP9 Profile 0, kvantáló 31"),
    ("ProRes 422 HQ", "ProRes 422 HQ"),
    ("YUV 4:2:2 10-bit, Film Scan 1", "YUV 4:2:2 10 bites, Filmszkennelés 1"),
    ("RGBA 4:4:4:4 12-bit, Low", "RGBA 4:4:4:4 12 bites, alacsony"),
    ("quality 90", "minőség 90"),
    ("10-bit RGB", "10 bites RGB"),
    ("JPEG 2000 DCI 4K, 12-bit X'Y'Z'", "JPEG 2000 DCI 4K, 12 bites X'Y'Z'"),
    ("16-bit half float, linear, uncompressed", "16 bites, félpontos lebegőpontos, lineáris, tömörítetlen"),
    (
        "16-bit half float, linear, ZIP (16 scan lines) compression",
        "16 bites, félpontos lebegőpontos, lineáris, ZIP (16 sor) tömörítés",
    ),
    ("No video", "Nincs videó"),
]

AUDIO = [
    ("AAC, 320 kbps, 48000 Hz, Stereo", "AAC, 320 kbps, 48000 Hz, Sztereó"),
    ("Opus, 128 kbps, 48000 Hz, Mono", "Opus, 128 kbps, 48000 Hz, Monó"),
    ("FLAC (lossless) 24-bit, level 5, 48000 Hz, 5.1", "FLAC (veszteségmentes) 24 bites, 5. szint, 48000 Hz, 5.1"),
    ("Uncompressed 24-bit PCM, 48000 Hz, Stereo", "tömörítetlen 24 bites PCM, 48000 Hz, Sztereó"),
    ("Uncompressed 32-bit float PCM, 96000 Hz, Stereo", "tömörítetlen 32 bites lebegőpontos PCM, 96000 Hz, Sztereó"),
    ("No audio", "Nincs hang"),
]

FORMAT = [
    ("H.264 (MP4)", "H.264 (MP4)"),
    ("H.265 (HEVC) (QuickTime)", "H.265 (HEVC) (QuickTime)"),
    ("MPEG-2 (Program Stream)", "MPEG-2 (programfolyam)"),
    ("MPEG-2 (BDAV Transport Stream)", "MPEG-2 (BDAV transzportfolyam)"),
    ("HLS (HTTP Live Streaming) (fragmented MP4 segments)", "HLS (élő HTTP-közvetítés) (fragmentált MP4-szegmensek)"),
    ("OpenEXR (Image sequence)", "OpenEXR (képsorozat)"),
    ("DCP (Digital Cinema Package) (SMPTE, Scope 2.39:1)", "DCP (digitális moziformátum) (SMPTE, Scope 2,39:1)"),
    ("MXF OP1a (Avid DNxHR)", "MXF OP1a (Avid DNxHR)"),
    ("Animated GIF", "Animált GIF"),
    ("Waveform Audio", "Waveform hang"),
]


@pytest.mark.parametrize(("engine", "shown"), VIDEO + AUDIO)
def test_hungarian_lines(hu: None, engine: str, shown: str):
    assert es.line(engine) == shown


@pytest.mark.parametrize(("engine", "shown"), FORMAT)
def test_hungarian_format_line(hu: None, engine: str, shown: str):
    assert es.format_line(engine) == shown


def test_sizes_and_english():
    TR.set_language("hu")
    try:
        assert es.size("1.2 GB") == "1,2 GB"
        assert es.size("350 MB") == "350 MB"
    finally:
        TR.set_language("en")
    for engine, _ in VIDEO + AUDIO:
        assert es.line(engine) == engine, "English shows the engine's text as it is"
    for engine, _ in FORMAT:
        assert es.format_line(engine) == engine
    assert es.size("1.2 GB") == "1.2 GB"


def test_language_without_the_templates_keeps_english_fragments():
    """German has no Summary templates yet: the fragments stay English, numbers untouched."""
    TR.set_language("de")
    try:
        assert es.line("CBR 4.00 Mbps, keyframe every 50 frames") == "CBR 4.00 Mbps, keyframe every 50 frames"
    finally:
        TR.set_language("en")


def test_hostile_text_never_breaks(hu: None):
    for text in (
        "",
        ", ",
        ",,,",
        " (",
        "()",
        "x (",
        "(MP4)",
        "A (B",
        "9" * 5000,
        "\x00, \t",
        "Target  Mbps",
        "1x (1.0)",
        " compression",
    ):
        assert isinstance(es.line(text), str)
        assert isinstance(es.format_line(text), str)
        assert isinstance(es.size(text), str)
    assert es.line("9" * 5000) == "9" * 5000, "over-long lines are shown as they are"


def test_every_template_and_word_is_translated_into_hungarian():
    hu = Translator("hu")
    missing = [t for t in es.WHOLE if not hu.has(t)] + [t for _, t in es.PATTERNS if not hu.has(t)]
    missing += [w for w in es.CONTEXT_WORDS if hu.tr_ctx(es.CONTEXT, w) == hu.tr(w)]
    assert not missing, missing


# names and pieces of patterns that rightly stay as the engine writes them
PASS_THROUGH = {
    "2K", "4K", "AAC", "Opus", "HQ", "MP4", "QuickTime", "SMPTE", "VP9 Profile 0", "auto level", "float",
    "ProRes 422", "ProRes 422 HQ", "ProRes 422 LT", "ProRes 422 Proxy", "ProRes 4444", "ProRes 4444 XQ",
    "Flat 1.85:1", "Scope 2.39:1", "Full 1.90:1",
}  # fmt: skip


def test_engine_summary_fragments_are_known():
    """A plain fragment added to the engine's Summary needs a translation here (or PASS_THROUGH)."""
    src = (ROOT / "crates" / "export" / "src" / "settings.rs").read_text(encoding="utf-8")
    start = src.index("pub fn summary(&self")
    body = src[start : src.index("\n    }\n}\n", start)]
    hu = Translator("hu")
    unknown = []
    for literal in re.findall(r'"((?:[^"\\]|\\.)*)"', body):
        for piece in (p.strip() for p in literal.split(", ")):
            if "{" in piece or not re.search(r"[A-Za-z]", piece) or piece in PASS_THROUGH:
                continue
            if piece not in es.WHOLE or not hu.has(piece):
                unknown.append(piece)
    assert not unknown, unknown
