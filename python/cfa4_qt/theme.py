"""The editor's look: egui's design tokens (crates/ui-egui/src/theme.rs, measured from Premiere's
"Darkest" UI) for the Darkest, Dark and Light themes, applied to Qt as the Fusion style, an
application palette and a small style sheet. Panels that paint themselves read ``current()``.

The theme is chosen as egui chooses it, from the engine's Settings ▸ Appearance preferences
(shared with the egui editor): the Appearance Mode (``auto`` follows the system, ``light``,
``dark``) picks the light or the dark theme; the highlight colour and accessible contrast apply
on top (egui's ``with_appearance``).
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Any

from PySide6.QtCore import Qt
from PySide6.QtGui import QColor, QGuiApplication, QPalette
from PySide6.QtWidgets import QApplication

from .i18n import N_

# (pref value, label): egui's Color Theme choices (View ▸ Appearance)
THEMES: tuple[tuple[str, str], ...] = (("darkest", N_("Darkest")), ("dark", N_("Dark")), ("light", N_("Light")))
APPEARANCE_MODES = ("auto", "light", "dark")


@dataclass(frozen=True)
class Tokens:
    kind: str
    app_bg: str
    header_bg: str
    panel_bg: str
    tab_bg: str
    tab_text: str
    tab_text_active: str
    focus: str
    accent: str
    accent_hover: str
    text: str
    text_dim: str
    text_faint: str
    icon: str
    hover: str
    pressed: str
    field_bg: str
    field_border: str
    separator: str
    row_alt: str
    row_selected: str
    hot_text: str
    control_surface: str
    button_hover: str
    button_border: str
    scroll_thumb: str
    scroll_thumb_hover: str
    tl_bg: str
    tl_track_bg: str
    tl_track_bg_alt: str
    tl_header_bg: str
    tl_ruler_bg: str
    tl_ruler_tick: str
    tl_ruler_text: str
    playhead: str
    track_target: str
    in_out_shade: str
    clip_selected_border: str
    clip_bar_bg: str  # a clip's bar in Effect Controls' lane (egui's)
    plot_bg: str  # Fx Editor plots (egui's)
    plot_grid: str
    plot_axis: str
    eq_track: str
    eq_tick: str
    eq_knob: str
    eq_knob_active: str
    eq_node: str
    eq_node_off: str
    crossover_handle: str
    monitor_bg: str
    timecode: str
    meter_bg: str
    meter_readout_bg: str
    danger: str

    def c(self, name: str) -> QColor:
        return QColor(getattr(self, name))

    @property
    def dark(self) -> bool:
        return self.kind != "light"


DARKEST = Tokens(
    kind="darkest",
    app_bg="#000000",
    header_bg="#1d1d1d",
    panel_bg="#1d1d1d",
    tab_bg="#1d1d1d",
    tab_text="#b0b0b0",
    tab_text_active="#d1d1d1",
    focus="#5794ec",
    accent="#2f6bdf",
    accent_hover="#3f7ce8",
    text="#d1d1d1",
    text_dim="#b0b0b0",
    text_faint="#6e6e6e",
    icon="#b0b0b0",
    hover="#2c2c2c",
    pressed="#4b4b4b",
    field_bg="#0e0e0e",
    field_border="#303030",
    separator="#303030",
    row_alt="#212121",
    row_selected="#333333",
    hot_text="#4096f3",
    control_surface="#242424",
    button_hover="#2a2a2a",
    button_border="#4b4b4b",
    scroll_thumb="#4b4b4b",
    scroll_thumb_hover="#6a6a6a",
    tl_bg="#1d1d1d",
    tl_track_bg="#1d1d1d",
    tl_track_bg_alt="#1d1d1d",
    tl_header_bg="#1d1d1d",
    tl_ruler_bg="#1d1d1d",
    tl_ruler_tick="#8d8d8d",
    tl_ruler_text="#b0b0b0",
    playhead="#5895ec",
    track_target="#265bc1",
    in_out_shade="#3f3f3f",
    clip_selected_border="#ebebeb",
    clip_bar_bg="#3a3a46",
    plot_bg="#181818",
    plot_grid="#2c2c2c",
    plot_axis="#444444",
    eq_track="#303030",
    eq_tick="#606060",
    eq_knob="#c0c0c0",
    eq_knob_active="#f0f0f0",
    eq_node="#e8e8e8",
    eq_node_off="#707070",
    crossover_handle="#ffffff",
    monitor_bg="#1d1d1d",
    timecode="#4096f3",
    meter_bg="#000000",
    meter_readout_bg="#141414",
    danger="#dc513d",
)
DARK = replace(
    DARKEST,
    kind="dark",
    app_bg="#141414",
    header_bg="#323232",
    panel_bg="#323232",
    tab_bg="#323232",
    field_bg="#222222",
    tl_bg="#323232",
    tl_track_bg="#323232",
    tl_track_bg_alt="#323232",
    tl_header_bg="#323232",
    tl_ruler_bg="#323232",
    monitor_bg="#323232",
    row_alt="#323232",
    hover="#404040",
)
LIGHT = replace(
    DARKEST,
    kind="light",
    clip_bar_bg="#becbe0",
    plot_bg="#f5f5f5",
    plot_grid="#d2d2d2",
    plot_axis="#aaaaaa",
    eq_track="#c4c4c4",
    eq_tick="#969696",
    eq_knob="#555555",
    eq_knob_active="#222222",
    eq_node="#323232",
    eq_node_off="#8c8c8c",
    crossover_handle="#323232",
    app_bg="#b4b4b4",
    header_bg="#d6d6d6",
    panel_bg="#e8e8e8",
    tab_bg="#e8e8e8",
    tab_text="#5a5a5a",
    tab_text_active="#141414",
    text="#222222",
    text_dim="#5a5a5a",
    text_faint="#8c8c8c",
    icon="#3c3c3c",
    hover="#d2d2d2",
    pressed="#c4c4c4",
    field_bg="#fafafa",
    field_border="#aaaaaa",
    separator="#c8c8c8",
    row_alt="#e0e0e0",
    row_selected="#aac8f0",
    control_surface="#dadada",
    button_hover="#d2d2d2",
    button_border="#aaaaaa",
    scroll_thumb="#969696",
    scroll_thumb_hover="#828282",
    in_out_shade="#b9b9b9",
    tl_bg="#d2d2d2",
    tl_track_bg="#dedede",
    tl_track_bg_alt="#e4e4e4",
    tl_header_bg="#d6d6d6",
    tl_ruler_bg="#e2e2e2",
    tl_ruler_text="#505050",
    meter_bg="#cdcdcd",
    meter_readout_bg="#dadada",
)
BY_KIND = {"darkest": DARKEST, "dark": DARK, "light": LIGHT}

_current: Tokens = DARKEST


def current() -> Tokens:
    return _current


def _hex(c: QColor) -> str:
    return c.name(QColor.NameFormat.HexRgb)


def with_appearance(t: Tokens, highlight: str | None, accessible_contrast: bool) -> Tokens:
    """egui's with_appearance: a highlight colour of the user's own also tints the theme's other
    blues (focus, playhead, timecodes, track targeting); accessible contrast brightens secondary
    text and borders."""
    changes: dict[str, str] = {}
    c = QColor(highlight) if highlight else QColor()
    if c.isValid():
        if _hex(c) != t.accent:

            def tint(s: float, v: float) -> str:
                h, sat, val, _a = c.getHsvF()
                return _hex(QColor.fromHsvF(max(0.0, h), min(1.0, sat * s), min(1.0, val * v)))

            changes.update(focus=tint(0.92, 1.06), playhead=tint(0.92, 1.06), timecode=tint(0.98, 1.09), hot_text=tint(0.98, 1.09), track_target=tint(1.0, 0.865))
        changes.update(accent=_hex(c), accent_hover=_hex(QColor(min(255, c.red() + 16), min(255, c.green() + 16), min(255, c.blue() + 16))))
    if accessible_contrast:
        if t.kind == "light":
            changes.update(text_dim="#282828", text_faint="#505050", field_border="#6e6e6e")
        else:
            changes.update(text_dim=t.text, text_faint="#a8a8a8", tab_text=t.tab_text_active, field_border="#6a6a6a", separator="#505050")
    return replace(t, **changes)


def system_is_light() -> bool | None:
    hints = QGuiApplication.styleHints()
    scheme = hints.colorScheme() if hints is not None else Qt.ColorScheme.Unknown
    if scheme == Qt.ColorScheme.Light:
        return True
    if scheme == Qt.ColorScheme.Dark:
        return False
    return None


def tokens_from_prefs(appearance: dict[str, Any] | None) -> Tokens:
    """The theme the engine's ``appearance`` preferences select (egui's appearance_kind and
    shown_theme): the light or the dark theme of the Appearance Mode, Auto following the system
    (dark when it gives no answer)."""
    a = appearance or {}
    mode = str(a.get("appearanceMode") or "dark")
    light = mode == "light" or (mode == "auto" and system_is_light() is True)
    name = str(a.get("lightTheme") or "light") if light else str(a.get("darkTheme") or "darkest")
    base = BY_KIND.get(name, LIGHT if light else DARKEST)
    hl = a.get("highlightColor")
    return with_appearance(base, hl if isinstance(hl, str) else None, bool(a.get("accessibleContrast")))


def palette(t: Tokens) -> QPalette:
    p = QPalette()
    R, G = QPalette.ColorRole, QPalette.ColorGroup
    for role, name in (
        (R.Window, "panel_bg"),
        (R.WindowText, "text"),
        (R.Base, "panel_bg"),
        (R.AlternateBase, "row_alt"),
        (R.ToolTipBase, "header_bg"),
        (R.ToolTipText, "text"),
        (R.PlaceholderText, "text_faint"),
        (R.Text, "text"),
        (R.Button, "control_surface"),
        (R.ButtonText, "text"),
        (R.BrightText, "danger"),
        (R.Light, "hover"),
        (R.Midlight, "pressed"),
        (R.Mid, "separator"),
        (R.Dark, "app_bg"),
        (R.Shadow, "app_bg"),
        (R.Highlight, "accent"),
        (R.Link, "hot_text"),
        (R.LinkVisited, "hot_text"),
    ):
        p.setColor(role, t.c(name))
    p.setColor(R.HighlightedText, QColor("#ffffff"))
    for role in (R.WindowText, R.Text, R.ButtonText):
        p.setColor(G.Disabled, role, t.c("text_faint"))
    p.setColor(G.Disabled, R.Highlight, t.c("pressed"))
    return p


def style_sheet(t: Tokens) -> str:
    return f"""
QMainWindow {{ background: {t.app_bg}; }}
QMainWindow::separator {{ background: {t.app_bg}; width: 4px; height: 4px; }}
QDockWidget {{ color: {t.tab_text}; titlebar-close-icon: none; }}
QDockWidget::title {{ background: {t.tab_bg}; padding: 6px 8px; text-align: left; }}
QTabBar {{ background: {t.app_bg}; }}
QTabBar::tab {{ background: {t.tab_bg}; color: {t.tab_text}; padding: 6px 12px; border: none; margin-right: 1px; }}
QTabBar::tab:selected {{ color: {t.tab_text_active}; border-bottom: 2px solid {t.focus}; }}
QTabBar::tab:hover {{ color: {t.tab_text_active}; }}
QMenuBar {{ background: {t.header_bg}; color: {t.text}; }}
QMenuBar::item:selected {{ background: {t.hover}; }}
QMenu {{ background: {t.header_bg}; color: {t.text}; border: 1px solid {t.separator}; }}
QMenu::item:selected {{ background: {t.accent}; color: #ffffff; }}
QMenu::item:disabled {{ color: {t.text_faint}; }}
QMenu::separator {{ height: 1px; background: {t.separator}; margin: 4px 6px; }}
QToolTip {{ background: {t.header_bg}; color: {t.text}; border: 1px solid {t.separator}; }}
QStatusBar {{ background: {t.header_bg}; color: {t.text_dim}; }}
QLineEdit, QSpinBox, QDoubleSpinBox, QComboBox, QPlainTextEdit, QTextEdit {{
    background: {t.field_bg}; color: {t.text}; border: 1px solid {t.field_border}; border-radius: 3px;
    selection-background-color: {t.accent}; selection-color: #ffffff; }}
QLineEdit, QSpinBox, QDoubleSpinBox, QComboBox {{ padding: 2px 4px; }}
QPushButton {{ background: {t.control_surface}; color: {t.text}; border: 1px solid {t.button_border};
    border-radius: 4px; padding: 4px 12px; }}
QPushButton:hover {{ background: {t.button_hover}; }}
QPushButton:pressed {{ background: {t.pressed}; }}
QPushButton:disabled {{ color: {t.text_faint}; }}
QPushButton:default {{ background: {t.accent}; color: #ffffff; border-color: {t.accent}; }}
QToolButton {{ color: {t.text}; border: 1px solid transparent; border-radius: 4px; padding: 2px; }}
QToolButton:hover {{ background: {t.hover}; }}
QToolButton:pressed, QToolButton:checked {{ background: {t.pressed}; }}
QHeaderView::section {{ background: {t.panel_bg}; color: {t.text_dim}; border: none;
    border-right: 1px solid {t.separator}; border-bottom: 1px solid {t.separator}; padding: 3px 6px; }}
QTreeView, QListView, QTableView {{ background: {t.panel_bg}; alternate-background-color: {t.row_alt};
    color: {t.text}; border: none; selection-background-color: {t.row_selected}; selection-color: {t.text}; }}
QScrollBar:vertical {{ background: {t.panel_bg}; width: 10px; margin: 0; }}
QScrollBar:horizontal {{ background: {t.panel_bg}; height: 10px; margin: 0; }}
QScrollBar::handle {{ background: {t.scroll_thumb}; border-radius: 4px; min-height: 24px; min-width: 24px; margin: 2px; }}
QScrollBar::handle:hover {{ background: {t.scroll_thumb_hover}; }}
QScrollBar::add-line, QScrollBar::sub-line {{ width: 0; height: 0; }}
QScrollBar::add-page, QScrollBar::sub-page {{ background: none; }}
QGroupBox {{ color: {t.text_dim}; border: 1px solid {t.separator}; border-radius: 4px; margin-top: 10px; padding-top: 6px; }}
QGroupBox::title {{ subcontrol-origin: margin; left: 8px; padding: 0 4px; }}
QCheckBox, QRadioButton, QLabel {{ color: {t.text}; }}
"""


def apply(t: Tokens, app: QApplication | None = None) -> None:
    """Make ``t`` the look of the whole application."""
    global _current
    _current = t
    app = app or QApplication.instance()  # type: ignore[assignment]
    if not isinstance(app, QApplication):
        return
    if app.style().name().lower() != "fusion":
        app.setStyle("Fusion")
    app.setPalette(palette(t))
    sheet = style_sheet(t)
    if app.styleSheet() != sheet:  # re-polishing every widget is the slow part
        app.setStyleSheet(sheet)
