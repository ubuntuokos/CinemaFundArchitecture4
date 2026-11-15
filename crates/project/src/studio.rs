//! The live studio's scenes, kept in the project (CFA4-LIVE-STUDIO-001 §I.4, V.5, phase S1).
//!
//! A scene is an ordered list of sources (bottom first). A source is something to show (a solid
//! colour, a picture file, a line of text…) and where to show it on the studio canvas
//! ([`SourceTransform`]). The engine's `live.studio.scene.*` / `live.studio.source.*` commands
//! edit this model as undoable project edits; the live studio host builds its scene from the
//! current one.
//!
//! Everything here is read from project files, so nothing is trusted: [`LiveStudio::repair`]
//! brings every number into range and drops what cannot be shown, and the bounds below keep a
//! hostile file from growing the project or the host's work without limit.
//!
//! Passwords and stream keys never live here (plan V.5): they belong to the user's keyring.

use serde::{Deserialize, Serialize};

/// Most scenes in a project.
pub const MAX_SCENES: usize = 100;
/// Most sources in one scene.
pub const MAX_SOURCES: usize = 200;
/// Longest scene or source name (characters).
pub const MAX_NAME_CHARS: usize = 200;
/// Longest text of a text source (characters).
pub const MAX_TEXT_CHARS: usize = 10_000;
/// Longest file path of a picture or media source (bytes).
pub const MAX_PATH_BYTES: usize = 4096;
/// Largest side of a colour source (and of any source picture), in pixels.
pub const MAX_SOURCE_SIDE: u32 = 8192;
/// Largest absolute position on the canvas, in pixels.
pub const MAX_POSITION: f64 = 100_000.0;
/// Largest scale factor (and smallest, as `-MAX_SCALE`: a negative scale flips).
pub const MAX_SCALE: f64 = 100.0;
/// Smallest text size, in pixels.
pub const MIN_TEXT_SIZE: f64 = 4.0;
/// Largest text size, in pixels.
pub const MAX_TEXT_SIZE: f64 = 1000.0;

/// The live studio of a project: its scenes and which one is on the program.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LiveStudio {
    /// The scenes, in the order the scene list shows them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scenes: Vec<StudioScene>,
    /// The scene on the program (the host shows it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<u64>,
}

/// One scene.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StudioScene {
    /// Project-wide id (from `Project::alloc_id`).
    pub id: u64,
    pub name: String,
    /// The sources, bottom first (the last one is drawn on top).
    #[serde(default)]
    pub sources: Vec<StudioSource>,
}

/// One source of a scene.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StudioSource {
    /// Project-wide id (from `Project::alloc_id`).
    pub id: u64,
    pub name: String,
    /// What it shows.
    pub kind: SourceKind,
    /// Where it shows it.
    #[serde(default)]
    pub transform: SourceTransform,
    /// Hidden sources stay in the scene but are not drawn (the eye in the source list).
    #[serde(default = "yes")]
    pub visible: bool,
}

fn yes() -> bool {
    true
}

/// What a source shows. Colours are sRGB-encoded straight RGBA, 8 bits per channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SourceKind {
    /// A solid rectangle.
    Color { color: [u8; 4], width: u32, height: u32 },
    /// A still picture file (any format the editor imports).
    Image { path: String },
    /// A line (or lines) of text in a font of the editor.
    #[serde(rename_all = "camelCase")]
    Text {
        text: String,
        #[serde(default = "default_font")]
        font: String,
        #[serde(default = "default_text_size")]
        size: f64,
        #[serde(default = "white")]
        color: [u8; 4],
    },
}

fn default_font() -> String {
    "Inter".into()
}

fn default_text_size() -> f64 {
    96.0
}

fn white() -> [u8; 4] {
    [255, 255, 255, 255]
}

impl SourceKind {
    /// The type name used by the commands (`color`, `image`, `text`).
    pub fn type_name(&self) -> &'static str {
        match self {
            SourceKind::Color { .. } => "color",
            SourceKind::Image { .. } => "image",
            SourceKind::Text { .. } => "text",
        }
    }

    /// A default text source.
    pub fn text(text: &str) -> Self {
        SourceKind::Text { text: text.into(), font: default_font(), size: default_text_size(), color: white() }
    }
}

/// Where a source sits on the studio canvas, as OBS Studio's scene items place it: the cropped
/// picture is scaled, rotated (degrees, clockwise on screen) about its top-left corner, and that
/// corner is put at `x`, `y` (canvas pixels).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceTransform {
    pub x: f64,
    pub y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    /// Degrees, clockwise on screen.
    pub rotation: f64,
    /// Pixels cut from each edge of the source picture before scaling: left, top, right, bottom.
    #[serde(default)]
    pub crop: [u32; 4],
}

impl Default for SourceTransform {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, scale_x: 1.0, scale_y: 1.0, rotation: 0.0, crop: [0; 4] }
    }
}

impl SourceTransform {
    /// The transform with every number in range, or `None` when one is not finite.
    pub fn checked(self) -> Option<Self> {
        let all = [self.x, self.y, self.scale_x, self.scale_y, self.rotation];
        if all.iter().any(|v| !v.is_finite()) {
            return None;
        }
        Some(Self {
            x: self.x.clamp(-MAX_POSITION, MAX_POSITION),
            y: self.y.clamp(-MAX_POSITION, MAX_POSITION),
            scale_x: self.scale_x.clamp(-MAX_SCALE, MAX_SCALE),
            scale_y: self.scale_y.clamp(-MAX_SCALE, MAX_SCALE),
            rotation: self.rotation.rem_euclid(360.0),
            crop: self.crop.map(|c| c.min(MAX_SOURCE_SIDE)),
        })
    }
}

fn clip_chars(s: &mut String, max: usize) {
    if let Some((i, _)) = s.char_indices().nth(max) {
        s.truncate(i);
    }
}

impl StudioSource {
    /// The source made usable, or `None` when it cannot be shown at all (a non-finite number,
    /// an empty or too long path).
    fn repaired(mut self) -> Option<Self> {
        clip_chars(&mut self.name, MAX_NAME_CHARS);
        self.transform = self.transform.checked()?;
        match &mut self.kind {
            SourceKind::Color { width, height, .. } => {
                *width = (*width).clamp(1, MAX_SOURCE_SIDE);
                *height = (*height).clamp(1, MAX_SOURCE_SIDE);
            }
            SourceKind::Image { path } => {
                if path.is_empty() || path.len() > MAX_PATH_BYTES || path.contains('\0') {
                    return None;
                }
            }
            SourceKind::Text { text, font, size, .. } => {
                clip_chars(text, MAX_TEXT_CHARS);
                clip_chars(font, MAX_NAME_CHARS);
                if !size.is_finite() {
                    return None;
                }
                *size = size.clamp(MIN_TEXT_SIZE, MAX_TEXT_SIZE);
            }
        }
        Some(self)
    }
}

impl LiveStudio {
    /// True when there are no scenes (the project file then has no `studio` key).
    pub fn is_empty(&self) -> bool {
        self.scenes.is_empty() && self.current.is_none()
    }

    /// The scene `id`.
    pub fn scene(&self, id: u64) -> Option<&StudioScene> {
        self.scenes.iter().find(|s| s.id == id)
    }

    /// The scene `id`, to change it.
    pub fn scene_mut(&mut self, id: u64) -> Option<&mut StudioScene> {
        self.scenes.iter_mut().find(|s| s.id == id)
    }

    /// The scene on the program.
    pub fn current_scene(&self) -> Option<&StudioScene> {
        self.current.and_then(|id| self.scene(id))
    }

    /// The scene holding source `id`, and the source's index in it.
    pub fn find_source(&self, id: u64) -> Option<(u64, usize)> {
        self.scenes.iter().find_map(|s| s.sources.iter().position(|x| x.id == id).map(|i| (s.id, i)))
    }

    /// Every id in use (scenes and sources).
    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.scenes.iter().flat_map(|s| std::iter::once(s.id).chain(s.sources.iter().map(|x| x.id)))
    }

    /// Makes a studio read from a file usable: at most [`MAX_SCENES`] scenes of at most
    /// [`MAX_SOURCES`] sources, names and texts cut to their limits, numbers in range, duplicate
    /// ids and sources that cannot be shown dropped, and `current` pointing at a scene (or none).
    /// Returns how many scenes and sources were dropped.
    pub fn repair(&mut self) -> usize {
        let mut dropped = 0;
        let mut seen = std::collections::HashSet::new();
        if self.scenes.len() > MAX_SCENES {
            dropped += self.scenes.len() - MAX_SCENES;
            self.scenes.truncate(MAX_SCENES);
        }
        let scenes = std::mem::take(&mut self.scenes);
        for mut scene in scenes {
            if !seen.insert(scene.id) {
                dropped += 1;
                continue;
            }
            clip_chars(&mut scene.name, MAX_NAME_CHARS);
            if scene.sources.len() > MAX_SOURCES {
                dropped += scene.sources.len() - MAX_SOURCES;
                scene.sources.truncate(MAX_SOURCES);
            }
            let sources = std::mem::take(&mut scene.sources);
            for src in sources {
                if !seen.insert(src.id) {
                    dropped += 1;
                    continue;
                }
                match src.repaired() {
                    Some(s) => scene.sources.push(s),
                    None => dropped += 1,
                }
            }
            self.scenes.push(scene);
        }
        if self.current.is_some_and(|c| self.scene(c).is_none()) {
            self.current = self.scenes.first().map(|s| s.id);
        }
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(id: u64) -> StudioSource {
        StudioSource {
            id,
            name: format!("Color {id}"),
            kind: SourceKind::Color { color: [10, 20, 30, 255], width: 100, height: 50 },
            transform: SourceTransform::default(),
            visible: true,
        }
    }

    #[test]
    fn the_wire_form_is_stable_and_reads_back() {
        let st = LiveStudio {
            scenes: vec![StudioScene { id: 3, name: "Main".into(), sources: vec![color(4), StudioSource { kind: SourceKind::text("Hi"), ..color(5) }] }],
            current: Some(3),
        };
        let json = serde_json::to_string(&st).expect("json");
        assert!(json.contains(r#""kind":{"type":"color","color":[10,20,30,255],"width":100,"height":50}"#), "{json}");
        assert!(json.contains(r#""type":"text","text":"Hi","font":"Inter","size":96.0"#), "{json}");
        assert!(json.contains(r#""scaleX":1.0"#), "{json}");
        let back: LiveStudio = serde_json::from_str(&json).expect("read");
        assert_eq!(back, st);
        // missing optional fields take their defaults
        let short: StudioSource = serde_json::from_str(r#"{"id":1,"name":"T","kind":{"type":"text","text":"x"}}"#).expect("short");
        assert!(short.visible);
        assert_eq!(short.transform, SourceTransform::default());
        assert_eq!(short.kind, SourceKind::text("x"));
        assert!(LiveStudio::default().is_empty());
    }

    #[test]
    fn hostile_studios_are_repaired() {
        let mut bad_t = color(11);
        bad_t.transform.x = f64::NAN;
        let mut huge = color(12);
        huge.kind = SourceKind::Color { color: [0; 4], width: u32::MAX, height: 0 };
        huge.transform = SourceTransform { x: 1e12, y: -1e12, scale_x: 1e9, scale_y: -1e9, rotation: -90.0, crop: [u32::MAX; 4] };
        huge.name = "n".repeat(10_000);
        let empty_path = StudioSource { kind: SourceKind::Image { path: String::new() }, ..color(13) };
        let nan_text = StudioSource { kind: SourceKind::Text { text: "x".repeat(20_000), font: "F".into(), size: f64::INFINITY, color: [0; 4] }, ..color(14) };
        let big_text = StudioSource { kind: SourceKind::Text { text: "t".into(), font: "F".into(), size: 1e9, color: [0; 4] }, ..color(15) };
        let mut st = LiveStudio {
            scenes: vec![
                StudioScene { id: 1, name: "A".into(), sources: vec![bad_t, huge, empty_path, nan_text, big_text, color(1)] },
                StudioScene { id: 1, name: "duplicate".into(), sources: vec![] },
            ],
            current: Some(99),
        };
        let dropped = st.repair();
        assert_eq!(dropped, 5, "NaN transform, empty path, infinite text size, duplicate source id, duplicate scene");
        assert_eq!(st.scenes.len(), 1);
        assert_eq!(st.current, Some(1));
        let s = &st.scenes[0].sources;
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].kind, SourceKind::Color { color: [0; 4], width: MAX_SOURCE_SIDE, height: 1 });
        assert_eq!(s[0].transform.x, MAX_POSITION);
        assert_eq!(s[0].transform.scale_y, -MAX_SCALE);
        assert_eq!(s[0].transform.rotation, 270.0);
        assert_eq!(s[0].transform.crop, [MAX_SOURCE_SIDE; 4]);
        assert_eq!(s[0].name.chars().count(), MAX_NAME_CHARS);
        assert!(matches!(s[1].kind, SourceKind::Text { size, .. } if size == MAX_TEXT_SIZE));

        let mut many = LiveStudio {
            scenes: (0..MAX_SCENES as u64 + 5).map(|i| StudioScene { id: 1000 + i, name: String::new(), sources: vec![] }).collect(),
            current: None,
        };
        assert_eq!(many.repair(), 5);
        assert_eq!(many.scenes.len(), MAX_SCENES);
        assert_eq!(many.current, None, "no current scene stays none");
    }

    #[test]
    fn sources_are_found_by_id() {
        let st = LiveStudio { scenes: vec![StudioScene { id: 1, name: "A".into(), sources: vec![color(2), color(3)] }], current: Some(1) };
        assert_eq!(st.find_source(3), Some((1, 1)));
        assert_eq!(st.find_source(9), None);
        assert_eq!(st.current_scene().map(|s| s.id), Some(1));
        assert_eq!(st.ids().collect::<Vec<_>>(), vec![1, 2, 3]);
    }
}
