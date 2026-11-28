//! DNG camera profiles (`.dcp` files): the colour calibration of one camera model, kept outside
//! the raw file. DNG specification 1.4, "Camera Profile File Format": a TIFF file with the magic
//! number 0x4352 whose IFD 0 carries the profile tags.
//!
//! Only the matrix part of a profile is read: UniqueCameraModel, ProfileName, ColorMatrix1/2,
//! ForwardMatrix1/2, CalibrationIlluminant1/2 and BaselineExposureOffset. The hue / saturation
//! maps, the look table and the tone curve are not applied, so a picture developed with a
//! profile matches the camera's colour matrix rendering, not the profile's full "look".
//!
//! CFA4 ships no profiles. A user installs them (for example the ones that come with Adobe's free
//! DNG Converter) and CFA4 matches them to a raw file by camera model.

use crate::color::{Calibration, M3};
use crate::tiff::{Entry, Ifd, Tiff, tags};
use crate::{Error, Result};

/// A parsed camera profile.
#[derive(Clone, Debug)]
pub struct Profile {
    /// UniqueCameraModel: the camera the profile is for, e.g. "Panasonic DC-GH6".
    pub camera_model: Option<String>,
    /// ProfileName, e.g. "Adobe Standard".
    pub name: Option<String>,
    /// The colour matrices, illuminants and BaselineExposureOffset (as `baseline_exposure`).
    /// No camera neutral: that comes from the raw file.
    pub calibration: Calibration,
}

/// Whether `b` starts like a camera profile.
pub fn sniff(b: &[u8]) -> bool {
    matches!(b.get(0..4), Some(b"IIRC") | Some(b"MMCR"))
}

struct Reader<'a> {
    tiff: Tiff<'a>,
    ifd: Ifd,
}

impl Reader<'_> {
    fn entry(&self, tag: u16) -> Option<&Entry> {
        self.ifd.get(tag)
    }

    fn matrix(&self, tag: u16) -> Result<Option<M3>> {
        let Some(e) = self.entry(tag) else { return Ok(None) };
        let v = self.tiff.floats(e)?;
        let Ok(v) = <[f64; 9]>::try_from(v.as_slice()) else { return Err(Error::Malformed("profile matrix is not 3 × 3")) };
        Ok(Some([[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]]))
    }

    fn text(&self, tag: u16) -> Option<String> {
        let e = self.entry(tag)?;
        // UniqueCameraModel is ASCII; ProfileName may be BYTE (UTF-8) in newer profiles
        let s = self.tiff.ascii(e).ok().or_else(|| self.tiff.value_bytes(e).ok().map(|b| String::from_utf8_lossy(b).into_owned()))?;
        let s = s.trim_end_matches('\0').trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

/// Parse a camera profile. A profile without ColorMatrix1 is an error: it has nothing CFA4 can
/// use.
pub fn parse(bytes: &[u8]) -> Result<Profile> {
    let (tiff, first) = Tiff::new_profile(bytes)?;
    let (ifd, _) = tiff.read_ifd(first, 0)?;
    let r = Reader { tiff, ifd };
    let illum = |tag| r.entry(tag).map(|e| r.tiff.uint(e)).transpose();
    let calibration = Calibration {
        color_matrix: [r.matrix(tags::COLOR_MATRIX_1)?, r.matrix(tags::COLOR_MATRIX_2)?],
        camera_calibration: [None, None],
        forward_matrix: [r.matrix(tags::FORWARD_MATRIX_1)?, r.matrix(tags::FORWARD_MATRIX_2)?],
        illuminant: [illum(tags::CALIBRATION_ILLUMINANT_1)?, illum(tags::CALIBRATION_ILLUMINANT_2)?],
        analog_balance: None,
        as_shot_neutral: None,
        as_shot_white_xy: None,
        baseline_exposure: r.entry(tags::BASELINE_EXPOSURE_OFFSET).map(|e| r.tiff.floats(e)).transpose()?.and_then(|v| v.first().copied()).unwrap_or(0.0),
    };
    if calibration.color_matrix[0].is_none() {
        return Err(Error::Malformed("camera profile without ColorMatrix1"));
    }
    Ok(Profile { camera_model: r.text(tags::UNIQUE_CAMERA_MODEL), name: r.text(tags::PROFILE_NAME), calibration })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::Value;

    /// A minimal little-endian profile: IFD 0 with the given entries (tag, type, count, bytes).
    fn profile(entries: &[(u16, Value)], big_endian: bool) -> Vec<u8> {
        crate::writer::write_profile(entries, big_endian)
    }

    const CM: [f64; 9] = [0.9, -0.3, -0.1, -0.4, 1.2, 0.2, -0.05, 0.15, 0.6];

    #[test]
    fn reads_the_matrix_part() {
        for be in [false, true] {
            let b = profile(
                &[
                    (tags::UNIQUE_CAMERA_MODEL, Value::Ascii("Panasonic DC-TEST".into())),
                    (tags::COLOR_MATRIX_1, Value::srationals(&CM)),
                    (tags::COLOR_MATRIX_2, Value::srationals(&CM)),
                    (tags::CALIBRATION_ILLUMINANT_1, Value::Short(vec![17])),
                    (tags::CALIBRATION_ILLUMINANT_2, Value::Short(vec![21])),
                    (tags::PROFILE_NAME, Value::Ascii("Adobe Standard".into())),
                    (tags::BASELINE_EXPOSURE_OFFSET, Value::srationals(&[0.25])),
                ],
                be,
            );
            assert!(sniff(&b));
            let p = parse(&b).unwrap();
            assert_eq!(p.camera_model.as_deref(), Some("Panasonic DC-TEST"));
            assert_eq!(p.name.as_deref(), Some("Adobe Standard"));
            let m = p.calibration.color_matrix[0].unwrap();
            assert!((m[1][1] - 1.2).abs() < 1e-6 && (m[0][1] + 0.3).abs() < 1e-6);
            assert_eq!(p.calibration.illuminant, [Some(17), Some(21)]);
            assert!((p.calibration.baseline_exposure - 0.25).abs() < 1e-6);
            assert!(p.calibration.as_shot_neutral.is_none());
        }
    }

    #[test]
    fn hostile_profiles_are_errors() {
        // no ColorMatrix1
        let b = profile(&[(tags::UNIQUE_CAMERA_MODEL, Value::Ascii("X".into()))], false);
        assert!(parse(&b).is_err());
        // a 2 × 2 matrix
        let b = profile(&[(tags::COLOR_MATRIX_1, Value::srationals(&[1.0, 0.0, 0.0, 1.0]))], false);
        assert!(parse(&b).is_err());
        // not a profile, truncated, garbage
        assert!(parse(b"II*\0\x08\0\0\0").is_err());
        assert!(parse(b"IIRC").is_err());
        assert!(parse(&[0x49, 0x49, 0x52, 0x43, 0xFF, 0xFF, 0xFF, 0x7F]).is_err());
        let mut b = profile(&[(tags::COLOR_MATRIX_1, Value::srationals(&CM))], false);
        b.truncate(b.len() - 20);
        assert!(parse(&b).is_err());
    }
}
