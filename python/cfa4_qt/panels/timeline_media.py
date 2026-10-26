"""Clip thumbnails and audio waveforms for the timeline, fetched from the engine.

Thumbnails are the Source picture of a clip's media at its source In (``frame.render`` with
``target: "source"``); waveforms are ``media.peaks`` of the clip's media range. Both are cached
by what they show, requested a few at a time (``MAX_IN_FLIGHT``) so the editor's own requests
never wait behind a long queue, and a finished request repaints the timeline. Nothing is
computed here: the pictures and the peaks are the engine's.
"""

from __future__ import annotations

import struct
from collections import OrderedDict, deque
from typing import Any

from PySide6.QtCore import QObject, Signal
from PySide6.QtGui import QImage

from ..timecode import TICKS_PER_SECOND

MAX_IN_FLIGHT = 3
MAX_THUMBS = 400
MAX_PEAKS = 200
PEAKS_PER_SECOND = 100
MAX_BUCKETS = 20_000
MAX_PEAK_SECONDS = 600  # the engine's limit per request


class TimelineMedia(QObject):
    updated = Signal()

    def __init__(self, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self.engine: Any = None
        self.thumbs: OrderedDict[tuple, QImage | None] = OrderedDict()
        self.peaks: OrderedDict[tuple, list[tuple[float, float]] | None] = OrderedDict()
        self._queue: deque[tuple[str, tuple]] = deque()
        self._queued: set[tuple[str, tuple]] = set()
        self._in_flight = 0

    def attach(self, engine: Any) -> None:
        self.engine = engine

    def clear(self) -> None:
        """Forget everything (another project was opened)."""
        self.thumbs.clear()
        self.peaks.clear()
        self._queue.clear()
        self._queued.clear()

    # ----- lookups (return what is cached, queue what is not) -----
    def thumbnail(self, item: Any, media_time: int, height: int) -> QImage | None:
        if not isinstance(item, int) or height < 8:
            return None
        key = (item, int(media_time), max(16, (int(height) + 15) // 16 * 16))  # few sizes
        if key in self.thumbs:
            self.thumbs.move_to_end(key)
            return self.thumbs[key]
        self._enqueue("thumb", key)
        return None

    def waveform(self, item: Any, start: int, end: int) -> list[tuple[float, float]] | None:
        """The peaks of ``start``..``end``, spread evenly over that range. A range longer than the
        engine reads per request comes in pieces of the same density, joined once all are here."""
        if not isinstance(item, int) or end <= start:
            return None
        start, end = int(start), int(end)
        span = MAX_PEAK_SECONDS * TICKS_PER_SECOND
        if end - start <= span:
            return self._piece((item, start, end, PEAKS_PER_SECOND, 32))
        out: list[tuple[float, float]] = []
        whole = True
        for t0 in range(start, end, span):
            piece = self._piece((item, t0, min(end, t0 + span), MAX_BUCKETS / MAX_PEAK_SECONDS, 1))
            if piece is None:
                whole = False  # keep asking for the other pieces
            elif whole:
                out.extend(piece)
        return out if whole else None

    def _piece(self, key: tuple) -> list[tuple[float, float]] | None:
        if key in self.peaks:
            self.peaks.move_to_end(key)
            return self.peaks[key]
        self._enqueue("peaks", key)
        return None

    # ----- requests -----
    def _enqueue(self, kind: str, key: tuple) -> None:
        job = (kind, key)
        if job not in self._queued:
            self._queued.add(job)
            self._queue.append(job)
        self._pump()

    def _pump(self) -> None:
        engine = self.engine
        if engine is None or not getattr(engine, "running", False):
            return
        while self._in_flight < MAX_IN_FLIGHT and self._queue:
            kind, key = self._queue.popleft()
            self._in_flight += 1
            if kind == "thumb":
                item, t, h = key
                engine.call(
                    "frame.render",
                    {"target": "source", "item": item, "time": t, "maxHeight": h, "maxWidth": h * 3},
                    on_reply=lambda r, b, k=key: self._thumb_done(k, r, b),
                    on_error=lambda _m, k=key: self._thumb_done(k, None, None),
                )
            else:
                item, t0, t1, per_second, least = key
                seconds = (t1 - t0) / TICKS_PER_SECOND
                buckets = max(least, min(MAX_BUCKETS, round(seconds * per_second)))
                engine.call(
                    "media.peaks",
                    {"item": item, "start": t0, "end": t1, "buckets": buckets},
                    on_reply=lambda r, b, k=key: self._peaks_done(k, r, b),
                    on_error=lambda _m, k=key: self._peaks_done(k, None, None),
                )

    def _finish(self, kind: str, key: tuple) -> None:
        self._in_flight = max(0, self._in_flight - 1)
        self._queued.discard((kind, key))
        self.updated.emit()
        self._pump()

    def _thumb_done(self, key: tuple, result: Any, data: bytes | None) -> None:
        image = None
        if isinstance(result, dict) and data:
            w, h = int(result.get("w") or 0), int(result.get("h") or 0)
            if w > 0 and h > 0 and len(data) == w * h * 4:
                image = QImage(data, w, h, w * 4, QImage.Format.Format_RGBA8888).copy()
        self.thumbs[key] = image  # None: no picture (audio-only, offline): don't ask again
        while len(self.thumbs) > MAX_THUMBS:
            self.thumbs.popitem(last=False)
        self._finish("thumb", key)

    def _peaks_done(self, key: tuple, result: Any, data: bytes | None) -> None:
        peaks = None
        if isinstance(result, dict) and data and len(data) % 8 == 0:
            values = struct.unpack(f"<{len(data) // 4}f", data)
            peaks = list(zip(values[0::2], values[1::2], strict=True))
        self.peaks[key] = peaks
        while len(self.peaks) > MAX_PEAKS:
            self.peaks.popitem(last=False)
        self._finish("peaks", key)
