"""Help ▸ About CFA4 as egui's (``crates/ui-egui/src/panels/dialogs.rs`` ``about``): three tabs,
About (version, the CFA4 project links, licence), Contributors (a name toggle, a sort, the list
as a grab bag or a table) and Models (the AI models credited in Co-Authored-By trailers), with
egui's automation ids (``about.tab.<about|contributors|models>``, ``about.github``,
``about.reportIssue``, ``about.credits.*``)."""

from __future__ import annotations

from collections.abc import Callable

from PySide6.QtCore import QPoint, QRect, QSize, Qt, QUrl
from PySide6.QtGui import QDesktopServices
from PySide6.QtWidgets import (
    QButtonGroup,
    QComboBox,
    QDialog,
    QFrame,
    QGridLayout,
    QHBoxLayout,
    QLabel,
    QLayout,
    QLayoutItem,
    QPushButton,
    QScrollArea,
    QStackedWidget,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from .. import credits as cr
from .. import theme
from ..branding import logo_pixmap
from ..i18n import fmt, tr
from .home import GITHUB, ISSUES

LIST_HEIGHT = 360  # egui's LIST_HEIGHT: the window stays on screen


class FlowLayout(QLayout):
    """Widgets left to right, wrapping (egui's ``horizontal_wrapped``)."""

    def __init__(self, parent: QWidget | None = None, spacing: int = 4) -> None:
        super().__init__(parent)
        self.items: list[QLayoutItem] = []
        self.setSpacing(spacing)

    def addItem(self, item: QLayoutItem) -> None:
        self.items.append(item)

    def count(self) -> int:
        return len(self.items)

    def itemAt(self, i: int) -> QLayoutItem | None:
        return self.items[i] if 0 <= i < len(self.items) else None

    def takeAt(self, i: int) -> QLayoutItem | None:
        return self.items.pop(i) if 0 <= i < len(self.items) else None

    def expandingDirections(self) -> Qt.Orientation:
        return Qt.Orientation(0)

    def hasHeightForWidth(self) -> bool:
        return True

    def heightForWidth(self, width: int) -> int:
        return self._place(QRect(0, 0, width, 0), move=False)

    def setGeometry(self, rect: QRect) -> None:
        super().setGeometry(rect)
        self._place(rect, move=True)

    def sizeHint(self) -> QSize:
        return self.minimumSize()

    def minimumSize(self) -> QSize:
        size = QSize()
        for item in self.items:
            size = size.expandedTo(item.minimumSize())
        m = self.contentsMargins()
        return size + QSize(m.left() + m.right(), m.top() + m.bottom())

    def _place(self, rect: QRect, move: bool) -> int:
        m = self.contentsMargins()
        r = rect.adjusted(m.left(), m.top(), -m.right(), -m.bottom())
        x, y, line = r.x(), r.y(), 0
        gap = self.spacing()
        for item in self.items:
            hint = item.sizeHint()
            if x + hint.width() > r.right() + 1 and line > 0:
                x, y, line = r.x(), y + line + gap, 0
            if move:
                item.setGeometry(QRect(QPoint(x, y), hint))
            x += hint.width() + gap
            line = max(line, hint.height())
        return y + line - rect.y() + m.bottom()


def _link(text: str, url: str, tip: str, parent: QWidget) -> QLabel:
    label = QLabel(f'<a href="{url}">{text}</a>', parent)
    label.setTextFormat(Qt.TextFormat.RichText)
    label.setOpenExternalLinks(False)
    label.linkActivated.connect(lambda u: QDesktopServices.openUrl(QUrl(u)))
    label.setToolTip(tip)
    return label


class AboutDialog(QDialog):
    def __init__(self, engine_name: str, version: str, parent: QWidget | None = None, data: cr.Credits | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("about")
        self.setWindowTitle(tr("About CFA4"))
        self.credits = data if data is not None else cr.load()
        self.open_url: Callable[[str], object] = lambda u: QDesktopServices.openUrl(QUrl(u))
        # the credits view (egui's View): names, sort key and direction, grab bag or table
        self.names = "username"
        self.key = "firstCommit"
        self.ascending = True
        self.table = False

        tabs = QHBoxLayout()
        tabs.setSpacing(2)
        self.tab_group = QButtonGroup(self)
        self.stack = QStackedWidget(self)
        self.tabs: dict[str, QToolButton] = {}
        for i, (tid, label, page) in enumerate(
            (
                ("about", tr("About"), self._about_page(engine_name, version)),
                ("contributors", tr("Contributors"), self._contributors_page()),
                ("models", tr("Models"), self._models_page()),
            )
        ):
            b = QToolButton(self)
            b.setObjectName(f"about.tab.{tid}")
            b.setText(label)
            b.setCheckable(True)
            b.setAutoRaise(True)
            self.tab_group.addButton(b, i)
            tabs.addWidget(b)
            self.tabs[tid] = b
            self.stack.addWidget(page)
        tabs.addStretch(1)
        self.tab_group.idClicked.connect(self.stack.setCurrentIndex)
        self.tabs["about"].setChecked(True)
        line = QFrame(self)
        line.setFrameShape(QFrame.Shape.HLine)
        layout = QVBoxLayout(self)
        layout.addLayout(tabs)
        layout.addWidget(line)
        layout.addWidget(self.stack, 1)
        self._fill_contributors()

    def show_tab(self, tid: str) -> None:
        self.tabs[tid].click()

    # ----- About -----
    def _about_page(self, engine_name: str, version: str) -> QWidget:
        page = QWidget(self)
        lay = QVBoxLayout(page)
        logo = QLabel(page)
        logo.setObjectName("about.logo")
        logo.setPixmap(logo_pixmap(180))
        lay.addWidget(logo)
        heading = QLabel("CFA4", page)
        f = heading.font()
        f.setPointSizeF(f.pointSizeF() * 1.6)
        f.setBold(True)
        heading.setFont(f)
        lay.addWidget(heading)
        lay.addWidget(QLabel(fmt("Version {version}", version=version), page))
        if engine_name:
            lay.addWidget(QLabel(engine_name, page))
        lay.addWidget(QLabel("Linux video editor derived from CFA4.", page))
        lay.addSpacing(10)
        for lid, label, url, primary in (("github", "CFA4 on GitHub", GITHUB, True), ("reportIssue", "Report a CFA4 issue", ISSUES, False)):
            b = QPushButton(label, page)
            b.setObjectName(f"about.{lid}")
            b.setToolTip(url)
            b.setMinimumHeight(36 if primary else 28)
            b.setFlat(not primary)
            b.setDefault(primary)
            b.clicked.connect(lambda _c=False, u=url: self.open_url(u))
            lay.addWidget(b)
        lay.addSpacing(10)
        lic = QLabel(tr("GPL-3.0-or-later. Fonts: Inter, Noto Serif and JetBrains Mono (SIL OFL 1.1)."), page)
        lic.setObjectName("about.licence")
        lic.setWordWrap(True)
        lic.setStyleSheet(f"color: {theme.current().text_dim};")
        lay.addWidget(lic)
        lay.addStretch(1)
        page.setMinimumWidth(380)
        return page

    # ----- Contributors -----
    def _contributors_page(self) -> QWidget:
        page = QWidget(self)
        page.setMinimumWidth(680)
        bar = FlowLayout(spacing=6)
        bar.addWidget(QLabel(tr("Show"), page))
        self.name_buttons: dict[str, QToolButton] = {}
        group = QButtonGroup(page)
        for mode, label in cr.NAME_MODES:
            b = QToolButton(page)
            b.setObjectName(f"about.credits.names.{mode}")
            b.setText(tr(label))
            b.setCheckable(True)
            b.setAutoRaise(True)
            b.setChecked(mode == self.names)
            b.clicked.connect(lambda _c=False, m=mode: self.set_view(names=m))
            group.addButton(b)
            bar.addWidget(b)
            self.name_buttons[mode] = b
        bar.addWidget(QLabel(tr("Sort"), page))
        self.sort = QComboBox(page)
        self.sort.setObjectName("about.credits.sort")
        for key, label, _h in cr.SORT_KEYS:
            self.sort.addItem(tr(label), key)
        self.sort.setCurrentIndex(self.sort.findData(self.key))
        self.sort.activated.connect(lambda i: self.set_view(key=self.sort.itemData(i), ascending=cr.default_ascending(self.sort.itemData(i))))
        bar.addWidget(self.sort)
        self.reverse = QToolButton(page)
        self.reverse.setObjectName("about.credits.reverse")
        self.reverse.setToolTip(tr("Reverse the order"))
        self.reverse.clicked.connect(lambda: self.set_view(ascending=not self.ascending))
        bar.addWidget(self.reverse)
        view = QButtonGroup(page)
        self.grab_bag_button = QToolButton(page)
        self.grab_bag_button.setObjectName("about.credits.grabBag")
        self.grab_bag_button.setText(tr("Grab bag"))
        self.table_button = QToolButton(page)
        self.table_button.setObjectName("about.credits.table")
        self.table_button.setText(tr("Table"))
        for b, table in ((self.grab_bag_button, False), (self.table_button, True)):
            b.setCheckable(True)
            b.setAutoRaise(True)
            b.clicked.connect(lambda _c=False, t=table: self.set_view(table=t))
            view.addButton(b)
            bar.addWidget(b)
        self.grab_bag_button.setChecked(True)
        self.count = QLabel(page)
        self.count.setObjectName("about.credits.count")
        self.count.setStyleSheet(f"color: {theme.current().text_dim};")
        self.scroll = QScrollArea(page)
        self.scroll.setObjectName("about.credits.list")
        self.scroll.setWidgetResizable(True)
        self.scroll.setFixedHeight(LIST_HEIGHT)
        lay = QVBoxLayout(page)
        lay.addLayout(bar)
        lay.addWidget(self.count)
        lay.addWidget(self.scroll, 1)
        return page

    def set_view(self, names: str | None = None, key: str | None = None, ascending: bool | None = None, table: bool | None = None) -> None:
        if names is not None:
            self.names = names
        if key is not None:
            self.key = key
        if ascending is not None:
            self.ascending = ascending
        if table is not None:
            self.table = table
        self._fill_contributors()

    def _sort_by_header(self, key: str) -> None:
        if key == self.key:
            self.set_view(ascending=not self.ascending)
        else:
            self.set_view(key=key, ascending=cr.default_ascending(key))

    def shown(self) -> list[cr.Contributor]:
        return cr.sorted_contributors(self.credits.contributors, self.names, self.key, self.ascending)

    def _fill_contributors(self) -> None:
        for mode, b in self.name_buttons.items():
            b.setChecked(mode == self.names)
        self.sort.setCurrentIndex(self.sort.findData(self.key))
        self.reverse.setText("▲" if self.ascending else "▼")
        (self.table_button if self.table else self.grab_bag_button).setChecked(True)
        people = self.shown()
        self.count.setText(f"{len(people)} contributors · {cr.group(self.credits.total_commits)} commits")
        body = QWidget()
        body.setObjectName("about.credits.body")
        if not people:
            lay = QVBoxLayout(body)
            lay.addWidget(QLabel(tr("No contributor data was built into this copy."), body))
            lay.addStretch(1)
        elif self.table:
            grid = QGridLayout(body)
            grid.setHorizontalSpacing(12)
            grid.setVerticalSpacing(2)
            for i, (key, label, header) in enumerate(cr.SORT_KEYS):
                arrow = (" ▲" if self.ascending else " ▼") if key == self.key else ""
                b = QToolButton(body)
                b.setObjectName(f"about.credits.header.{i}")
                b.setText(header + arrow)
                b.setToolTip(tr(label))
                b.setAutoRaise(True)
                f = b.font()
                f.setBold(True)
                b.setFont(f)
                b.clicked.connect(lambda _c=False, k=key: self._sort_by_header(k))
                grid.addWidget(b, 0, i)
            for row, c in enumerate(people, start=1):
                grid.addWidget(_link(c.name(self.names), f"https://github.com/{c.login}", c.summary(), body), row, 0)
                cells = (
                    cr.group(c.prs),
                    cr.group(c.commits),
                    cr.group(c.lines_added),
                    cr.group(c.lines_deleted),
                    cr.signed(c.lines_delta),
                    cr.group(c.binary_added),
                    cr.group(c.binary_deleted),
                    cr.day(c.first_commit),
                    cr.day(c.last_commit),
                )
                for col, text in enumerate(cells, start=1):
                    grid.addWidget(QLabel(text, body), row, col)
            grid.setRowStretch(len(people) + 1, 1)
            grid.setColumnStretch(len(cr.SORT_KEYS), 1)
        else:
            flow = FlowLayout(body, spacing=4)
            dim = theme.current().text_dim
            for i, c in enumerate(people):
                if i > 0:
                    dot = QLabel("·", body)
                    dot.setStyleSheet(f"color: {dim};")
                    flow.addWidget(dot)
                flow.addWidget(_link(c.name(self.names), f"https://github.com/{c.login}", c.summary(), body))
        self.scroll.setWidget(body)

    # ----- Models -----
    def _models_page(self) -> QWidget:
        page = QWidget(self)
        page.setMinimumWidth(680)
        lay = QVBoxLayout(page)
        models = self.credits.models
        if not models:
            lay.addWidget(QLabel(tr("No model credits were built into this copy."), page))
            lay.addStretch(1)
            return page
        body = QWidget()
        body.setObjectName("about.models")
        grid = QGridLayout(body)
        grid.setHorizontalSpacing(12)
        for col, h in enumerate((tr("Company"), tr("Model"), tr("Version"), tr("Commits"), tr("% of all commits"), tr("Lines +/−"))):
            label = QLabel(h, body)
            f = label.font()
            f.setBold(True)
            label.setFont(f)
            grid.addWidget(label, 0, col)
        assisted = max(1, max((m.commits for m in models), default=0))
        total = max(self.credits.total_commits, assisted)
        for row, m in enumerate(models, start=1):
            cells = (
                m.company,
                m.model,
                m.version,
                cr.group(m.commits),
                f"{100.0 * m.commits / total:.0f}%",
                f"+{cr.group(m.lines_added)} / −{cr.group(m.lines_deleted)}",
            )
            for col, text in enumerate(cells):
                grid.addWidget(QLabel(text, body), row, col)
        grid.setRowStretch(len(models) + 1, 1)
        grid.setColumnStretch(6, 1)
        scroll = QScrollArea(page)
        scroll.setWidgetResizable(True)
        scroll.setMaximumHeight(LIST_HEIGHT)
        scroll.setWidget(body)
        lay.addWidget(scroll)
        lay.addStretch(1)
        return page

