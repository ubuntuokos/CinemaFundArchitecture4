# F0: Qt6 funkcionális akadályok (a `docs/qt-parity.md` géppel kivonatolt hiánylistája)

> **Eredet:** `docs/qt-parity.md` (a Qt 6 GUI és az egui sorról sorra), a Qt-oszlop szerint nem ✅ sorok. A szöveg a forrásból van kimásolva,
> nem újraírva (a hosszú cellák csonkítva); a lista újragenerálható ugyanezzel a szűréssel. A sor ✅ állapotát a forrásdokumentum szerint teszt vagy rögzített
> kézi próba támasztja alá. A terv „Qt6 paritáshibák: ismertek, listájuk nincs csatolva” sora ezzel a listával pótolva; a terv ~53%-os becslése közlés,
> nem ennek a listának az eredménye (a pontok száma nem munkamennyiség).

**Összesítés (67 pont):** ✅ 63 · 🟡 4 · ❌ 0 · nem besorolt vagy n/a 0.

**Nyitott pontok** (a forrás állapotjelzése szerint):

| Pont | Terület | Állapot | Mi hiányzik (a forrás szerint) |
|---|---|---|---|
| 1.9 | Wayland and X11 on every Ubuntu 26.04 flavour (1. Foundation) | 🟡 | 🟡 the Qt GUI on a real X11 server (Xvfb, `xcb`) and a real Wayland compositor (weston headless, `wayland`), not only Qt's `offscreen`: it starts, the control channel drives it, `ui.resize` goes through the window system, a tool shortcut sent by `ui.key` reaches the timeline although the window never takes the keyboard (a Wayland client cannot; Qt's shortcut matching is pointed at the window inside Qt only), `ui.screenshot` draws the panels and `app.quit` exits cleanly; CI runs it in the Qt 6 … |
| 1.10 | Packaging (.deb, AppImage, Flatpak, tar.gz) with the Qt runtime (1. Foundation) | 🟡 | 🟡 ADR F8 phase 7. The .deb and the tar.gz ship `cfa4-qt` (`packaging/linux/cfa4-qt`) and the GUI's Python package in `lib/cfa4/python`, with the top-level `i18n/` catalogs and their attribution sidecars in `cfa4_qt/i18n`; Python and PySide6 6.10 come from Ubuntu 26.04's archive (`python3-pyside6.*` depends, QtMultimedia recommended), never bundled; the launcher uses the `cfa4-cli` beside it unless `CFA4_ENGINE` names another (`test_qt_gui_installs_and_starts_with_the_engine_beside_it`, `test_… |
| 4.14 | Media quality check (QC report, contact sheet, human review) (4. Modes and dialogs) | 🟡 | 🟡 Quality Check window opened from the Export window's queue (Quality column with the QC state apart from the render status, "Check quality after export"): verdict, reasons, checks with value and bounds, time-coded events, contact sheet, guided questions, findings with severity and time code saved into the report (`qc.*`, docs/media-qc.md; `test_real_engine_quality_check_through_the_export_window`, engine `queued_export_is_checked_and_reviewed`). egui has none (frozen, ADR F8). Playing the ex… |
| 5.5 | GPU frame path (no RGBA copy per frame) (5. Playback) | 🟡 | 🟡 the monitors' frames travel through shared memory (`frame.render {shm}`): the engine writes each RGBA8 frame into a slot of a private ring file, the Qt GUI maps it once (read-only, removed from the directory once mapped) and shows the frame in place, with no copy through the pipe, the reader thread or a QImage (1080p: about 25 ms less per frame, 120 → 97 ms, locally); anything unexpected falls back to the pipe (`test_real_engine_monitor_shows_frames_from_shared_memory`, `test_frame_maps_tak… |
