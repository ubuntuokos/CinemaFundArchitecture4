"""Tools menu: the editor's tools grouped by what they work on (owner request, 2026-11-10).

- **Video Tools**: GOP Smart Cut of the Source monitor's clip (moved here from the Source
  monitor's wrench menu), the Export window's Smart Export, and the timeline's GOP boundary
  marks (WP-08 of CFA4-SMART-CUT-RENDER-R1).
- **Audio Tools**: the engine's audio commands (Audio Gain, Add Beat Markers) and the audio
  panels.
- **Publication Tools**: export and delivery (Export Media, Quick Export, Check Before Export,
  Social Deliverables, Batch Variants).

The menu holds no logic of its own: engine commands reuse the registry's actions (same label,
shortcut and enabled state as in their engine menu), the rest call the window's existing
methods. Every entry has an automation id (``tools.<group>.<name>``); a command the running
engine does not offer is left out.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from PySide6.QtGui import QAction
from PySide6.QtWidgets import QMenu

from .i18n import N_, tr

if TYPE_CHECKING:
    from .main_window import MainWindow

# (automation id, panel name) of the audio panels the Audio Tools menu shows
AUDIO_PANELS = (
    ("tools.audio.essentialSound", N_("Essential Sound")),
    ("tools.audio.trackMixer", N_("Audio Track Mixer")),
    ("tools.audio.clipMixer", N_("Audio Clip Mixer")),
    ("tools.audio.meters", N_("Audio Meters")),
    ("tools.audio.textToSpeech", N_("Text to Speech")),
)
AUDIO_COMMANDS = ("clip.audioGain", "markers.addBeats")
PUBLICATION_COMMANDS = ("sequence.checkDelivery", "export.socialDeliverables", "export.batchVariants")


def _engine_actions(win: MainWindow, menu: QMenu, ids: tuple[str, ...]) -> None:
    for cid in ids:
        action = win.registry.actions.get(cid)
        if action is not None:
            menu.addAction(action)


def _show_dock(win: MainWindow, name: str) -> None:
    dock = win.docks.get(name)
    if dock is not None:
        dock.show()
        dock.raise_()


def open_smart_export(win: MainWindow) -> None:
    """The Export window with a Smart Export mode chosen (Automatic unless one already is)."""
    win.open_export()
    box = getattr(win.export_dialog, "smart", None)
    if box is not None and not box.smart():
        i = box.mode.findData("auto")
        if i >= 0:
            box.mode.setCurrentIndex(i)


def build_tools_menu(win: MainWindow, menu: QMenu) -> None:
    menu.setObjectName("menu.Tools")

    video = menu.addMenu(tr("Video Tools"))
    video.setObjectName("menu.Tools.Video")
    smart_cut = video.addAction(tr("Smart Cut Source…"))
    smart_cut.setObjectName("tools.video.smartCut")
    smart_cut.setToolTip(tr("Open a video clip in the Source monitor first."))
    smart_cut.triggered.connect(lambda _c=False: win.source_monitor.open_smart_cut())
    find_segments = video.addAction(tr("Find Segments…"))
    find_segments.setObjectName("tools.video.findSegments")
    find_segments.setToolTip(tr("Open a video clip in the Source monitor first."))
    find_segments.triggered.connect(lambda _c=False: win.source_monitor.open_find_segments())
    smart_export = video.addAction(tr("Smart Export…"))
    smart_export.setObjectName("tools.video.smartExport")
    smart_export.triggered.connect(lambda _c=False: open_smart_export(win))
    video.addSeparator()
    gop = video.addAction(tr("Show GOP Boundaries"))
    gop.setObjectName("tools.video.showGopMarks")
    gop.setCheckable(True)
    gop.toggled.connect(lambda on: win.timeline.canvas.set_show_gop_marks(on))

    def video_shown() -> None:
        src = win.source_monitor
        smart_cut.setEnabled(src.source is not None and src.has_video())
        find_segments.setEnabled(src.source is not None and src.has_video())
        smart_export.setEnabled(win.sequence is not None)
        gop.blockSignals(True)
        gop.setChecked(bool(win.timeline.canvas.show_gop_marks))
        gop.blockSignals(False)

    video.aboutToShow.connect(video_shown)
    video_shown()

    audio = menu.addMenu(tr("Audio Tools"))
    audio.setObjectName("menu.Tools.Audio")
    _engine_actions(win, audio, AUDIO_COMMANDS)
    if audio.actions():
        audio.addSeparator()
    for oid, name in AUDIO_PANELS:
        if name not in win.docks:
            continue
        a = audio.addAction(tr(name))
        a.setObjectName(oid)
        a.triggered.connect(lambda _c=False, n=name: _show_dock(win, n))

    publication = menu.addMenu(tr("Publication Tools"))
    publication.setObjectName("menu.Tools.Publication")
    export = QAction(tr("Export Media…"), publication)
    export.setObjectName("tools.publication.exportMedia")
    export.triggered.connect(lambda _c=False: win.open_export())
    publication.addAction(export)
    quick = publication.addAction(tr("Quick Export"))
    quick.setObjectName("tools.publication.quickExport")
    quick.triggered.connect(lambda _c=False: win.toggle_quick_export())
    # the popup opens below the header's Quick Export button
    quick.setEnabled(getattr(win, "quick_export_button", None) is not None)
    publication.aboutToShow.connect(
        lambda: quick.setEnabled(getattr(win, "quick_export_button", None) is not None and win.quick_export_button.isVisible())
    )
    publication.addSeparator()
    _engine_actions(win, publication, PUBLICATION_COMMANDS)
