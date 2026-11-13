//! Session Description Protocol (IETF RFC 4566 / RFC 8866) as ST 2110 uses it: one session per
//! RTP stream or redundant pair (ST 2110-10:2022 §8), with typed helpers for the clock attributes
//! (`ts-refclk` RFC 7273, `mediaclk`) and the `fmtp` parameter list.
//!
//! The model keeps lines in order, so `parse(write(s)) == s` and `write(parse(t)) == t` for text
//! in canonical form (CRLF line ends, one space after each `;` in `fmtp`).

use crate::Error;

/// An attribute line, `a=name` or `a=name:value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub value: Option<String>,
}

impl Attribute {
    /// `a=name:value`.
    pub fn new(name: &str, value: impl Into<String>) -> Self {
        Attribute { name: name.to_string(), value: Some(value.into()) }
    }

    /// `a=name` (a property attribute such as `recvonly`).
    pub fn flag(name: &str) -> Self {
        Attribute { name: name.to_string(), value: None }
    }

    fn parse(s: &str) -> Self {
        match s.split_once(':') {
            Some((n, v)) => Attribute { name: n.to_string(), value: Some(v.to_string()) },
            None => Attribute { name: s.to_string(), value: None },
        }
    }
}

/// One media description (`m=` section).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    /// `video`, `audio`, …
    pub media: String,
    pub port: u16,
    /// `RTP/AVP` in ST 2110.
    pub proto: String,
    /// Payload type numbers (as text, per RFC 4566).
    pub formats: Vec<String>,
    /// `i=` media title.
    pub info: Option<String>,
    /// `c=` lines.
    pub connections: Vec<String>,
    /// `b=` lines (`AS:216000`).
    pub bandwidths: Vec<String>,
    pub attributes: Vec<Attribute>,
}

impl Media {
    /// The first value of attribute `name`.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|a| a.name == name).and_then(|a| a.value.as_deref())
    }

    /// The `fmtp` parameters for payload type `pt` (§7.1 of ST 2110-20: `name=value` or `name`,
    /// separated by `;`).
    pub fn fmtp(&self, pt: u8) -> Option<FmtpParams> {
        let prefix = format!("{pt} ");
        self.attributes.iter().filter(|a| a.name == "fmtp").filter_map(|a| a.value.as_deref()).find_map(|v| v.strip_prefix(&prefix)).map(FmtpParams::parse)
    }

    /// The `rtpmap` of payload type `pt`: encoding name, clock rate and optional channel count.
    pub fn rtpmap(&self, pt: u8) -> Result<RtpMap, Error> {
        let prefix = format!("{pt} ");
        let v = self
            .attributes
            .iter()
            .filter(|a| a.name == "rtpmap")
            .filter_map(|a| a.value.as_deref())
            .find_map(|v| v.strip_prefix(&prefix))
            .ok_or(Error::Missing("a=rtpmap"))?;
        RtpMap::parse(v)
    }

    /// The timestamp reference clock (`a=ts-refclk`, required by ST 2110-10).
    pub fn ts_refclk(&self) -> Result<RefClock, Error> {
        RefClock::parse(self.attribute("ts-refclk").ok_or(Error::Missing("a=ts-refclk"))?)
    }

    /// The media clock (`a=mediaclk`, required by ST 2110-10).
    pub fn mediaclk(&self) -> Result<MediaClock, Error> {
        MediaClock::parse(self.attribute("mediaclk").ok_or(Error::Missing("a=mediaclk"))?)
    }
}

/// A session description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// `o=` (originator and session id), kept verbatim.
    pub origin: String,
    /// `s=`.
    pub name: String,
    /// `i=` session information.
    pub info: Option<String>,
    /// `c=` at session level.
    pub connections: Vec<String>,
    /// `t=`, normally `0 0`.
    pub timing: String,
    pub attributes: Vec<Attribute>,
    pub media: Vec<Media>,
}

impl Session {
    /// Parse SDP text. Accepts LF or CRLF line ends; unknown line types are an error (ST 2110
    /// receivers act on the SDP, so silently dropping a line could change its meaning).
    pub fn parse(text: &str) -> Result<Self, Error> {
        let mut lines = text.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty());
        if lines.next() != Some("v=0") {
            return Err(Error::Invalid("SDP must start with v=0"));
        }
        let mut s = Session {
            origin: String::new(),
            name: String::new(),
            info: None,
            connections: Vec::new(),
            timing: String::new(),
            attributes: Vec::new(),
            media: Vec::new(),
        };
        for line in lines {
            let (kind, value) = line.split_once('=').ok_or(Error::Invalid("SDP line without '='"))?;
            if kind == "m" {
                let mut f = value.split(' ');
                let media = f.next().unwrap_or_default().to_string();
                let port = f.next().and_then(|p| p.split('/').next()).and_then(|p| p.parse().ok()).ok_or(Error::Invalid("m= port"))?;
                let proto = f.next().ok_or(Error::Invalid("m= proto"))?.to_string();
                let formats: Vec<String> = f.map(str::to_string).collect();
                s.media.push(Media { media, port, proto, formats, info: None, connections: Vec::new(), bandwidths: Vec::new(), attributes: Vec::new() });
                continue;
            }
            match (s.media.last_mut(), kind) {
                (None, "o") => s.origin = value.to_string(),
                (None, "s") => s.name = value.to_string(),
                (None, "i") => s.info = Some(value.to_string()),
                (None, "c") => s.connections.push(value.to_string()),
                (None, "t") => s.timing = value.to_string(),
                (None, "a") => s.attributes.push(Attribute::parse(value)),
                (Some(m), "i") => m.info = Some(value.to_string()),
                (Some(m), "c") => m.connections.push(value.to_string()),
                (Some(m), "b") => m.bandwidths.push(value.to_string()),
                (Some(m), "a") => m.attributes.push(Attribute::parse(value)),
                _ => return Err(Error::Unsupported(format!("SDP line {kind}="))),
            }
        }
        if s.origin.is_empty() || s.timing.is_empty() {
            return Err(Error::Missing("SDP o= or t="));
        }
        Ok(s)
    }

    /// Write canonical SDP text (CRLF line ends).
    pub fn write(&self) -> String {
        let mut out = String::new();
        let mut line = |k: &str, v: &str| {
            out.push_str(k);
            out.push('=');
            out.push_str(v);
            out.push_str("\r\n");
        };
        line("v", "0");
        line("o", &self.origin);
        line("s", &self.name);
        if let Some(i) = &self.info {
            line("i", i);
        }
        for c in &self.connections {
            line("c", c);
        }
        line("t", &self.timing);
        for a in &self.attributes {
            line("a", &attr_text(a));
        }
        for m in &self.media {
            line("m", &format!("{} {} {} {}", m.media, m.port, m.proto, m.formats.join(" ")));
            if let Some(i) = &m.info {
                line("i", i);
            }
            for c in &m.connections {
                line("c", c);
            }
            for b in &m.bandwidths {
                line("b", b);
            }
            for a in &m.attributes {
                line("a", &attr_text(a));
            }
        }
        out
    }
}

fn attr_text(a: &Attribute) -> String {
    match &a.value {
        Some(v) => format!("{}:{}", a.name, v),
        None => a.name.clone(),
    }
}

/// `a=rtpmap:<pt> <encoding>/<clock>[/<channels>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpMap {
    pub encoding: String,
    pub clock_rate: u32,
    pub channels: Option<u32>,
}

impl RtpMap {
    fn parse(v: &str) -> Result<Self, Error> {
        let mut f = v.trim().split('/');
        let encoding = f.next().filter(|e| !e.is_empty()).ok_or(Error::Invalid("rtpmap encoding"))?.to_string();
        let clock_rate = f.next().and_then(|c| c.parse().ok()).ok_or(Error::Invalid("rtpmap clock rate"))?;
        let channels = match f.next() {
            Some(c) => Some(c.parse().map_err(|_| Error::Invalid("rtpmap channels"))?),
            None => None,
        };
        Ok(RtpMap { encoding, clock_rate, channels })
    }

    /// The attribute value for payload type `pt`.
    pub fn to_value(&self, pt: u8) -> String {
        match self.channels {
            Some(c) => format!("{pt} {}/{}/{c}", self.encoding, self.clock_rate),
            None => format!("{pt} {}/{}", self.encoding, self.clock_rate),
        }
    }
}

/// An ordered `fmtp` parameter list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FmtpParams(pub Vec<(String, Option<String>)>);

impl FmtpParams {
    /// Parse `a; b=c; d=e` (whitespace after `;` optional, as ST 2110-22 allows).
    pub fn parse(s: &str) -> Self {
        FmtpParams(
            s.split(';')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(|p| match p.split_once('=') {
                    Some((n, v)) => (n.to_string(), Some(v.to_string())),
                    None => (p.to_string(), None),
                })
                .collect(),
        )
    }

    /// The value of `name` (`Some("")` is never returned for a standalone name: use [`has`]).
    ///
    /// [`has`]: FmtpParams::has
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(n, _)| n == name).and_then(|(_, v)| v.as_deref())
    }

    /// True when `name` is present, with or without a value.
    pub fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(n, _)| n == name)
    }

    /// Append `name=value`.
    pub fn push(&mut self, name: &str, value: impl Into<String>) {
        self.0.push((name.to_string(), Some(value.into())));
    }

    /// Append a standalone `name`.
    pub fn push_flag(&mut self, name: &str) {
        self.0.push((name.to_string(), None));
    }

    /// The canonical text: entries separated by `"; "`, no trailing semicolon.
    pub fn to_text(&self) -> String {
        self.0.iter().map(|(n, v)| v.as_ref().map_or_else(|| n.clone(), |v| format!("{n}={v}"))).collect::<Vec<_>>().join("; ")
    }
}

/// The timestamp reference clock (`a=ts-refclk`, ST 2110-10:2022 §8.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefClock {
    /// IEEE 1588-2008 PTP with the grandmaster's EUI-64 clock identity and the domain number.
    Ptp { grandmaster: [u8; 8], domain: u8 },
    /// IEEE 1588-2008 PTP, traceable (the grandmaster is not named).
    PtpTraceable,
    /// No common reference: the sender's own timebase, named by its Ethernet MAC.
    LocalMac([u8; 6]),
}

impl RefClock {
    /// Parse the attribute value. The 2017 traceable form (`ptp=traceable`) is accepted, as the
    /// 2022 edition recommends receivers do.
    pub fn parse(v: &str) -> Result<Self, Error> {
        let bad = || Error::Unsupported(format!("ts-refclk:{v}"));
        if let Some(mac) = v.strip_prefix("localmac=") {
            return Ok(RefClock::LocalMac(hex_bytes(mac).ok_or_else(bad)?));
        }
        let rest = v.strip_prefix("ptp=").ok_or_else(bad)?;
        if rest == "traceable" || rest == "IEEE1588-2008:traceable" {
            return Ok(RefClock::PtpTraceable);
        }
        let rest = rest.strip_prefix("IEEE1588-2008:").ok_or_else(bad)?;
        let (gm, domain) = rest.split_once(':').ok_or_else(bad)?;
        Ok(RefClock::Ptp { grandmaster: hex_bytes(gm).ok_or_else(bad)?, domain: domain.parse().map_err(|_| bad())? })
    }

    /// The attribute value in the 2022 syntax.
    pub fn to_value(&self) -> String {
        match self {
            RefClock::Ptp { grandmaster, domain } => format!("ptp=IEEE1588-2008:{}:{domain}", hex_text(grandmaster)),
            RefClock::PtpTraceable => "ptp=IEEE1588-2008:traceable".to_string(),
            RefClock::LocalMac(mac) => format!("localmac={}", hex_text(mac)),
        }
    }
}

fn hex_bytes<const N: usize>(s: &str) -> Option<[u8; N]> {
    let mut out = [0u8; N];
    let mut parts = s.split('-');
    for b in &mut out {
        let p = parts.next()?;
        if p.len() != 2 {
            return None;
        }
        *b = u8::from_str_radix(p, 16).ok()?;
    }
    parts.next().is_none().then_some(out)
}

fn hex_text(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join("-")
}

/// The media clock (`a=mediaclk`, ST 2110-10:2022 §8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaClock {
    /// Derived directly from the reference clock with this offset (ST 2110 requires 0).
    Direct(u32),
    /// Asynchronous to the reference clock.
    Sender,
}

impl MediaClock {
    pub fn parse(v: &str) -> Result<Self, Error> {
        if v == "sender" {
            return Ok(MediaClock::Sender);
        }
        v.strip_prefix("direct=").and_then(|o| o.parse().ok()).map(MediaClock::Direct).ok_or_else(|| Error::Unsupported(format!("mediaclk:{v}")))
    }

    pub fn to_value(self) -> String {
        match self {
            MediaClock::Direct(o) => format!("direct={o}"),
            MediaClock::Sender => "sender".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Built for these tests following ST 2110-10:2022 §8 (not copied from the standard).
    const PAIR: &str = "v=0\r\n\
o=- 4242 7 IN IP4 10.20.0.5\r\n\
s=CFA4 program\r\n\
i=1080p50 program out\r\n\
t=0 0\r\n\
a=recvonly\r\n\
a=group:DUP red blue\r\n\
m=video 20000 RTP/AVP 98\r\n\
c=IN IP4 239.20.1.1/32\r\n\
a=source-filter: incl IN IP4 239.20.1.1 10.20.0.5\r\n\
a=rtpmap:98 raw/90000\r\n\
a=fmtp:98 sampling=YCbCr-4:2:2; width=1920; height=1080; exactframerate=50; depth=10; TCS=SDR; colorimetry=BT709; PM=2110GPM; SSN=ST2110-20:2017; TP=2110TPW; TSMODE=NEW; TSDELAY=40\r\n\
a=ts-refclk:ptp=IEEE1588-2008:00-1B-21-FF-FE-12-34-56:127\r\n\
a=mediaclk:direct=0\r\n\
a=mid:red\r\n\
m=video 20000 RTP/AVP 98\r\n\
c=IN IP4 239.21.1.1/32\r\n\
a=rtpmap:98 raw/90000\r\n\
a=ts-refclk:localmac=00-1B-21-12-34-57\r\n\
a=mediaclk:sender\r\n\
a=mid:blue\r\n";

    #[test]
    fn canonical_text_round_trips() {
        let s = Session::parse(PAIR).unwrap();
        assert_eq!(s.write(), PAIR);
        assert_eq!(Session::parse(&s.write()).unwrap(), s);
        assert_eq!(s.media.len(), 2);
        assert_eq!(s.attributes[1], Attribute::new("group", "DUP red blue"));
    }

    #[test]
    fn typed_accessors() {
        let s = Session::parse(PAIR).unwrap();
        let m = &s.media[0];
        assert_eq!(m.rtpmap(98).unwrap(), RtpMap { encoding: "raw".into(), clock_rate: 90000, channels: None });
        assert_eq!(m.ts_refclk().unwrap(), RefClock::Ptp { grandmaster: [0x00, 0x1B, 0x21, 0xFF, 0xFE, 0x12, 0x34, 0x56], domain: 127 });
        assert_eq!(m.mediaclk().unwrap(), MediaClock::Direct(0));
        let f = m.fmtp(98).unwrap();
        assert_eq!(f.get("width"), Some("1920"));
        assert_eq!(
            f.to_text(),
            "sampling=YCbCr-4:2:2; width=1920; height=1080; exactframerate=50; depth=10; TCS=SDR; colorimetry=BT709; PM=2110GPM; SSN=ST2110-20:2017; TP=2110TPW; TSMODE=NEW; TSDELAY=40"
        );
        assert_eq!(s.media[1].ts_refclk().unwrap(), RefClock::LocalMac([0x00, 0x1B, 0x21, 0x12, 0x34, 0x57]));
        assert_eq!(s.media[1].mediaclk().unwrap(), MediaClock::Sender);
        assert!(s.media[1].fmtp(98).is_none());
    }

    #[test]
    fn ref_clock_forms() {
        for v in ["ptp=IEEE1588-2008:traceable", "localmac=7C-00-11-22-33-44", "ptp=IEEE1588-2008:AA-BB-CC-DD-EE-FF-00-11:0"] {
            assert_eq!(RefClock::parse(v).unwrap().to_value(), v);
        }
        // 2017 traceable form, tolerated.
        assert_eq!(RefClock::parse("ptp=traceable").unwrap(), RefClock::PtpTraceable);
        assert!(RefClock::parse("ptp=IEEE1588-2008:AA-BB:1").is_err());
        assert!(RefClock::parse("ntp=pool.example").is_err());
    }

    #[test]
    fn fmtp_without_spaces_and_flags() {
        let f = FmtpParams::parse("width=1920;height=1080;interlace;segmented");
        assert!(f.has("interlace") && f.has("segmented"));
        assert_eq!(f.get("interlace"), None);
        assert_eq!(f.to_text(), "width=1920; height=1080; interlace; segmented");
    }

    #[test]
    fn audio_rtpmap_with_channels() {
        let s = Session::parse("v=0\no=- 1 1 IN IP4 10.0.0.1\ns=a\nt=0 0\nm=audio 5004 RTP/AVP 97\na=rtpmap:97 L24/48000/8\na=ptime:1\n").unwrap();
        let m = &s.media[0];
        let r = m.rtpmap(97).unwrap();
        assert_eq!((r.encoding.as_str(), r.clock_rate, r.channels), ("L24", 48000, Some(8)));
        assert_eq!(r.to_value(97), "97 L24/48000/8");
        assert_eq!(m.attribute("ptime"), Some("1"));
    }

    #[test]
    fn malformed_sdp_is_an_error() {
        assert!(Session::parse("").is_err());
        assert!(Session::parse("v=1\r\n").is_err());
        assert!(Session::parse("v=0\r\nx\r\n").is_err());
        assert!(Session::parse("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=x\r\nt=0 0\r\nm=video abc RTP/AVP 96\r\n").is_err());
        assert!(Session::parse("v=0\r\ns=x\r\n").is_err());
        assert!(Session::parse("v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=x\r\nt=0 0\r\nz=0\r\n").is_err());
    }
}
