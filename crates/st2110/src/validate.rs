//! The network rules of ST 2110-10:2022 as a checker over a parsed SDP (plan phase T1b, V-07 and
//! V-10). A receiver acts on the SDP it is given, so this reports every rule the description
//! breaks, each with the media section it concerns, and the host refuses the stream instead of
//! guessing.
//!
//! Rules checked:
//! - `N1` no `jxsv` (JPEG XS) `rtpmap` anywhere (plan D10).
//! - `N2` an IPv4 multicast destination is not in 224.0.0.0/24 or 224.0.1.0/24 (§6.2, §8.4).
//! - `N3` one payload type per `m=` section: no RTP session multiplexing (§6.5).
//! - `N4` `MAXUDP`, when given, is a number from 1461 to 8960 (§6.2; the standard limit is 1460).
//! - `N5` `ts-refclk` and `mediaclk` are present (media or session level) and readable; with a PTP
//!   reference the media clock is `direct=0` (§8.2, §8.3).
//! - `N6` an ST 2022-7 pair (`a=group:DUP`) names two existing media sections that agree on
//!   everything the receiver merges on, and the two flows do not share both source and
//!   destination address (§8.5).
//!
//! What stays outside: no IP fragmentation is a property of the sender's packets (the packetizer
//! keeps every datagram inside the UDP limit); RTCP and RTP header extensions are tolerated by
//! the RTP parser (`rtp`), nothing in the SDP describes them.

use crate::sdp::{Media, MediaClock, RefClock, RtpMap, Session};
use crate::{EXTENDED_UDP_LIMIT, Error, STANDARD_UDP_LIMIT};
use std::net::IpAddr;

/// One broken rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The rule, `N1` to `N6` (module documentation).
    pub rule: &'static str,
    /// The 0-based index of the `m=` section, or `None` for the session.
    pub media: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.media {
            Some(i) => write!(f, "{} (media {i}): {}", self.rule, self.message),
            None => write!(f, "{}: {}", self.rule, self.message),
        }
    }
}

/// Check `session` and return the first broken rule as an error.
pub fn validate(session: &Session) -> Result<(), Error> {
    match check(session).into_iter().next() {
        Some(f) => Err(Error::Unsupported(f.to_string())),
        None => Ok(()),
    }
}

/// Every broken rule, in the order of the media sections.
pub fn check(session: &Session) -> Vec<Finding> {
    let mut out = Vec::new();
    for (i, m) in session.media.iter().enumerate() {
        check_media(session, i, m, &mut out);
    }
    check_pairs(session, &mut out);
    out
}

fn find(out: &mut Vec<Finding>, rule: &'static str, media: Option<usize>, message: impl Into<String>) {
    out.push(Finding { rule, media, message: message.into() });
}

fn check_media(session: &Session, i: usize, m: &Media, out: &mut Vec<Finding>) {
    // N1: every rtpmap, not only the one of the first format.
    for a in m.attributes.iter().filter(|a| a.name == "rtpmap") {
        if let Some(v) = a.value.as_deref().and_then(|v| v.split_once(' ').map(|(_, rest)| rest))
            && let Err(e) = RtpMap::parse(v)
            && v.trim().split('/').next().is_some_and(|e| e.eq_ignore_ascii_case("jxsv"))
        {
            find(out, "N1", Some(i), e.to_string());
        }
    }
    // N2: destinations.
    for c in session.connections.iter().chain(&m.connections) {
        if let Some(ip) = connection_address(c)
            && let Some(why) = reserved_multicast(ip)
        {
            find(out, "N2", Some(i), format!("destination {ip} is {why}"));
        }
    }
    // N3.
    if m.formats.len() > 1 {
        find(out, "N3", Some(i), format!("{} payload types in one m= section: RTP session multiplexing is not allowed", m.formats.len()));
    }
    // N4.
    for a in m.attributes.iter().filter(|a| a.name == "fmtp") {
        let Some(v) = a.value.as_deref() else { continue };
        let params = v.split_once(' ').map_or("", |(_, p)| p);
        let Some(max) = crate::sdp::FmtpParams::parse(params).get("MAXUDP").map(str::to_string) else { continue };
        match max.parse::<usize>() {
            Ok(n) if n > STANDARD_UDP_LIMIT && n <= EXTENDED_UDP_LIMIT => {}
            _ => find(out, "N4", Some(i), format!("MAXUDP={max} must be a number above {STANDARD_UDP_LIMIT} and at most {EXTENDED_UDP_LIMIT}")),
        }
    }
    // N5.
    let attr = |name: &str| m.attribute(name).or_else(|| session.attributes.iter().find(|a| a.name == name).and_then(|a| a.value.as_deref()));
    let refclk = match attr("ts-refclk") {
        None => {
            find(out, "N5", Some(i), "a=ts-refclk is missing");
            None
        }
        Some(v) => match RefClock::parse(v) {
            Ok(r) => Some(r),
            Err(e) => {
                find(out, "N5", Some(i), e.to_string());
                None
            }
        },
    };
    match attr("mediaclk").map(MediaClock::parse) {
        None => find(out, "N5", Some(i), "a=mediaclk is missing"),
        Some(Err(e)) => find(out, "N5", Some(i), e.to_string()),
        Some(Ok(MediaClock::Direct(o))) if o != 0 && matches!(refclk, Some(RefClock::Ptp { .. } | RefClock::PtpTraceable)) => {
            find(out, "N5", Some(i), format!("mediaclk direct={o}: ST 2110-10 requires direct=0 with a PTP reference"));
        }
        Some(Ok(_)) => {}
    }
}

/// The address of a `c=` line (`IN IP4 239.1.1.1/32`), without the TTL or count.
fn connection_address(c: &str) -> Option<IpAddr> {
    let addr = c.split_whitespace().nth(2)?.split('/').next()?;
    addr.parse().ok()
}

/// Why an address is not allowed as a destination, if it is not.
fn reserved_multicast(ip: IpAddr) -> Option<&'static str> {
    let IpAddr::V4(v4) = ip else { return None };
    let o = v4.octets();
    match (o[0], o[1], o[2]) {
        (224, 0, 0) => Some("in 224.0.0.0/24 (link-local control block, reserved)"),
        (224, 0, 1) => Some("in 224.0.1.0/24 (internetwork control block, reserved)"),
        _ => None,
    }
}

/// The source address of a media section from `a=source-filter: incl IN IP4 <dest> <source>`.
fn source_address(m: &Media) -> Option<IpAddr> {
    let v = m.attribute("source-filter")?;
    let mut f = v.split_whitespace();
    // incl IN IP4 <dest> <source...>
    f.nth(3)?;
    f.next()?.parse().ok()
}

fn destination(m: &Media) -> Option<IpAddr> {
    m.connections.first().and_then(|c| connection_address(c))
}

fn check_pairs(session: &Session, out: &mut Vec<Finding>) {
    for g in session.attributes.iter().filter(|a| a.name == "group") {
        let Some(v) = g.value.as_deref() else { continue };
        let mut f = v.split_whitespace();
        if f.next() != Some("DUP") {
            continue;
        }
        let mids: Vec<&str> = f.collect();
        if mids.len() != 2 {
            find(out, "N6", None, format!("a=group:DUP names {} streams; an ST 2022-7 pair has two", mids.len()));
            continue;
        }
        let idx: Vec<Option<usize>> = mids.iter().map(|mid| session.media.iter().position(|m| m.attribute("mid") == Some(mid))).collect();
        let (Some(a), Some(b)) = (idx[0], idx[1]) else {
            find(out, "N6", None, format!("a=group:DUP names {} and {}, one of them has no m= section with that a=mid", mids[0], mids[1]));
            continue;
        };
        if a == b {
            find(out, "N6", None, "a=group:DUP names the same stream twice");
            continue;
        }
        let (ma, mb) = (&session.media[a], &session.media[b]);
        // What the receiver merges on must be identical: payload description and clocks.
        let describe = |m: &Media| {
            let pick = |name: &str| m.attributes.iter().filter(|x| x.name == name).map(|x| x.value.clone().unwrap_or_default()).collect::<Vec<_>>();
            (m.media.clone(), m.formats.clone(), pick("rtpmap"), pick("fmtp"), pick("ptime"), pick("ts-refclk"), pick("mediaclk"))
        };
        if describe(ma) != describe(mb) {
            find(out, "N6", Some(b), format!("the two streams of the pair differ in payload, rtpmap, fmtp, ptime or clock attributes (media {a} and {b})"));
        }
        let (da, db) = (destination(ma), destination(mb));
        let (sa, sb) = (source_address(ma), source_address(mb));
        if da.is_some() && da == db && sa == sb {
            find(
                out,
                "N6",
                Some(b),
                format!(
                    "both streams of the pair use destination {} and the same source: the paths cannot be told apart",
                    da.map_or_else(String::new, |d| d.to_string())
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Built for these tests following ST 2110-10:2022 §6 and §8 (not copied from the standard).
    fn pair(red_dst: &str, blue_dst: &str) -> String {
        let leg = |mid: &str, dst: &str, src: &str| {
            format!(
                "m=video 20000 RTP/AVP 98\r\nc=IN IP4 {dst}/32\r\na=source-filter: incl IN IP4 {dst} {src}\r\na=rtpmap:98 raw/90000\r\n\
a=fmtp:98 sampling=YCbCr-4:2:2; width=1920; height=1080; exactframerate=50; depth=10; colorimetry=BT709; PM=2110GPM; SSN=ST2110-20:2017; TP=2110TPW\r\n\
a=ts-refclk:ptp=IEEE1588-2008:00-1B-21-FF-FE-12-34-56:127\r\na=mediaclk:direct=0\r\na=mid:{mid}\r\n"
            )
        };
        format!(
            "v=0\r\no=- 1 1 IN IP4 10.20.0.5\r\ns=t\r\nt=0 0\r\na=group:DUP red blue\r\n{}{}",
            leg("red", red_dst, "10.20.0.5"),
            leg("blue", blue_dst, "10.21.0.5")
        )
    }

    fn rules(text: &str) -> Vec<&'static str> {
        check(&Session::parse(text).unwrap()).into_iter().map(|f| f.rule).collect()
    }

    #[test]
    fn a_good_pair_passes() {
        let t = pair("239.20.1.1", "239.21.1.1");
        assert!(rules(&t).is_empty(), "{:?}", check(&Session::parse(&t).unwrap()));
        assert!(validate(&Session::parse(&t).unwrap()).is_ok());
        // Same destination is fine when the sources differ.
        assert!(rules(&pair("239.20.1.1", "239.20.1.1")).is_empty());
    }

    #[test]
    fn reserved_multicast_ranges_are_refused() {
        for dst in ["224.0.0.1", "224.0.0.251", "224.0.1.129", "224.0.1.0"] {
            assert_eq!(rules(&pair(dst, "239.21.1.1")), vec!["N2"], "{dst}");
        }
        for dst in ["224.0.2.1", "239.255.0.1", "224.1.0.1", "10.0.0.9"] {
            assert!(rules(&pair(dst, "239.21.1.1")).is_empty(), "{dst}");
        }
        // The error says which address and why.
        let e = validate(&Session::parse(&pair("224.0.0.7", "239.21.1.1")).unwrap()).unwrap_err().to_string();
        assert!(e.contains("224.0.0.7") && e.contains("224.0.0.0/24"), "{e}");
        // A session-level c= line counts too.
        let t = "v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=t\r\nc=IN IP4 224.0.1.1/32\r\nt=0 0\r\nm=video 5000 RTP/AVP 96\r\na=rtpmap:96 raw/90000\r\na=ts-refclk:localmac=00-11-22-33-44-55\r\na=mediaclk:sender\r\n";
        assert_eq!(rules(t), vec!["N2"]);
    }

    #[test]
    fn identical_source_and_destination_in_a_pair_is_refused() {
        let t = pair("239.20.1.1", "239.20.1.1").replace("10.21.0.5", "10.20.0.5");
        assert_eq!(rules(&t), vec!["N6"]);
    }

    #[test]
    fn pair_legs_must_agree_and_exist() {
        let t = pair("239.20.1.1", "239.21.1.1");
        // Different clock in the second leg.
        let mut parts = t.rsplitn(2, "m=video");
        let (tail, head) = (parts.next().unwrap().to_string(), parts.next().unwrap().to_string());
        let t2 = format!("{head}m=video{}", tail.replace("mediaclk:direct=0", "mediaclk:sender"));
        assert_eq!(rules(&t2), vec!["N6"]);
        // Different fmtp.
        assert_eq!(rules(&t.replacen("depth=10", "depth=8", 1)), vec!["N6"]);
        // Unknown mid, one stream, three streams.
        assert_eq!(rules(&t.replace("group:DUP red blue", "group:DUP red green")), vec!["N6"]);
        assert_eq!(rules(&t.replace("group:DUP red blue", "group:DUP red red")), vec!["N6"]);
        assert_eq!(rules(&t.replace("group:DUP red blue", "group:DUP red")), vec!["N6"]);
        assert_eq!(rules(&t.replace("group:DUP red blue", "group:DUP red blue extra")), vec!["N6"]);
        // Other group semantics (LS) are not ours to judge.
        assert!(rules(&t.replace("group:DUP red blue", "group:LS red blue")).is_empty());
    }

    #[test]
    fn rtp_multiplexing_is_refused() {
        let t = pair("239.20.1.1", "239.21.1.1").replace("RTP/AVP 98", "RTP/AVP 98 99");
        assert_eq!(rules(&t), vec!["N3", "N3"]);
    }

    #[test]
    fn maxudp_range() {
        let with = |v: &str| pair("239.20.1.1", "239.21.1.1").replace("TP=2110TPW", &format!("TP=2110TPW; MAXUDP={v}"));
        // Both legs carry it, so the pair still agrees; only N4 fires, once per leg.
        assert_eq!(rules(&with("8960")), Vec::<&str>::new());
        assert_eq!(rules(&with("1461")), Vec::<&str>::new());
        assert_eq!(rules(&with("9000")), vec!["N4", "N4"]);
        assert_eq!(rules(&with("1460")), vec!["N4", "N4"]);
        assert_eq!(rules(&with("big")), vec!["N4", "N4"]);
    }

    #[test]
    fn clock_attributes_are_required_and_consistent() {
        let t = pair("239.20.1.1", "239.21.1.1");
        assert_eq!(rules(&t.replacen("a=ts-refclk:ptp=IEEE1588-2008:00-1B-21-FF-FE-12-34-56:127\r\n", "", 1)), vec!["N5", "N6"]);
        assert!(rules(&t.replace("mediaclk:direct=0", "mediaclk:direct=5")).iter().all(|r| *r == "N5"));
        assert!(rules(&t.replace("mediaclk:direct=0", "mediaclk:sender")).is_empty());
        assert!(rules(&t.replace("mediaclk:direct=0", "mediaclk:what")).iter().all(|r| *r == "N5"));
        // A session-level pair of clock lines serves every media section.
        let s = "v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=t\r\nt=0 0\r\na=ts-refclk:localmac=00-11-22-33-44-55\r\na=mediaclk:sender\r\nm=audio 5004 RTP/AVP 97\r\nc=IN IP4 239.1.1.1/32\r\na=rtpmap:97 L24/48000/2\r\n";
        assert!(rules(s).is_empty());
    }

    #[test]
    fn jxsv_is_refused_in_every_format_slot() {
        let t = "v=0\r\no=- 1 1 IN IP4 1.2.3.4\r\ns=t\r\nt=0 0\r\nm=video 5000 RTP/AVP 96\r\nc=IN IP4 239.1.1.1/32\r\na=rtpmap:96 jxsv/90000\r\na=ts-refclk:localmac=00-11-22-33-44-55\r\na=mediaclk:sender\r\n";
        assert_eq!(rules(t), vec!["N1"]);
        let e = validate(&Session::parse(t).unwrap()).unwrap_err().to_string();
        assert!(e.contains("JPEG XS") && e.contains("D10"), "{e}");
        // Not only the first payload type: a stray rtpmap is found too.
        let t2 = t.replace("rtpmap:96 jxsv/90000", "rtpmap:96 raw/90000\r\na=rtpmap:97 JXSV/90000");
        assert_eq!(rules(&t2), vec!["N1"]);
        assert!(rules(&t.replace("jxsv", "raw")).is_empty());
    }

    #[test]
    fn the_roundtrip_of_a_checked_session_is_stable() {
        let t = pair("239.20.1.1", "239.21.1.1");
        let s = Session::parse(&t).unwrap();
        assert_eq!(Session::parse(&s.write()).unwrap(), s);
        assert_eq!(check(&Session::parse(&s.write()).unwrap()), check(&s));
    }
}
