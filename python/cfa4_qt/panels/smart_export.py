"""Export window ▸ Smart Export: the export mode and the render plan preview (WP-08 of
CFA4-SMART-CUT-RENDER-R1, §10.2 and §10.3).

Modes: Full Render (the regular export through the queue), Automatic (proposes stream copy or
Smart Render and asks before re-encoding), Stream Copy – Bit-exact, Stream Copy – Compatible and
Smart Render. Every mode but Full Render asks the engine for the sequence's plan
(``export.plan`` with ``"scope": "timeline"``) and shows it before anything is written: the copied,
re-encoded and rendered ranges as a bar, their frame counts, why ranges are rendered, and why the
plan cannot run. ``export.start`` writes and verifies the file. The window computes nothing itself.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from PySide6.QtCore import QRectF, Qt, QTimer
from PySide6.QtGui import QColor, QPainter, QPaintEvent
from PySide6.QtWidgets import QComboBox, QFormLayout, QGroupBox, QLabel, QMessageBox, QSizePolicy, QWidget

from ..engine import EngineClient
from ..i18n import N_, fmt, tr

MODES = (
    ("full", N_("Full Render")),
    ("auto", N_("Automatic")),
    ("bit-exact", N_("Stream Copy – Bit-exact")),
    ("compatible", N_("Stream Copy – Compatible")),
    ("smart", N_("Smart Render")),
)
# segment mode → (colour, legend)
KINDS = {
    "copy": (QColor("#3fa36b"), N_("copied")),
    "boundary": (QColor("#d9a33a"), N_("re-encoded at a cut")),
    "render": (QColor("#d05a4a"), N_("rendered")),
}
CAUSES = {
    "effect": N_("effects"),
    "transition": N_("transitions"),
    "composite": N_("compositing"),
    "overlay": N_("titles or overlays"),
    "transform": N_("transforms"),
    "speed": N_("speed changes"),
    "frame-rate": N_("frame rate"),
    "color-conversion": N_("colour conversion"),
    "indirect": N_("proxies or nested sequences"),
    "source": N_("sources that cannot be copied"),
}


def segment_kind(mode: str) -> str:
    if mode in ("strict-copy", "compatible-copy"):
        return "copy"
    if mode == "boundary-reencode":
        return "boundary"
    return "render"


def plan_segments(result: dict[str, Any]) -> list[tuple[str, int, list[str]]]:
    """(kind, frames, render causes) of each plan segment, in output order."""
    plan = (result.get("plan") or {}).get("plan") or {}
    out = []
    for s in plan.get("segments") or []:
        if not isinstance(s, dict):
            continue
        span = s.get("presentation") or {}
        first, last = span.get("first"), span.get("last")
        frames = int(last) - int(first) + 1 if isinstance(first, int) and isinstance(last, int) and last >= first else 0
        out.append((segment_kind(str(s.get("mode") or "")), frames, [str(c) for c in s.get("render_causes") or s.get("renderCauses") or []]))
    return out


def smart_output(path: str) -> str:
    """The export file of a Smart Export: MP4 unless the name asks for MOV."""
    return path if path.lower().endswith((".mp4", ".m4v", ".mov")) else f"{path.rsplit('.', 1)[0] if '.' in path.rsplit('/', 1)[-1] else path}.mp4"


def describe(result: dict[str, Any], mode: str) -> tuple[list[str], str | None, bool]:
    """Lines of the plan preview, the reason it cannot run (None when it can), and whether the
    export re-encodes (and so asks first)."""
    summary = result.get("summary") or {}
    copied, reencoded, rendered = (int(summary.get(k) or 0) for k in ("copiedFrames", "reencodedFrames", "renderedFrames"))
    lines = [fmt("{copied} frames copied, {reencoded} re-encoded at cuts, {rendered} rendered", copied=copied, reencoded=reencoded, rendered=rendered)]
    causes: dict[str, int] = {}
    for kind, frames, why in plan_segments(result):
        if kind == "render":
            for c in why:
                causes[c] = causes.get(c, 0) + frames
    if causes:
        named = ", ".join(f"{tr(CAUSES.get(c, c))} ({n})" for c, n in sorted(causes.items(), key=lambda kv: -kv[1]))
        lines.append(fmt("Rendered because of: {causes}", causes=named))
    lossless = bool(summary.get("lossless"))
    if mode == "auto":
        lines.append(tr("Recommended: Stream Copy (nothing is re-encoded)") if lossless else tr("Recommended: Smart Render (asks before re-encoding)"))
    plan = result.get("plan") or {}
    problems = [str(b) for b in plan.get("blockers") or []]
    render = result.get("timelineRender") or {}
    if render.get("problem"):
        problems.append(str(render["problem"]))
    if mode in ("bit-exact", "compatible") and not lossless:
        problems.insert(0, fmt("Stream copy is not possible: {frames} frames need re-encoding.", frames=reencoded + rendered))
    if not plan.get("executable", True) and not problems:
        problems.append(tr("The export cannot run."))
    return lines, ("; ".join(problems) if problems else None), bool(summary.get("requiresConfirmation"))


class PlanBar(QWidget):
    """The plan as a bar: each segment as wide as its frames, coloured by what happens to it."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("export.smart.bar")
        self.setMinimumHeight(14)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed)
        self.segments: list[tuple[str, int, list[str]]] = []

    def set_segments(self, segments: list[tuple[str, int, list[str]]]) -> None:
        self.segments = segments
        self.setToolTip(" · ".join(f"{tr(KINDS[k][1])}: {n}" for k, n, _c in segments if k in KINDS))
        self.update()

    def paintEvent(self, _e: QPaintEvent) -> None:
        total = sum(n for _k, n, _c in self.segments)
        if total <= 0:
            return
        p = QPainter(self)
        x = 0.0
        w, h = float(self.width()), float(self.height())
        for kind, frames, _c in self.segments:
            width = w * frames / total
            p.fillRect(QRectF(x, 0, max(width, 1.0), h), KINDS.get(kind, KINDS["render"])[0])
            x += width
        p.end()


class SmartExportBox(QGroupBox):
    """The Smart Export mode and the plan of the open sequence (``export.smart.*``)."""

    def __init__(
        self,
        engine: EngineClient,
        sequence: Callable[[], int | None],
        output: Callable[[], str],
        said: Callable[[str, bool], None],
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(tr("Smart Export"), parent)
        self.setObjectName("export.smart")
        self.engine = engine
        self.sequence = sequence
        self.output = output
        self.said = said  # (message, is_error) for the window's summary line
        self.result: dict[str, Any] | None = None
        self.problem: str | None = None
        self.confirm = False
        self._request = 0
        form = QFormLayout(self)
        self.mode = QComboBox(self)
        self.mode.setObjectName("export.smart.mode")
        for key, label in MODES:
            self.mode.addItem(tr(label), key)
        form.addRow(tr("Export mode"), self.mode)
        self.bar = PlanBar(self)
        form.addRow(self.bar)
        legend = " ".join(f'<span style="color:{c.name()}">■</span> {tr(label)}' for c, label in KINDS.values())
        self.legend = QLabel(legend, self)
        self.legend.setObjectName("export.smart.legend")
        form.addRow(self.legend)
        self.text = QLabel("", self)
        self.text.setObjectName("export.smart.plan")
        self.text.setWordWrap(True)
        self.text.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        form.addRow(self.text)
        self.error = QLabel("", self)
        self.error.setObjectName("export.smart.problem")
        self.error.setWordWrap(True)
        self.error.setStyleSheet("color: #e08a6a;")
        form.addRow(self.error)
        self._timer = QTimer(self, singleShot=True, interval=200)
        self._timer.timeout.connect(self._plan)
        self.mode.currentIndexChanged.connect(lambda _i: self.refresh())
        self._show_plan_rows(False)

    def current(self) -> str:
        return str(self.mode.currentData() or "full")

    def smart(self) -> bool:
        """A mode that this box runs (all but Full Render, which goes through the queue)."""
        return self.current() != "full"

    def guarantee(self) -> str:
        return "compatible" if self.current() == "compatible" else "bit-exact"

    def _show_plan_rows(self, on: bool) -> None:
        for w in (self.bar, self.legend, self.text, self.error):
            w.setVisible(on)

    def refresh(self) -> None:
        """Plan again (debounced) after the mode, the sequence or the output changed."""
        self.result, self.problem = None, None
        self._show_plan_rows(self.smart())
        if self.smart():
            self.text.setText(tr("Planning…"))
            self.error.setText("")
            self._timer.start()

    def params(self) -> dict[str, Any]:
        p: dict[str, Any] = {"scope": "timeline", "guarantee": self.guarantee()}
        seq = self.sequence()
        if seq is not None:
            p["sequence"] = seq
        out = self.output().strip()
        if out:
            p["output"] = smart_output(out)
        return p

    def _plan(self) -> None:
        if not self.smart() or not self.engine.running:
            return
        self._request += 1
        request, mode = self._request, self.current()

        def got(result: Any) -> None:
            if request != self._request or not isinstance(result, dict):
                return
            self.result = result
            lines, self.problem, self.confirm = describe(result, mode)
            self.bar.set_segments(plan_segments(result))
            self.text.setText("\n".join(lines))
            self.error.setText(self.problem or "")

        def failed(message: str) -> None:
            if request == self._request:
                self.result, self.problem = None, message
                self.bar.set_segments([])
                self.text.setText("")
                self.error.setText(message)

        self.engine.execute("export.plan", self.params(), on_reply=lambda r, _b: got(r), on_error=failed)

    def start(self) -> bool:
        """Run the planned Smart Export. False (and a message) when it cannot run yet."""
        if self.result is None:
            self.said(self.problem or tr("Wait for the export plan."), True)
            return False
        if self.problem:
            self.said(self.problem, True)
            return False
        params = self.params()
        if not params.get("output"):
            self.said(tr("Choose a file name and location."), True)
            return False
        if self.confirm:
            summary = self.result.get("summary") or {}
            frames = int(summary.get("reencodedFrames") or 0) + int(summary.get("renderedFrames") or 0)
            asked = QMessageBox.question(self, tr("Smart Export"), fmt("{frames} frames are re-encoded or rendered. Continue?", frames=frames))
            if asked != QMessageBox.StandardButton.Yes:
                return False
            params["confirm"] = True
        self.said(tr("Writing and verifying the export…"), False)

        def done(result: Any) -> None:
            r = result if isinstance(result, dict) else {}
            self.said(fmt("Smart Export written and verified: {path}", path=str(r.get("output") or params["output"])), False)

        self.engine.execute("export.start", params, on_reply=lambda r, _b: done(r), on_error=lambda m: self.said(m, True))
        return True
