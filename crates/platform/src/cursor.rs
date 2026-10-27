//! The mouse position in screen pixels, for when the windowing layer has none.
//!
//! On Linux the windowing layer (winit) reports the pointer during an OS drag-and-drop, so there is
//! no OS call here: the function answers `None` and the UI uses the last pointer position egui saw.
//! Kept as a stable hook for the UI (`hooks.cursor_screen_position`).

/// The cursor's position on the virtual screen in physical pixels, or `None` where the OS cannot
/// say or this platform's windowing layer already reports it during a drag.
pub fn cursor_screen_position() -> Option<(i32, i32)> {
    None
}
