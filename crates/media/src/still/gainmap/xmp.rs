//! Gain map metadata in XMP, from the public Adobe Gain Map Specification (1.0) and Google's Ultra
//! HDR Image Format v1.1: properties of the `hdrgm` namespace (`http://ns.adobe.com/hdr-gain-map/1.0/`)
//! on an `rdf:Description`, as attributes or as elements; the per-channel ones hold one value or an
//! `rdf:Seq` of three. `GainMapMin`, `GainMapMax`, `HDRCapacityMin` and `HDRCapacityMax` are log2
//! values. Missing properties take the specification's defaults (min 0, max 1, gamma 1, offsets 1/64,
//! capacity 0 to 1); `Version` is required.

use super::{Channel, GainMapParams, Origin};

const HDRGM: &str = "http://ns.adobe.com/hdr-gain-map/1.0/";

/// What the XMP of a picture says about gain maps.
#[derive(Debug, Default)]
pub struct Info {
    /// The gain map parameters (present in the gain map image's XMP).
    pub params: Option<GainMapParams>,
    /// Whether the XMP names an `hdrgm:Version` (Ultra HDR files carry it in the primary image).
    pub has_version: bool,
    /// The byte length of the gain map item from the GContainer directory (Ultra HDR v1.0), when given.
    pub gain_map_length: Option<usize>,
}

/// One or three numbers of a property.
fn values(desc: roxmltree::Node<'_, '_>, name: &str) -> Option<Vec<f32>> {
    if let Some(a) = desc.attribute((HDRGM, name)) {
        return a.trim().parse().ok().map(|v| vec![v]);
    }
    let el = desc.children().find(|n| n.is_element() && n.tag_name().namespace() == Some(HDRGM) && n.tag_name().name() == name)?;
    let li: Vec<f32> = el.descendants().filter(|n| n.is_element() && n.tag_name().name() == "li").filter_map(|n| n.text()?.trim().parse().ok()).collect();
    if !li.is_empty() {
        return Some(li);
    }
    el.text()?.trim().parse().ok().map(|v| vec![v])
}

/// Spread one or three values over the three channels.
fn per_channel(v: Option<Vec<f32>>, default: f32) -> Result<[f32; 3], String> {
    match v.as_deref() {
        None => Ok([default; 3]),
        Some([a]) => Ok([*a; 3]),
        Some([a, b, c]) => Ok([*a, *b, *c]),
        Some(v) => Err(format!("{} values where 1 or 3 are allowed", v.len())),
    }
}

fn scalar(v: Option<Vec<f32>>, default: f32) -> Result<f32, String> {
    match v.as_deref() {
        None => Ok(default),
        Some([a]) => Ok(*a),
        Some(_) => Err("a list where one value is allowed".into()),
    }
}

/// Read the XMP packet `xmp` (the APP1 payload after its identifier).
pub fn parse(xmp: &[u8]) -> Result<Info, String> {
    let text = std::str::from_utf8(xmp).map_err(|_| "XMP is not UTF-8".to_string())?;
    // the packet may be padded with whitespace or NULs after the closing processing instruction
    let text = text.trim_end_matches(['\0', ' ', '\n', '\r', '\t']);
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("XMP: {e}"))?;
    let mut info = Info::default();
    for n in doc.descendants().filter(|n| n.is_element()) {
        if n.tag_name().name() == "Item" {
            // GContainer directory entry: Item:Semantic="GainMap" Item:Length="..."
            let attr = |name: &str| n.attributes().find(|a| a.name() == name).map(|a| a.value());
            if attr("Semantic") == Some("GainMap") {
                info.gain_map_length = attr("Length").and_then(|l| l.trim().parse().ok());
            }
        }
        if n.tag_name().name() != "Description" {
            continue;
        }
        if values(n, "Version").is_some() || n.attribute((HDRGM, "Version")).is_some() {
            info.has_version = true;
        }
        let has_params = ["GainMapMax", "GainMapMin", "Gamma", "HDRCapacityMax"].iter().any(|p| values(n, p).is_some());
        if !has_params || info.params.is_some() {
            continue;
        }
        let min = per_channel(values(n, "GainMapMin"), 0.0)?;
        let max = per_channel(values(n, "GainMapMax"), 1.0)?;
        let gamma = per_channel(values(n, "Gamma"), 1.0)?;
        let off_sdr = per_channel(values(n, "OffsetSDR"), 1.0 / 64.0)?;
        let off_hdr = per_channel(values(n, "OffsetHDR"), 1.0 / 64.0)?;
        let cap_min = scalar(values(n, "HDRCapacityMin"), 0.0)?;
        let cap_max = scalar(values(n, "HDRCapacityMax"), 1.0)?;
        let backward = n
            .attribute((HDRGM, "BaseRenditionIsHDR"))
            .map(str::to_owned)
            .or_else(|| {
                n.children()
                    .find(|c| c.tag_name().namespace() == Some(HDRGM) && c.tag_name().name() == "BaseRenditionIsHDR")
                    .and_then(|c| c.text().map(str::to_owned))
            })
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"));
        // the base rendition's offset is OffsetSDR unless the base is the HDR one
        let (base_off, alt_off) = if backward { (off_hdr, off_sdr) } else { (off_sdr, off_hdr) };
        let (base_headroom, alternate_headroom) = if backward { (cap_max, cap_min) } else { (cap_min, cap_max) };
        let mut channels = [Channel::default(); 3];
        for (k, c) in channels.iter_mut().enumerate() {
            *c = Channel { min: min[k], max: max[k], gamma: gamma[k], base_offset: base_off[k], alternate_offset: alt_off[k] };
        }
        let p = GainMapParams { channels, base_headroom, alternate_headroom, use_base_color_space: true, backward, origin: Origin::Xmp };
        p.validate()?;
        info.params = Some(p);
    }
    Ok(info)
}

/// An XMP packet holding `params` (used by the tests to build Ultra HDR files from FFmpeg's output).
#[cfg(test)]
pub fn write(p: &GainMapParams) -> String {
    let seq = |f: &dyn Fn(&Channel) -> f32| {
        let v: Vec<String> = p.channels.iter().map(|c| format!("<rdf:li>{}</rdf:li>", f(c))).collect();
        format!("<rdf:Seq>{}</rdf:Seq>", v.join(""))
    };
    format!(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
         <rdf:Description xmlns:hdrgm=\"{HDRGM}\" hdrgm:Version=\"1.0\" hdrgm:HDRCapacityMin=\"{}\" hdrgm:HDRCapacityMax=\"{}\">\
         <hdrgm:GainMapMin>{}</hdrgm:GainMapMin><hdrgm:GainMapMax>{}</hdrgm:GainMapMax><hdrgm:Gamma>{}</hdrgm:Gamma>\
         <hdrgm:OffsetSDR>{}</hdrgm:OffsetSDR><hdrgm:OffsetHDR>{}</hdrgm:OffsetHDR>\
         </rdf:Description></rdf:RDF></x:xmpmeta>",
        p.base_headroom,
        p.alternate_headroom,
        seq(&|c| c.min),
        seq(&|c| c.max),
        seq(&|c| c.gamma),
        seq(&|c| c.base_offset),
        seq(&|c| c.alternate_offset),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_with_defaults() {
        let x = format!(
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
             <rdf:Description xmlns:hdrgm=\"{HDRGM}\" hdrgm:Version=\"1.0\" hdrgm:GainMapMax=\"2.5\" hdrgm:HDRCapacityMax=\"2.5\"/>\
             </rdf:RDF></x:xmpmeta>\0\0  "
        );
        let i = parse(x.as_bytes()).unwrap();
        let p = i.params.unwrap();
        assert!(i.has_version);
        assert_eq!((p.base_headroom, p.alternate_headroom), (0.0, 2.5));
        assert_eq!(p.channels[2], Channel { min: 0.0, max: 2.5, gamma: 1.0, base_offset: 1.0 / 64.0, alternate_offset: 1.0 / 64.0 });
    }

    #[test]
    fn sequences_round_trip() {
        let mut p = GainMapParams::identity();
        p.alternate_headroom = 3.0;
        for (k, c) in p.channels.iter_mut().enumerate() {
            c.max = 1.0 + k as f32;
            c.gamma = 0.5 + k as f32;
        }
        p.origin = Origin::Xmp;
        assert_eq!(parse(write(&p).as_bytes()).unwrap().params, Some(p));
    }

    #[test]
    fn primary_directory_and_garbage() {
        let x = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
                 <rdf:Description xmlns:hdrgm=\"http://ns.adobe.com/hdr-gain-map/1.0/\" xmlns:Container=\"http://ns.google.com/photos/1.0/container/\" \
                 xmlns:Item=\"http://ns.google.com/photos/1.0/container/item/\" hdrgm:Version=\"1.0\"><Container:Directory><rdf:Seq>\
                 <rdf:li><Container:Item Item:Semantic=\"Primary\" Item:Mime=\"image/jpeg\"/></rdf:li>\
                 <rdf:li><Container:Item Item:Semantic=\"GainMap\" Item:Mime=\"image/jpeg\" Item:Length=\"1234\"/></rdf:li>\
                 </rdf:Seq></Container:Directory></rdf:Description></rdf:RDF></x:xmpmeta>";
        let i = parse(x.as_bytes()).unwrap();
        assert!(i.has_version && i.params.is_none());
        assert_eq!(i.gain_map_length, Some(1234));
        assert!(parse(b"<x:xmpmeta").is_err());
        assert!(parse(&[0xFF, 0xFE]).is_err());
    }
}
