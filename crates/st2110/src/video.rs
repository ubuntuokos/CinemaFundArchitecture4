//! ST 2110-20:2022 video description: sampling, depth, colorimetry, transfer characteristic,
//! packing mode, and the pgroup table of §6.2 (Tables 1–4).

use crate::Error;

/// Colour-difference sampling structure (`sampling`, §7.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sampling {
    YCbCr444,
    YCbCr422,
    YCbCr420,
    ClYCbCr444,
    ClYCbCr422,
    ClYCbCr420,
    ICtCp444,
    ICtCp422,
    ICtCp420,
    Rgb,
    Xyz,
    /// Key (alpha) signal, one component (SMPTE RP 157).
    Key,
}

impl Sampling {
    const ALL: [(Sampling, &'static str); 12] = [
        (Sampling::YCbCr444, "YCbCr-4:4:4"),
        (Sampling::YCbCr422, "YCbCr-4:2:2"),
        (Sampling::YCbCr420, "YCbCr-4:2:0"),
        (Sampling::ClYCbCr444, "CLYCbCr-4:4:4"),
        (Sampling::ClYCbCr422, "CLYCbCr-4:2:2"),
        (Sampling::ClYCbCr420, "CLYCbCr-4:2:0"),
        (Sampling::ICtCp444, "ICtCp-4:4:4"),
        (Sampling::ICtCp422, "ICtCp-4:2:2"),
        (Sampling::ICtCp420, "ICtCp-4:2:0"),
        (Sampling::Rgb, "RGB"),
        (Sampling::Xyz, "XYZ"),
        (Sampling::Key, "KEY"),
    ];

    /// The SDP token.
    pub fn as_str(self) -> &'static str {
        Self::ALL.iter().find(|(s, _)| *s == self).map(|(_, n)| *n).unwrap_or("")
    }

    /// Parse the SDP token (case-sensitive, as in the standard).
    pub fn parse(s: &str) -> Result<Self, Error> {
        Self::ALL.iter().find(|(_, n)| *n == s).map(|(v, _)| *v).ok_or_else(|| Error::Unsupported(format!("sampling={s}")))
    }

    /// True for 4:2:0 structures, whose pgroups span two sample rows.
    pub fn is_420(self) -> bool {
        matches!(self, Sampling::YCbCr420 | Sampling::ClYCbCr420 | Sampling::ICtCp420)
    }

    fn is_422(self) -> bool {
        matches!(self, Sampling::YCbCr422 | Sampling::ClYCbCr422 | Sampling::ICtCp422)
    }

    fn is_444(self) -> bool {
        matches!(self, Sampling::YCbCr444 | Sampling::ClYCbCr444 | Sampling::ICtCp444 | Sampling::Rgb)
    }
}

/// Bits per sample (`depth`, §7.4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Depth {
    D8,
    D10,
    D12,
    D16,
    /// 16-bit floating point.
    D16f,
}

impl Depth {
    /// The SDP token.
    pub fn as_str(self) -> &'static str {
        match self {
            Depth::D8 => "8",
            Depth::D10 => "10",
            Depth::D12 => "12",
            Depth::D16 => "16",
            Depth::D16f => "16f",
        }
    }

    /// Parse the SDP token.
    pub fn parse(s: &str) -> Result<Self, Error> {
        Ok(match s {
            "8" => Depth::D8,
            "10" => Depth::D10,
            "12" => Depth::D12,
            "16" => Depth::D16,
            "16f" => Depth::D16f,
            _ => return Err(Error::Unsupported(format!("depth={s}"))),
        })
    }

    /// Bits per sample in the payload.
    pub fn bits(self) -> u32 {
        match self {
            Depth::D8 => 8,
            Depth::D10 => 10,
            Depth::D12 => 12,
            Depth::D16 | Depth::D16f => 16,
        }
    }
}

/// The pgroup of a sampling/depth pair (§6.2.3–6.2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PGroup {
    /// Size in octets.
    pub octets: u32,
    /// Horizontal pixels covered (4:2:0 pgroups also cover the next sample row).
    pub pixels: u32,
}

/// The pgroup for `sampling` at `depth`, or an error for combinations the standard does not
/// define (XYZ below 12 bits, 4:2:0 at 16 bits).
pub fn pgroup(sampling: Sampling, depth: Depth) -> Result<PGroup, Error> {
    use Depth::*;
    let (octets, pixels) = if sampling.is_444() {
        match depth {
            D8 => (3, 1),
            D10 => (15, 4),
            D12 => (9, 2),
            D16 | D16f => (6, 1),
        }
    } else if sampling == Sampling::Xyz {
        match depth {
            D12 => (9, 2),
            D16 | D16f => (6, 1),
            _ => return Err(Error::Unsupported(format!("XYZ at depth {}", depth.as_str()))),
        }
    } else if sampling.is_422() {
        match depth {
            D8 => (4, 2),
            D10 => (5, 2),
            D12 => (6, 2),
            D16 | D16f => (8, 2),
        }
    } else if sampling.is_420() {
        // Table 3 gives the coverage as 2×2 (8, 12 bit) or 4×2 (10 bit) pixels.
        match depth {
            D8 => (6, 2),
            D10 => (15, 4),
            D12 => (9, 2),
            _ => return Err(Error::Unsupported(format!("{} at depth {}", sampling.as_str(), depth.as_str()))),
        }
    } else {
        match depth {
            D8 => (1, 1),
            D10 => (5, 4),
            D12 => (3, 2),
            D16 | D16f => (2, 1),
        }
    };
    Ok(PGroup { octets, pixels })
}

macro_rules! token_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident => $tok:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($var),+ }
        impl $name {
            /// The SDP token.
            pub fn as_str(self) -> &'static str { match self { $($name::$var => $tok),+ } }
            /// Parse the SDP token.
            pub fn parse(s: &str) -> Result<Self, Error> {
                match s { $($tok => Ok($name::$var),)+ _ => Err(Error::Unsupported(format!(concat!(stringify!($name), " {}"), s))) }
            }
        }
    };
}

token_enum!(
    /// System colorimetry (`colorimetry`, §7.5).
    Colorimetry {
        Bt601 => "BT601", Bt709 => "BT709", Bt2020 => "BT2020", Bt2100 => "BT2100",
        St2065_1 => "ST2065-1", St2065_3 => "ST2065-3", Unspecified => "UNSPECIFIED", Xyz => "XYZ", Alpha => "ALPHA",
    }
);

token_enum!(
    /// Transfer characteristic system (`TCS`, §7.6). Absent means SDR.
    Tcs {
        Sdr => "SDR", Pq => "PQ", Hlg => "HLG", Linear => "LINEAR", Bt2100LinPq => "BT2100LINPQ",
        Bt2100LinHlg => "BT2100LINHLG", St2065_1 => "ST2065-1", St428_1 => "ST428-1", Density => "DENSITY",
        St2115LogS3 => "ST2115LOGS3", Unspecified => "UNSPECIFIED",
    }
);

token_enum!(
    /// Encoding range (`RANGE`, §7.3). Absent means NARROW.
    Range { Narrow => "NARROW", FullProtect => "FULLPROTECT", Full => "FULL" }
);

token_enum!(
    /// Packing mode (`PM`, §6.3).
    PackingMode { General => "2110GPM", Block => "2110BPM" }
);

token_enum!(
    /// ST 2110-21 sender type (`TP`, §7.1): narrow, narrow linear, wide.
    SenderType { Narrow => "2110TPN", NarrowLinear => "2110TPNL", Wide => "2110TPW" }
);

token_enum!(
    /// ST 2110-10:2022 timestamp mode (`TSMODE`, §8.7). Absent means NEW.
    TsMode { Samp => "SAMP", New => "NEW", Pres => "PRES" }
);

/// An exact frame rate, `exactframerate` (§7.2): an integer, or a reduced ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    /// A rate `num/den`, reduced to the smallest numerator (the form the standard requires).
    pub fn new(num: u32, den: u32) -> Result<Self, Error> {
        if num == 0 || den == 0 {
            return Err(Error::Invalid("frame rate must be positive"));
        }
        let g = gcd(num, den);
        Ok(FrameRate { num: num / g, den: den / g })
    }

    /// Parse `25` or `30000/1001`; an unreduced ratio is accepted and reduced.
    pub fn parse(s: &str) -> Result<Self, Error> {
        let bad = || Error::Unsupported(format!("exactframerate={s}"));
        match s.split_once('/') {
            Some((n, d)) => Self::new(n.parse().map_err(|_| bad())?, d.parse().map_err(|_| bad())?),
            None => Self::new(s.parse().map_err(|_| bad())?, 1),
        }
    }

    /// The SDP token.
    pub fn to_token(self) -> String {
        if self.den == 1 { self.num.to_string() } else { format!("{}/{}", self.num, self.den) }
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Bytes of one transmitted sample row: `width` pixels rounded up to whole pgroups (the last
/// pgroup is zero-filled, §6.2.1). For 4:2:0 a transmitted row carries two image rows.
pub fn row_bytes(width: u32, pg: PGroup) -> usize {
    width.div_ceil(pg.pixels) as usize * pg.octets as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pgroup_table() {
        let p = |s, d| pgroup(s, d).map(|g| (g.octets, g.pixels));
        assert_eq!(p(Sampling::YCbCr422, Depth::D10).unwrap(), (5, 2));
        assert_eq!(p(Sampling::YCbCr422, Depth::D8).unwrap(), (4, 2));
        assert_eq!(p(Sampling::Rgb, Depth::D10).unwrap(), (15, 4));
        assert_eq!(p(Sampling::Rgb, Depth::D16f).unwrap(), (6, 1));
        assert_eq!(p(Sampling::YCbCr420, Depth::D10).unwrap(), (15, 4));
        assert_eq!(p(Sampling::Key, Depth::D10).unwrap(), (5, 4));
        assert_eq!(p(Sampling::Key, Depth::D12).unwrap(), (3, 2));
        assert!(p(Sampling::Xyz, Depth::D8).is_err());
        assert!(p(Sampling::YCbCr420, Depth::D16).is_err());
    }

    #[test]
    fn tokens_round_trip() {
        for (s, n) in Sampling::ALL {
            assert_eq!(Sampling::parse(n).unwrap(), s);
            assert_eq!(s.as_str(), n);
        }
        assert_eq!(Tcs::parse("ST2115LOGS3").unwrap(), Tcs::St2115LogS3);
        assert_eq!(Colorimetry::Alpha.as_str(), "ALPHA");
        assert!(PackingMode::parse("2110XPM").is_err());
        assert_eq!(Depth::parse("16f").unwrap().bits(), 16);
    }

    #[test]
    fn frame_rates_are_reduced() {
        assert_eq!(FrameRate::parse("60000/1001").unwrap().to_token(), "60000/1001");
        assert_eq!(FrameRate::parse("50/2").unwrap().to_token(), "25");
        assert_eq!(FrameRate::parse("120000/2002").unwrap().to_token(), "60000/1001");
        assert!(FrameRate::parse("0").is_err());
        assert!(FrameRate::parse("x/1").is_err());
    }

    #[test]
    fn row_bytes_rounds_up_to_pgroups() {
        let pg = pgroup(Sampling::YCbCr422, Depth::D10).unwrap();
        assert_eq!(row_bytes(1920, pg), 4800);
        assert_eq!(row_bytes(1921, pg), 4805);
    }
}
