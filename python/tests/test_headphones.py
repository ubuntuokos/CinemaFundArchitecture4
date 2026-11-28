"""Headphone monitoring settings and the monitor's line (binaural plan B-04; no Qt)."""

from __future__ import annotations

from cfa4_qt.headphones import ROOMS, heard_text, monitor_param


def test_the_speakers_send_no_monitor_parameter() -> None:
    assert monitor_param({}) is None
    assert monitor_param({"monitoring": "speakers", "hrtfSet": "/x.sofa"}) is None


def test_headphones_send_the_set_room_and_lfe_level() -> None:
    defaults = monitor_param({"monitoring": "headphones"})
    assert defaults == {"mode": "binaural", "hrtf": "builtin", "room": ROOMS["little"], "lfe_db": -10.0}
    mine = monitor_param({"monitoring": "headphones", "hrtfSet": " /sets/mine.sofa ", "headphoneRoom": "off", "headphoneLfeDb": -20})
    assert mine == {"mode": "binaural", "hrtf": "/sets/mine.sofa", "room": 0.0, "lfe_db": -20.0}
    assert ROOMS["off"] == 0.0 < ROOMS["little"] <= 0.5 < ROOMS["medium"] <= 1.0  # dry, small, large


def test_damaged_preferences_fall_back_to_the_defaults() -> None:
    bad = monitor_param({"monitoring": "headphones", "headphoneRoom": "huge", "headphoneLfeDb": float("nan")})
    assert bad is not None and bad["room"] == ROOMS["little"] and bad["lfe_db"] == -10.0
    loud = monitor_param({"monitoring": "headphones", "headphoneLfeDb": 12, "hrtfSet": None})
    assert loud is not None and loud["lfe_db"] == 0.0 and loud["hrtf"] == "builtin"
    assert monitor_param({"monitoring": "headphones", "headphoneLfeDb": True}) == monitor_param({"monitoring": "headphones"})


def test_the_monitor_says_what_the_headphones_play() -> None:
    assert heard_text({}) == ""
    assert heard_text({"mode": "speakers", "error": "gone"}) == ""
    assert heard_text({"mode": "speakers", "pending": "binaural"}) == "Loading HRTF…"
    assert heard_text({"mode": "binaural", "objects": 16, "format": "Dolby TrueHD Atmos"}) == "Atmos · 16 objects · binaural"
    assert heard_text({"mode": "binaural", "objects": 3, "format": "IAB"}) == "IAB · 3 objects · binaural"
    assert heard_text({"mode": "binaural", "layout": 12}) == "7.1.4 · binaural"
    assert heard_text({"mode": "binaural", "layout": 2}) == "Stereo · binaural"
    assert heard_text({"mode": "binaural", "layout": 7}) == "Binaural"
