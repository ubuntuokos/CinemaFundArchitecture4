//! The `fmtp` parameters of an ST 2110-20 video stream, with the ST 2110-21 (`TP`, `TROFF`,
//! `CMAX`) and ST 2110-10:2022 (`MAXUDP`, `TSMODE`, `TSDELAY`) parameters, as one typed value.

use crate::sdp::FmtpParams;
use crate::video::{Colorimetry, Depth, FrameRate, PGroup, PackingMode, Range, Sampling, SenderType, Tcs, TsMode, pgroup};
use crate::{Error, STANDARD_UDP_LIMIT};

/// Everything a receiver needs to interpret a -20 stream, as signalled in `a=fmtp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFormat {
    pub sampling: Sampling,
    pub depth: Depth,
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
    pub colorimetry: Colorimetry,
    pub packing: PackingMode,
    /// Interlaced (or PsF when `segmented`).
    pub interlace: bool,
    pub segmented: bool,
    /// `None`: not signalled (receivers assume SDR).
    pub tcs: Option<Tcs>,
    /// `None`: not signalled (receivers assume NARROW).
    pub range: Option<Range>,
    /// `None`: Standard UDP Size Limit.
    pub max_udp: Option<u32>,
    /// Pixel aspect ratio; `None` means 1:1.
    pub par: Option<(u32, u32)>,
    /// ST 2110-21 sender type.
    pub sender_type: SenderType,
    /// ST 2110-21 `TROFF` in microseconds, when not the default.
    pub troff: Option<u32>,
    /// ST 2110-21 `CMAX`, when signalled.
    pub cmax: Option<u32>,
    /// ST 2110-10:2022 `TSMODE`.
    pub ts_mode: Option<TsMode>,
    /// ST 2110-10:2022 `TSDELAY` in microseconds.
    pub ts_delay: Option<u32>,
}

impl VideoFormat {
    /// A progressive format with the defaults a software sender uses (GPM, type W, no optional
    /// parameters).
    pub fn progressive(sampling: Sampling, depth: Depth, width: u32, height: u32, rate: FrameRate, colorimetry: Colorimetry) -> Self {
        VideoFormat {
            sampling,
            depth,
            width,
            height,
            rate,
            colorimetry,
            packing: PackingMode::General,
            interlace: false,
            segmented: false,
            tcs: None,
            range: None,
            max_udp: None,
            par: None,
            sender_type: SenderType::Wide,
            troff: None,
            cmax: None,
            ts_mode: None,
            ts_delay: None,
        }
    }

    /// Check the combinations the standards forbid; returns the pgroup.
    pub fn validate(&self) -> Result<PGroup, Error> {
        let pg = pgroup(self.sampling, self.depth)?;
        if !(1..=32767).contains(&self.width) || !(1..=32767).contains(&self.height) {
            return Err(Error::Invalid("width and height must be 1..=32767"));
        }
        if self.segmented && !self.interlace {
            return Err(Error::Invalid("segmented requires interlace"));
        }
        if self.sampling.is_420() && self.interlace {
            return Err(Error::Invalid("4:2:0 is progressive only"));
        }
        if self.sampling.is_420() && !self.height.is_multiple_of(2) {
            return Err(Error::Invalid("4:2:0 needs an even height"));
        }
        if (self.sampling == Sampling::Key) != (self.colorimetry == Colorimetry::Alpha) {
            return Err(Error::Invalid("KEY sampling goes with colorimetry ALPHA, and only with it"));
        }
        if self.sampling == Sampling::Key && self.tcs.is_some() {
            return Err(Error::Invalid("a KEY stream does not signal TCS"));
        }
        if let Some(m) = self.max_udp {
            if m <= STANDARD_UDP_LIMIT as u32 || m > crate::EXTENDED_UDP_LIMIT as u32 {
                return Err(Error::Invalid("MAXUDP must be above the standard limit and at most 8960"));
            }
            if self.packing == PackingMode::Block {
                return Err(Error::Invalid("Block Packing Mode does not use the extended UDP limit"));
            }
        }
        Ok(pg)
    }

    /// The `SSN` value: `ST2110-20:2017`, unless a 2022-only feature (ALPHA, LOGS3) is used.
    pub fn ssn(&self) -> &'static str {
        if self.colorimetry == Colorimetry::Alpha || self.tcs == Some(Tcs::St2115LogS3) { "ST2110-20:2022" } else { "ST2110-20:2017" }
    }

    /// The `fmtp` parameters in the order of ST 2110-20 §7.2, §7.3, then -21 and -10.
    pub fn to_fmtp(&self) -> FmtpParams {
        let mut f = FmtpParams::default();
        f.push("sampling", self.sampling.as_str());
        f.push("width", self.width.to_string());
        f.push("height", self.height.to_string());
        f.push("exactframerate", self.rate.to_token());
        f.push("depth", self.depth.as_str());
        if let Some(t) = self.tcs {
            f.push("TCS", t.as_str());
        }
        f.push("colorimetry", self.colorimetry.as_str());
        f.push("PM", self.packing.as_str());
        f.push("SSN", self.ssn());
        if self.interlace {
            f.push_flag("interlace");
        }
        if self.segmented {
            f.push_flag("segmented");
        }
        if let Some(r) = self.range {
            f.push("RANGE", r.as_str());
        }
        if let Some(m) = self.max_udp {
            f.push("MAXUDP", m.to_string());
        }
        if let Some((w, h)) = self.par {
            f.push("PAR", format!("{w}:{h}"));
        }
        f.push("TP", self.sender_type.as_str());
        if let Some(t) = self.troff {
            f.push("TROFF", t.to_string());
        }
        if let Some(c) = self.cmax {
            f.push("CMAX", c.to_string());
        }
        if let Some(m) = self.ts_mode {
            f.push("TSMODE", m.as_str());
        }
        if let Some(d) = self.ts_delay {
            f.push("TSDELAY", d.to_string());
        }
        f
    }

    /// Read a -20 stream's parameters. Required parameters missing are an error; unknown ones
    /// are ignored (codec-specific or newer parameters).
    pub fn from_fmtp(f: &FmtpParams) -> Result<Self, Error> {
        let req = |n: &'static str| f.get(n).ok_or(Error::Missing(n));
        let num = |n: &'static str, v: &str| v.parse::<u32>().map_err(|_| Error::Unsupported(format!("{n}={v}")));
        let opt_num = |n: &'static str| f.get(n).map(|v| num(n, v)).transpose();
        let par = match f.get("PAR") {
            Some(v) => {
                let (w, h) = v.split_once(':').ok_or_else(|| Error::Unsupported(format!("PAR={v}")))?;
                Some((num("PAR", w)?, num("PAR", h)?))
            }
            None => None,
        };
        let v = VideoFormat {
            sampling: Sampling::parse(req("sampling")?)?,
            depth: Depth::parse(req("depth")?)?,
            width: num("width", req("width")?)?,
            height: num("height", req("height")?)?,
            rate: FrameRate::parse(req("exactframerate")?)?,
            colorimetry: Colorimetry::parse(req("colorimetry")?)?,
            packing: PackingMode::parse(req("PM")?)?,
            interlace: f.has("interlace"),
            segmented: f.has("segmented"),
            tcs: f.get("TCS").map(Tcs::parse).transpose()?,
            range: f.get("RANGE").map(Range::parse).transpose()?,
            max_udp: opt_num("MAXUDP")?,
            par,
            sender_type: SenderType::parse(req("TP")?)?,
            troff: opt_num("TROFF")?,
            cmax: opt_num("CMAX")?,
            ts_mode: f.get("TSMODE").map(TsMode::parse).transpose()?,
            ts_delay: opt_num("TSDELAY")?,
        };
        v.validate()?;
        Ok(v)
    }

    /// The largest UDP datagram this stream may use.
    pub fn udp_limit(&self) -> usize {
        self.max_udp.map_or(STANDARD_UDP_LIMIT, |m| m as usize)
    }

    /// Transmitted sample rows per frame or field: 4:2:0 sends row pairs; interlaced and PsF
    /// formats send each field separately, the first field taking the extra row of an odd height
    /// (ST 2110-20 §6.1.5).
    pub fn rows_per_image(&self, second_field: bool) -> u32 {
        if self.sampling.is_420() {
            self.height / 2
        } else if self.interlace {
            if second_field { self.height / 2 } else { self.height.div_ceil(2) }
        } else {
            self.height
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hd() -> VideoFormat {
        VideoFormat::progressive(Sampling::YCbCr422, Depth::D10, 1920, 1080, FrameRate::new(50, 1).unwrap(), Colorimetry::Bt709)
    }

    #[test]
    fn fmtp_round_trip() {
        let mut v = hd();
        v.tcs = Some(Tcs::Sdr);
        v.ts_mode = Some(TsMode::New);
        v.ts_delay = Some(40);
        v.par = Some((1, 1));
        let text = v.to_fmtp().to_text();
        assert_eq!(
            text,
            "sampling=YCbCr-4:2:2; width=1920; height=1080; exactframerate=50; depth=10; TCS=SDR; colorimetry=BT709; PM=2110GPM; SSN=ST2110-20:2017; PAR=1:1; TP=2110TPW; TSMODE=NEW; TSDELAY=40"
        );
        assert_eq!(VideoFormat::from_fmtp(&FmtpParams::parse(&text)).unwrap(), v);
    }

    #[test]
    fn key_signals_use_the_2022_ssn() {
        let mut k = hd();
        k.sampling = Sampling::Key;
        k.colorimetry = Colorimetry::Alpha;
        assert_eq!(k.ssn(), "ST2110-20:2022");
        assert!(k.validate().is_ok());
        k.tcs = Some(Tcs::Sdr);
        assert!(k.validate().is_err());
    }

    #[test]
    fn invalid_combinations() {
        let mut v = hd();
        v.segmented = true;
        assert!(v.validate().is_err());
        let mut v = hd();
        v.max_udp = Some(1000);
        assert!(v.validate().is_err());
        let mut v = hd();
        v.max_udp = Some(8960);
        v.packing = PackingMode::Block;
        assert!(v.validate().is_err());
        let mut v = hd();
        v.sampling = Sampling::YCbCr420;
        v.interlace = true;
        assert!(v.validate().is_err());
        assert!(VideoFormat::from_fmtp(&FmtpParams::parse("sampling=YCbCr-4:2:2; width=1920")).is_err());
    }

    #[test]
    fn rows_per_field() {
        let mut v = hd();
        assert_eq!(v.rows_per_image(false), 1080);
        v.interlace = true;
        v.height = 487;
        assert_eq!((v.rows_per_image(false), v.rows_per_image(true)), (244, 243));
    }
}
