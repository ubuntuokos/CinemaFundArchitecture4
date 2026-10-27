"""egui's calc.rs tests, for the Qt GUI's typed number fields."""

from __future__ import annotations

import math

import pytest
from cfa4_qt.calc import evaluate


def test_calculations():
    assert evaluate("100*2", 0) == 200
    assert evaluate("1920/3", 0) == 640
    assert evaluate("(50+10)*1.5", 0) == 90
    assert evaluate("2+3*4", 0) == 14, "* before +"
    assert evaluate("10-2-3", 0) == 5, "left to right"
    assert evaluate("-(4+1)", 0) == -5
    assert evaluate(" .5 + 1. ", 0) == 1.5
    assert evaluate("6×7", 0) == 42
    assert evaluate("9÷3", 0) == 3
    assert evaluate("42", 0) == 42


def test_units_are_ignored():
    assert evaluate("50%*2", 0) == 100
    assert evaluate("90°/2", 0) == 45
    assert evaluate("1920 px / 2", 0) == 960
    assert evaluate("-6 dB", 0) == -6


def test_a_leading_times_or_divide_applies_to_the_current_value():
    assert evaluate("*2", 150) == 300
    assert evaluate("/3", 300) == 100
    assert evaluate("/2+1", 10) == 6
    assert evaluate("-10", 50) == -10, "a leading minus is a sign"
    assert evaluate("+10", 50) == 10, "a leading plus is a sign"
    assert evaluate("200/3", 200) == pytest.approx(200 / 3)


@pytest.mark.parametrize("bad", ["", "   ", "abc", "2*", "*", "(1+2", "1+2)", "1..2", "1/0", "0/0", "2**3", "1,5", "()", "1e3", "10x20"])
def test_bad_input_gives_nothing(bad):
    assert evaluate(bad, 7) is None


def test_limits():
    assert evaluate("*2", math.nan) is None
    assert evaluate("9" * 400, 0) is None, "too long"
    assert evaluate("(" * 100 + "1" + ")" * 100, 0) is None, "too deep"
    assert evaluate("-" * 100 + "1", 0) is None, "too many signs"


def test_numbers_separated_by_spaces_or_a_unit_are_not_merged():
    for bad in ["10 x 20", "1 e 3", "1920 1080", "10px 20", "50% 2", "1 .5"]:
        assert evaluate(bad, 0) is None, bad
    assert evaluate("100 * 2", 0) == 200
    assert evaluate("10px", 0) == 10
    assert evaluate("10 px * 2", 0) == 20
    assert evaluate("*2", 5) == 10


def test_hot_number_takes_a_calculation():
    QtWidgets = pytest.importorskip("PySide6.QtWidgets")
    QtTest = pytest.importorskip("PySide6.QtTest")
    from cfa4_qt.hot_number import HotSpin
    from PySide6.QtCore import Qt

    app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])
    spin = HotSpin(None, 0, 1000, 1, 1.0, " %")
    spin.set_value(150)
    got: list[tuple[float, bool]] = []
    spin.edited.connect(lambda v, b: got.append((v, b)))
    spin.show()
    line = spin.lineEdit()
    QtTest.QTest.mouseClick(line, Qt.MouseButton.LeftButton)
    app.processEvents()
    line.selectAll()
    QtTest.QTest.keyClicks(line, "*2")
    QtTest.QTest.keyClick(line, Qt.Key.Key_Return)
    assert got == [(300.0, True)], got
    line.selectAll()
    QtTest.QTest.keyClicks(line, "(50+10)*1.5 %")
    QtTest.QTest.keyClick(line, Qt.Key.Key_Return)
    assert got[-1] == (90.0, True)
    spin.close()
