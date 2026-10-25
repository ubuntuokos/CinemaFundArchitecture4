//! Read-only compatibility with files written by the upstream application CFA4 was forked from.
//!
//! CFA4 renamed every on-disk `format` tag when it took its own identity (roadmap A1), but the
//! owner's decision of 2026-10-24 is that projects (and the small preset/template files that travel
//! with them) saved by the upstream application must keep opening in CFA4. Readers therefore accept
//! the upstream tag as a synonym of the CFA4 one; writers only ever emit the CFA4 tag, so a file is
//! upgraded the first time it is saved from CFA4.
//!
//! This is the only place in the tree where the upstream format names may appear
//! (`python/tests/test_independence.py` allows exactly this file). Do not use these constants for
//! anything but recognising old files.

/// Upstream tag of a project envelope (`.fcproj`); CFA4 writes `cfa4.project`.
pub const PROJECT_FORMAT: &str = "filmcraft.project";
/// Upstream tag of a graphics template (`.fcgt`); CFA4 writes `cfa4.graphicsTemplate`.
pub const GRAPHICS_TEMPLATE_FORMAT: &str = "filmcraft.graphicsTemplate";
/// Upstream tag of an effect preset file; CFA4 writes `cfa4.effect-presets`.
pub const EFFECT_PRESETS_FORMAT: &str = "filmcraft.effect-presets";
/// Upstream tag of an export preset file; CFA4 writes `cfa4.export-presets`.
pub const EXPORT_PRESETS_FORMAT: &str = "filmcraft.export-presets";
/// Upstream tag of a keyboard-shortcut preset; CFA4 writes `cfa4-keyboard-shortcuts`.
pub const KEYBOARD_SHORTCUTS_FORMAT: &str = "filmcraft-keyboard-shortcuts";

/// Every `(cfa4 tag, upstream tag)` pair readers accept.
const SYNONYMS: [(&str, &str); 5] = [
    ("cfa4.project", PROJECT_FORMAT),
    ("cfa4.graphicsTemplate", GRAPHICS_TEMPLATE_FORMAT),
    ("cfa4.effect-presets", EFFECT_PRESETS_FORMAT),
    ("cfa4.export-presets", EXPORT_PRESETS_FORMAT),
    ("cfa4-keyboard-shortcuts", KEYBOARD_SHORTCUTS_FORMAT),
];

/// True when a file's `format` tag `found` is the CFA4 tag `expected` or its upstream synonym.
#[must_use]
pub fn format_matches(found: &str, expected: &str) -> bool {
    found == expected || SYNONYMS.iter().any(|&(cfa4, upstream)| cfa4 == expected && found == upstream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_own_and_upstream_tags_only() {
        for (cfa4, upstream) in SYNONYMS {
            assert!(format_matches(cfa4, cfa4));
            assert!(format_matches(upstream, cfa4));
            assert!(!format_matches("something.else", cfa4));
        }
        // A synonym is tied to its own kind of file: an upstream project tag is not a template.
        assert!(!format_matches(PROJECT_FORMAT, "cfa4.graphicsTemplate"));
        // Unknown expected tags only match themselves.
        assert!(format_matches("x", "x"));
        assert!(!format_matches(PROJECT_FORMAT, "x"));
    }
}
