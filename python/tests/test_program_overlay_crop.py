"""On-monitor Crop rectangle (CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001, SC-1): the edge math of a
drag, and the overlay's handles and the commands a drag sends."""

from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest

pytest.importorskip("PySide6")

from PySide6 import QtCore, QtGui, QtWidgets  # noqa: E402

from cfa4_qt.panels.program_overlay import (  # noqa: E402
    CROP_MERGE,
    MIN_CROP,
    ProgramOverlay,
    crop_edges,
    crop_percents,
)

SIZE = (1000.0, 500.0)
START = (100.0, 100.0, 500.0, 300.0)  # 400 × 200, a 2:1 rectangle
ORIGIN = (0.0, 0.0)


def drag(grab, dx, dy, **kw):
    return crop_edges(grab, START, SIZE, (dx, dy), ORIGIN, **kw)


def test_dragging_an_edge_moves_only_that_edge():
    assert drag(4, 0, -50) == (100.0, 50.0, 500.0, 300.0)  # top
    assert drag(5, 70, 0) == (100.0, 100.0, 570.0, 300.0)  # right
    assert drag(6, 0, 40) == (100.0, 100.0, 500.0, 340.0)  # bottom
    assert drag(7, -30, 0) == (70.0, 100.0, 500.0, 300.0)  # left


def test_dragging_a_corner_moves_two_edges():
    assert drag(0, -10, -20) == (90.0, 80.0, 500.0, 300.0)
    assert drag(2, 10, 20) == (100.0, 100.0, 510.0, 320.0)


def test_the_rectangle_moves_and_stays_inside_the_picture():
    assert drag("move", 50, -20) == (150.0, 80.0, 550.0, 280.0)
    assert drag("move", -5000, -5000) == (0.0, 0.0, 400.0, 200.0)
    assert drag("move", 5000, 5000) == (600.0, 300.0, 1000.0, 500.0)


def test_an_edge_stops_at_the_picture_and_before_the_opposite_edge():
    assert drag(7, -5000, 0)[0] == 0.0
    assert drag(5, 5000, 0)[2] == SIZE[0]
    x0, _y0, x1, _y1 = drag(5, -5000, 0)  # dragged past the left edge: the smallest allowed width
    assert x1 - x0 == pytest.approx(SIZE[0] * MIN_CROP / 100)


def test_alt_resizes_about_the_centre():
    assert drag(5, 40, 0, centered=True) == (60.0, 100.0, 540.0, 300.0)
    assert drag(0, -20, -10, centered=True) == (80.0, 90.0, 520.0, 310.0)
    # the mirrored edge stops at the picture too
    x0, _y0, x1, _y1 = drag(5, 5000, 0, centered=True)
    assert (x0, x1) == (0.0, 600.0)  # 100 px from the left edge: the right edge may only go 100 px out


@pytest.mark.parametrize("grab", [0, 1, 2, 3, 4, 5, 6, 7])
@pytest.mark.parametrize("centered", [False, True])
def test_shift_keeps_the_shape_of_the_rectangle(grab, centered):
    for dx, dy in [(30, 10), (-60, -40), (200, 150), (-300, 20), (5000, 5000), (-5000, -5000)]:
        x0, y0, x1, y1 = drag(grab, dx, dy, lock=True, centered=centered)
        assert (x1 - x0) / (y1 - y0) == pytest.approx(2.0)
        assert 0.0 <= x0 < x1 <= SIZE[0] + 1e-9 and 0.0 <= y0 < y1 <= SIZE[1] + 1e-9


def test_shift_on_a_corner_keeps_the_opposite_corner():
    x0, y0, x1, y1 = drag(2, 100, 10, lock=True)  # bottom right, mostly sideways
    assert (x0, y0) == (100.0, 100.0)
    assert (x1 - x0, y1 - y0) == pytest.approx((500.0, 250.0))


def test_percents_are_the_cropped_parts_of_the_picture():
    assert crop_percents(START, SIZE) == {"left": 10.0, "top": 20.0, "right": 50.0, "bottom": 40.0}
    assert crop_percents((0.0, 0.0, 1000.0, 500.0), SIZE) == {"left": 0.0, "top": 0.0, "right": 0.0, "bottom": 0.0}
    # the edge math and the percents agree: a drag that changes nothing sends nothing different
    assert crop_percents(drag(5, 0, 0), SIZE) == crop_percents(START, SIZE)


@pytest.fixture(scope="module")
def app():
    return QtWidgets.QApplication.instance() or QtWidgets.QApplication([])


def make_overlay(app, sent):
    P = QtCore.QPointF

    class View(QtWidgets.QWidget):
        def __init__(self):
            super().__init__()
            self.resize(1000, 500)

        def picture_rect(self):
            return QtCore.QRectF(0, 0, 1000, 500)

    view = View()
    overlay = ProgramOverlay(view, None, lambda c, p: sent.append((c, p)))
    overlay.data = {
        "frame": [1000, 500],
        "crop": {"clip": 7, "effect": 2, "effectId": "crop", "size": [1000, 500], "toFrame": [1, 0, 0, 1, 0, 0],
                 "rect": list(START), "left": 10.0, "top": 20.0, "right": 50.0, "bottom": 40.0, "zoom": False},
    }
    return view, overlay, P


def press_event(at, mods=QtCore.Qt.KeyboardModifier.NoModifier, type_=QtCore.QEvent.Type.MouseButtonPress):
    return QtGui.QMouseEvent(type_, at, at, QtCore.Qt.MouseButton.LeftButton, QtCore.Qt.MouseButton.LeftButton, mods)


def test_overlay_finds_the_crop_handles(app):
    sent: list = []
    _view, overlay, P = make_overlay(app, sent)
    assert overlay.hit("crop.handle.0") == P(100, 100)
    assert overlay.hit("crop.handle.2") == P(500, 300)
    assert overlay.hit("crop.handle.5") == P(500, 200)
    assert overlay.hit("crop.box") == P(300, 200)
    assert overlay.hit("crop.handle.9") is None
    overlay.data["crop"] = None
    assert overlay.hit("crop.box") is None


def test_a_handle_drag_sets_the_crop_values_as_one_undo_step(app):
    sent: list = []
    _view, overlay, P = make_overlay(app, sent)
    assert overlay.press(press_event(overlay.hit("crop.handle.7")))  # left edge at x = 100
    assert overlay.drag is not None and overlay.drag["kind"] == "crop"
    overlay.drag["grab"] = 7
    overlay._crop_move(overlay.drag, P(150, 200))  # 50 px right: Left 10 % → 15 %
    overlay._crop_move(overlay.drag, P(200, 200))  # 100 px right: Left 20 %
    assert [(c, p["param"], p["value"], p["begin"], p["merge"]) for c, p in sent] == [
        ("effects.setParam", "left", 15.0, True, CROP_MERGE),
        ("effects.setParam", "left", 20.0, False, CROP_MERGE),
    ]
    assert all(p["clip"] == 7 and p["effect"] == 2 for _c, p in sent)  # only Left changed, on the selected Crop


def test_a_drag_inside_moves_the_rectangle_and_a_corner_with_shift_keeps_its_shape(app):
    sent: list = []
    _view, overlay, P = make_overlay(app, sent)
    assert overlay.press(press_event(overlay.hit("crop.box")))
    assert overlay.drag["grab"] == "move"
    overlay._crop_move(overlay.drag, P(350, 150))  # 50 right, 50 up
    got = {p["param"]: p["value"] for _c, p in sent}
    assert got == {"left": 15.0, "top": 10.0, "right": 45.0, "bottom": 50.0}
    overlay.release(press_event(P(350, 150), type_=QtCore.QEvent.Type.MouseButtonRelease))
    sent.clear()
    assert overlay.press(press_event(overlay.hit("crop.handle.2")))
    overlay._crop_move(overlay.drag, P(600, 320), lock=True)
    got = {"left": 10.0, "top": 20.0, "right": 50.0, "bottom": 40.0}  # only the changed values are sent
    got |= {p["param"]: p["value"] for _c, p in sent}
    assert set(p["param"] for _c, p in sent) == {"right", "bottom"}  # the bottom right corner: the top left stays
    width = 1000 * (100 - got["left"] - got["right"]) / 100
    height = 500 * (100 - got["top"] - got["bottom"]) / 100
    assert width / height == pytest.approx(2.0)
