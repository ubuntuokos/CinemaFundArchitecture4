//! Frame ABI v0 side data (CFA4-RENDER-INTEGRATION-PLAN-001 phase C0, `docs/audit/render-c0/`).
//!
//! The fields the contract needs beyond [`crate::VideoFrame`]: the source the frame came from, a
//! display rotation, HDR static metadata and where the pixels live. They travel *next to* the
//! frame as one optional value, not inside it: `VideoFrame` is built by struct literal in dozens of
//! crates, so a new field would break them all (spec §9, point 4), and most frames carry none of
//! this. A decoder or a worker that knows more fills a [`FrameSideData`]; everything else treats
//! `None` or `FrameSideData::default()` as "nothing signalled".
//!
//! **Pixel layout.** [`crate::PixelData`] is densely packed (no row stride) and shared by `Arc`;
//! the ABI is a value type. A borrowed or strided buffer is described separately at the worker
//! boundary and is never a `VideoFrame`.

use cfa4_color::HdrMetadata;
use serde::{Deserialize, Serialize};

/// Clockwise display rotation to apply when showing the frame (the container's display matrix
/// reduced to the four right angles; any other matrix is not representable and stays `None`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rotation {
    #[default]
    None,
    Cw90,
    Cw180,
    Cw270,
}

impl Rotation {
    /// From degrees clockwise; `None` when not a multiple of 90 (an arbitrary matrix).
    pub fn from_degrees(deg: i32) -> Option<Rotation> {
        match deg.rem_euclid(360) {
            0 => Some(Rotation::None),
            90 => Some(Rotation::Cw90),
            180 => Some(Rotation::Cw180),
            270 => Some(Rotation::Cw270),
            _ => None,
        }
    }

    pub fn degrees(self) -> i32 {
        match self {
            Rotation::None => 0,
            Rotation::Cw90 => 90,
            Rotation::Cw180 => 180,
            Rotation::Cw270 => 270,
        }
    }

    /// Whether the displayed width and height swap.
    pub fn swaps_axes(self) -> bool {
        matches!(self, Rotation::Cw90 | Rotation::Cw270)
    }
}

/// Where the pixels are. Only the CPU case is a [`crate::VideoFrame`]; the GPU case is the
/// description a worker or the GPU renderer hands over, bound to the types in `crates/gpu` later.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Residency {
    #[default]
    Cpu,
    Gpu {
        /// Render node or adapter name.
        device: String,
        /// A fence value to wait on before reading, when the producer signals one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fence: Option<u64>,
    },
}

/// Optional per-frame data of the Frame ABI. Every field is "not signalled" by default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameSideData {
    /// Which source (media item, clip or generator) the frame was made from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<Rotation>,
    /// SMPTE ST 2086 / CTA-861.3 static metadata (the existing colour crate type).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr: Option<HdrMetadata>,
    #[serde(default)]
    pub residency: Residency,
}

impl FrameSideData {
    /// The size a viewer shows for a `width`×`height` frame under this rotation.
    pub fn display_size(&self, width: u32, height: u32) -> (u32, u32) {
        if self.rotation.is_some_and(Rotation::swaps_axes) { (height, width) } else { (width, height) }
    }
}

/// The field names of [`crate::VideoFrame`] the ABI v0 contract covers, in declaration order.
/// A change to `VideoFrame` must change this list and the spec together (see the test).
pub const VIDEO_FRAME_FIELDS: [&str; 7] = ["width", "height", "data", "color", "par", "pts", "layout"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FrameLayout, PixelData, VideoFrame};
    use cfa4_color::ColorInfo;
    use cfa4_time::Tick;

    /// Exhaustive destructuring: adding, removing or renaming a `VideoFrame` field stops this
    /// from compiling until the contract list and the spec are updated.
    #[test]
    fn the_video_frame_fields_are_the_contract_list() {
        fn names(f: VideoFrame) -> [&'static str; 7] {
            let VideoFrame { width: _, height: _, data: _, color: _, par: _, pts: _, layout: _ } = f;
            VIDEO_FRAME_FIELDS
        }
        let f = VideoFrame {
            width: 2,
            height: 2,
            data: PixelData::Rgba8(std::sync::Arc::new(vec![0; 16])),
            color: ColorInfo::default(),
            par: (1, 1),
            pts: Tick(0),
            layout: FrameLayout::default(),
        };
        assert_eq!(names(f), VIDEO_FRAME_FIELDS);
    }

    #[test]
    fn rotation_round_trips_right_angles_and_refuses_the_rest() {
        for deg in [0, 90, 180, 270] {
            assert_eq!(Rotation::from_degrees(deg).unwrap().degrees(), deg);
        }
        assert_eq!(Rotation::from_degrees(-90), Some(Rotation::Cw270));
        assert_eq!(Rotation::from_degrees(450), Some(Rotation::Cw90));
        assert_eq!(Rotation::from_degrees(45), None);
    }

    #[test]
    fn a_quarter_turn_swaps_the_display_size() {
        let s = FrameSideData { rotation: Some(Rotation::Cw90), ..Default::default() };
        assert_eq!(s.display_size(1920, 1080), (1080, 1920));
        assert_eq!(FrameSideData::default().display_size(1920, 1080), (1920, 1080));
        let t = FrameSideData { rotation: Some(Rotation::Cw180), ..Default::default() };
        assert_eq!(t.display_size(1920, 1080), (1920, 1080));
    }

    #[test]
    fn nothing_signalled_serialises_to_the_cpu_default_and_the_wire_names_are_stable() {
        assert_eq!(serde_json::to_string(&FrameSideData::default()).unwrap(), r#"{"residency":{"kind":"cpu"}}"#);
        let s = FrameSideData {
            source: Some("clip-3".into()),
            rotation: Some(Rotation::Cw90),
            hdr: Some(HdrMetadata { max_cll: Some(1000.0), ..Default::default() }),
            residency: Residency::Gpu { device: "card0".into(), fence: Some(7) },
        };
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(
            j,
            r#"{"source":"clip-3","rotation":"cw90","hdr":{"masteringMaxNits":null,"masteringMinNits":null,"maxCll":1000.0,"maxFall":null},"residency":{"kind":"gpu","device":"card0","fence":7}}"#
        );
        assert_eq!(serde_json::from_str::<FrameSideData>(&j).unwrap(), s);
        // an old reader's JSON without any field is "nothing signalled"
        assert_eq!(serde_json::from_str::<FrameSideData>("{}").unwrap(), FrameSideData::default());
    }
}
