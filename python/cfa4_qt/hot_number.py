"""egui's hot number (``crates/ui-egui/src/widgets.rs`` ``hot_number``): a blue value that is
dragged left / right to scrub it (Shift: ten times as fast) or clicked to type a new one."""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QEvent, QObject, QSize, Qt, Signal
from PySide6.QtGui import QMouseEvent
from PySide6.QtWidgets import QDoubleSpinBox, QWidget

from . import theme

SCRUB_PX = 3  # movement before a press becomes a scrub


class HotSpin(QDoubleSpinBox):
    """``edited(value, begin)``: a typed value or a scrub step; ``begin`` is true for a typed
    value and for the first step of a scrub, so a whole scrub is one undo step."""

    edited = Signal(float, bool)

    def __init__(
        self,
        parent: QWidget | None = None,
        lo: float = -1e6,
        hi: float = 1e6,
        decimals: int = 1,
        speed: float = 1.0,
        suffix: str = "",
    ) -> None:
        super().__init__(parent)
        self.speed = speed
        self.setRange(lo, hi)
        self.setDecimals(decimals)
        self.setSingleStep(speed)
        self.setSuffix(suffix)
        self.setKeyboardTracking(False)
        self.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter)
        self._press: tuple[float, float] | None = None  # x, value at the press
        self._scrubbing = False
        self._style = ""
        self.restyle()
        self._line = self.lineEdit()  # kept: a fresh wrapper per lineEdit() call fails `is`
        self._line.installEventFilter(self)
        self._line.setCursor(Qt.CursorShape.SizeHorCursor)
        # focus only while typing: a press must reach the scrub unfocused (Qt focuses a
        # ClickFocus widget before its event filters see the press), and Tab or a deleted
        # neighbour's focus must not land here, where the value would stop following the engine
        self.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self._line.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        self.valueChanged.connect(lambda v: self.edited.emit(float(v), True))

    def restyle(self) -> None:
        t = theme.current()
        # zero-width arrows: NoButtons under the app style sheet leaves the line edit 1 px wide
        style = (
            f"QDoubleSpinBox {{ background: transparent; border: none; padding: 0 2px; color: {t.hot_text}; }}"
            f" QDoubleSpinBox:disabled {{ color: {t.text_faint}; }}"
            f" QDoubleSpinBox:focus {{ background: {t.field_bg}; border: 1px solid {t.focus}; color: {t.text}; }}"
            " QDoubleSpinBox::up-button, QDoubleSpinBox::down-button { width: 0px; border: none; }"
        )
        if style != self._style:
            self._style = style
            self.setStyleSheet(style)

    def sizeHint(self) -> QSize:
        # as wide as the value shown, not as the widest value in range (egui's)
        h = super().sizeHint()
        return QSize(max(36, self.fontMetrics().horizontalAdvance(self.text()) + 12), h.height())

    def minimumSizeHint(self) -> QSize:
        return self.sizeHint()

    @property
    def busy(self) -> bool:
        """Being typed into or scrubbed: a new value from the engine must not replace it."""
        return self._scrubbing or self.lineEdit().hasFocus()

    def set_value(self, value: Any) -> None:
        if self.busy:
            return
        try:
            v = float(value)
        except (TypeError, ValueError):
            return
        self.blockSignals(True)
        self.setValue(v)
        self.blockSignals(False)
        self.updateGeometry()

    def eventFilter(self, watched: QObject, e: QEvent) -> bool:
        if watched is self._line and e.type() == QEvent.Type.FocusOut:
            self.setFocusPolicy(Qt.FocusPolicy.NoFocus)
            self._line.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        if watched is not self._line or not self.isEnabled() or self._line.hasFocus():
            return super().eventFilter(watched, e)
        if e.type() == QEvent.Type.MouseButtonPress and isinstance(e, QMouseEvent) and e.button() == Qt.MouseButton.LeftButton:
            self._press = (e.position().x(), self.value())
            self._scrubbing = False
            return True
        if e.type() == QEvent.Type.MouseMove and isinstance(e, QMouseEvent) and self._press is not None:
            x0, v0 = self._press
            dx = e.position().x() - x0
            if not self._scrubbing and abs(dx) < SCRUB_PX:
                return True
            begin = not self._scrubbing
            self._scrubbing = True
            fast = 10.0 if e.modifiers() & Qt.KeyboardModifier.ShiftModifier else 1.0
            v = max(self.minimum(), min(self.maximum(), round(v0 + dx * self.speed * fast, self.decimals())))
            if v != self.value() or begin:
                self.blockSignals(True)
                self.setValue(v)
                self.blockSignals(False)
                self.edited.emit(v, begin)
            return True
        if e.type() == QEvent.Type.MouseButtonRelease and isinstance(e, QMouseEvent) and self._press is not None:
            self._press = None
            if self._scrubbing:
                self._scrubbing = False
            else:  # a click: type a value
                self.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
                self._line.setFocusPolicy(Qt.FocusPolicy.ClickFocus)
                self._line.setFocus(Qt.FocusReason.MouseFocusReason)
                self.lineEdit().selectAll()
            return True
        return super().eventFilter(watched, e)
