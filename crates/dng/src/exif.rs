//! The few EXIF fields CFA4 needs from a camera raw that another decoder parses: make, model and
//! orientation from IFD 0 of an EXIF TIFF block (CIPA DC-008, TIFF 6.0).

use crate::Result;
use crate::tiff::{Tiff, tags};

/// Camera fields of an EXIF block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Camera {
    pub make: Option<String>,
    pub model: Option<String>,
    /// TIFF Orientation 1–8 (1: rows top to bottom, columns left to right).
    pub orientation: Option<u16>,
}

impl Camera {
    /// "Make Model" as DNG profiles name a camera (UniqueCameraModel), e.g. "Panasonic DC-GH6".
    pub fn unique_model(&self) -> Option<String> {
        let mm = [self.make.as_deref(), self.model.as_deref()].into_iter().flatten().map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
        (!mm.is_empty()).then_some(mm)
    }
}

/// Read make, model and orientation from IFD 0 of the EXIF TIFF block `bytes`.
pub fn camera(bytes: &[u8]) -> Result<Camera> {
    let (t, first) = Tiff::new(bytes)?;
    let (ifd, _) = t.read_ifd(first, 0)?;
    let text = |tag| ifd.get(tag).and_then(|e| t.ascii(e).ok()).map(|s| s.trim_end_matches('\0').trim().to_string()).filter(|s| !s.is_empty());
    let orientation = ifd.get(tags::ORIENTATION).and_then(|e| t.uint(e).ok()).and_then(|v| u16::try_from(v).ok()).filter(|v| (1..=8).contains(v));
    Ok(Camera { make: text(tags::MAKE), model: text(tags::MODEL), orientation })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::{Value, write_tiff};

    #[test]
    fn make_model_orientation() {
        for be in [false, true] {
            let b = write_tiff(
                &[(tags::MAKE, Value::Ascii("Panasonic".into())), (tags::MODEL, Value::Ascii("DC-GH6".into())), (tags::ORIENTATION, Value::Short(vec![6]))],
                be,
            );
            let c = camera(&b).unwrap();
            assert_eq!(c.unique_model().as_deref(), Some("Panasonic DC-GH6"));
            assert_eq!(c.orientation, Some(6));
        }
        let b = write_tiff(&[(tags::ORIENTATION, Value::Short(vec![9]))], false);
        assert_eq!(camera(&b).unwrap(), Camera::default());
        assert!(camera(b"II*\0\xff\xff\xff\x7f").is_err());
        assert!(camera(b"").is_err());
    }
}
