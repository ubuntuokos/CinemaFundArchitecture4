"""SC-3: per-channel audio waveforms in the Qt timeline (plan CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001).

The peak cache asks the engine for `media.peaks` with `perChannel`, keeps the memory bounded and
falls back to the merged waveform; the canvas draws one band per channel, named in the engine's order."""

from __future__ import annotations

import os
import struct
from collections.abc import Iterator
from typing import Any

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
pytest.importorskip("PySide6.QtCore")
pytest.importorskip("PySide6.QtWidgets")

from PySide6 import QtCore, QtGui, QtWidgets

from cfa4_qt.panels import timeline_media as tm
from cfa4_qt.panels.timeline import HEADER_W, TimelineCanvas, channel_labels
from cfa4_qt.timecode import TICKS_PER_SECOND


@pytest.fixture(scope="module")
def app() -> Iterator[QtCore.QCoreApplication]:
    yield QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


class FakeEngine:
    running = True

    def __init__(self, channels: int = 2, per_channel_support: bool = True) -> None:
        self.calls: list[dict[str, Any]] = []
        self.channels = channels
        self.support = per_channel_support
        self.fail = False

    def call(self, method: str, params: dict[str, Any], on_reply: Any = None, on_error: Any = None) -> None:
        assert method == "media.peaks"
        self.calls.append(params)
        if self.fail:
            on_error("boom")
            return
        n = params["buckets"]
        split = bool(params.get("perChannel")) and self.support
        chans = self.channels if split else 1
        # channel c holds amplitude (c + 1) / 10 in every bucket
        values: list[float] = []
        for c in range(chans):
            for _ in range(n):
                values += [-(c + 1) / 10, (c + 1) / 10]
        meta = {"channels": chans} | ({"perChannel": True} if split else {})
        on_reply(meta, struct.pack(f"<{len(values)}f", *values))


def ask(media: tm.TimelineMedia, *args: Any, **kw: Any) -> Any:
    """The lookup queues what is missing; the fake engine answers at once, so ask again."""
    for _ in range(4):
        got = media.waveform_channels(*args, **kw)
        if got is not None:
            return got
    return None


def make(engine: FakeEngine) -> tm.TimelineMedia:
    media = tm.TimelineMedia()
    media.attach(engine)
    return media


def test_channel_labels_follow_the_engine_order() -> None:
    assert channel_labels(2) == ["L", "R"]
    assert channel_labels(6) == ["L", "R", "C", "LFE", "Ls", "Rs"]
    assert channel_labels(3) == ["1", "2", "3"]
    assert channel_labels(0) == []


def test_per_channel_peaks_split_by_channel(app: Any) -> None:
    engine = FakeEngine(channels=6)
    media = make(engine)
    media.begin_paint()
    assert media.waveform_channels(7, 0, 2 * TICKS_PER_SECOND, 0) is None  # queued first
    got = ask(media, 7, 0, 2 * TICKS_PER_SECOND, 0)
    assert got is not None and len(got) == 6
    assert engine.calls[0]["perChannel"] is True
    assert all(len(ch) == len(got[0]) for ch in got)
    assert got[0][0] == pytest.approx((-0.1, 0.1)) and got[5][0] == pytest.approx((-0.6, 0.6))
    n = len(engine.calls)
    assert media.waveform_channels(7, 0, 2 * TICKS_PER_SECOND, 0) is got and len(engine.calls) == n  # cached
    # the merged waveform is a separate request, without perChannel
    media.waveform(7, 0, TICKS_PER_SECOND, 0)
    assert "perChannel" not in engine.calls[-1]


def test_engine_without_per_channel_gives_none_so_the_merged_view_stays(app: Any) -> None:
    media = make(FakeEngine(per_channel_support=False))
    media.begin_paint()
    assert media.waveform_channels(7, 0, TICKS_PER_SECOND, 0) is None  # answered, but not split
    assert media.waveform_channels(7, 0, TICKS_PER_SECOND, 0) is None
    assert media.channel_peaks  # remembered: it is not asked again and again


def test_failed_request_and_hostile_arguments_do_not_raise(app: Any) -> None:
    engine = FakeEngine()
    engine.fail = True
    media = make(engine)
    media.begin_paint()
    assert media.waveform_channels(7, 0, TICKS_PER_SECOND, 0) is None
    for bad in [(None, 0, 10), ("x", 0, 10), (True, 0, 10), (7, 10, 5), (7, 5, 5)]:
        assert media.waveform_channels(*bad, 0) is None
    engine2 = FakeEngine()
    media2 = make(engine2)
    media2.begin_paint()
    assert ask(media2, 7, 0, TICKS_PER_SECOND, stream=-5) is not None
    assert engine2.calls[0]["stream"] == 0


def test_long_clip_is_read_in_pieces_and_joined_per_channel(app: Any) -> None:
    engine = FakeEngine(channels=2)
    media = make(engine)
    media.begin_paint()
    length = (tm.MAX_PEAK_SECONDS + 100) * TICKS_PER_SECOND
    got = ask(media, 7, 0, length, 0)
    assert got is not None and len(got) == 2 and len(got[0]) == len(got[1])
    assert len(engine.calls) == 2 and all(c["perChannel"] for c in engine.calls)


def test_cache_memory_is_bounded(app: Any, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(tm, "MAX_CHANNEL_VALUES", 1000)
    engine = FakeEngine(channels=2)
    media = make(engine)
    for i in range(30):
        media.begin_paint()  # a new paint: older entries may go
        ask(media, i, 0, TICKS_PER_SECOND, 0)  # 100 buckets x 2 channels
    total = sum(len(v) * len(v[0]) for v in media.channel_peaks.values() if v)
    assert total <= 1000 + 200 and len(media.channel_peaks) < 30


def test_generate_waveform_forgets_channel_peaks(app: Any) -> None:
    media = make(FakeEngine())
    media.begin_paint()
    ask(media, 7, 0, TICKS_PER_SECOND, 0)
    ask(media, 8, 0, TICKS_PER_SECOND, 0)
    media.forget_peaks({7})
    assert all(k[0] != 7 for k in media.channel_peaks) and any(k[0] == 8 for k in media.channel_peaks)
    media.clear()
    assert not media.channel_peaks


def test_canvas_draws_one_band_per_channel(app: Any) -> None:
    canvas = TimelineCanvas()
    canvas.resize(HEADER_W + 400, 120)
    canvas.dynamic_waveforms = False
    body = QtCore.QRectF(HEADER_W + 10, 10, 300, 100)
    # left loud, right silent: only the upper band is filled
    loud = [(-0.9, 0.9)] * 50
    quiet = [(0.0, 0.0)] * 50
    image = QtGui.QImage(canvas.width(), canvas.height(), QtGui.QImage.Format.Format_RGB32)
    image.fill(QtGui.QColor("black"))
    p = QtGui.QPainter(image)
    canvas._paint_channel_bands(p, [loud, quiet], body, QtGui.QColor(40, 90, 160), False)
    p.end()
    x = int(body.left()) + 150
    black = QtGui.QColor("black").rgb()
    top = sum(image.pixel(x, y) != black for y in range(int(body.top()), int(body.top() + 50)))
    bottom_wave = sum(image.pixel(x, y) != black for y in range(int(body.top() + 53), int(body.bottom())))
    assert top > 30, "the left channel fills its band"
    assert bottom_wave <= 1, "the silent right channel stays a line"
    # nothing is drawn left of the track headers or outside the clip
    assert image.pixel(HEADER_W - 5, 60) == black and image.pixel(int(body.right()) + 5, 60) == black
    # channel zoom: a quiet channel shows larger
    canvas.channel_zoom = 4.0
    image.fill(QtGui.QColor("black"))
    p = QtGui.QPainter(image)
    canvas._paint_channel_bands(p, [[(-0.1, 0.1)] * 50, quiet], body, QtGui.QColor(40, 90, 160), False)
    p.end()
    zoomed = sum(image.pixel(x, y) != black for y in range(int(body.top()), int(body.top() + 50)))
    canvas.channel_zoom = 1.0
    image.fill(QtGui.QColor("black"))
    p = QtGui.QPainter(image)
    canvas._paint_channel_bands(p, [[(-0.1, 0.1)] * 50, quiet], body, QtGui.QColor(40, 90, 160), False)
    p.end()
    plain = sum(image.pixel(x, y) != black for y in range(int(body.top()), int(body.top() + 50)))
    assert zoomed > plain
    # hostile input must not raise: empty bands, odd counts, a tiny body
    p = QtGui.QPainter(image)
    canvas._paint_channel_bands(p, [[], []], body, QtGui.QColor("red"), False)
    canvas._paint_channel_bands(p, [loud] * 10, QtCore.QRectF(HEADER_W, 0, 5, 5), QtGui.QColor("red"), True)
    p.end()


def test_default_display_is_merged(app: Any) -> None:
    canvas = TimelineCanvas()
    assert canvas.waveform_channels == "merged" and canvas.channel_zoom == 1.0
