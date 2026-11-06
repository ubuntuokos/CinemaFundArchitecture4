//! IABitstream writer: the inverse of [`crate::parse`].

use cfa4_bitstream::BitWriter;

use crate::frame::Ctx;
use crate::{
    AudioData, Bed, Coding, Decorrelation, Description, Error, Frame, Gain, Object, ObjectZones19, Remap, Result, Snap, Spread, ZoneGain, dlc, element,
};

/// Plex(n) coding in the smallest container (5.2).
fn plex(w: &mut BitWriter, v: u32, n: u32) -> Result<()> {
    if v == u32::MAX {
        return Err(Error::Invalid("Plex value 0xFFFFFFFF"));
    }
    let mut width = n;
    while width < 32 && v >= (1u32 << width) - 1 {
        w.write_bits((1u32 << width) - 1, width);
        width *= 2;
    }
    w.write_bits(v, width);
    Ok(())
}

fn put_element(out: &mut Vec<u8>, id: u32, payload: &[u8]) -> Result<()> {
    let mut w = BitWriter::new();
    plex(&mut w, id, 8)?;
    plex(&mut w, u32::try_from(payload.len()).map_err(|_| Error::Invalid("element larger than 4 GiB"))?, 8)?;
    out.extend(w.finish());
    out.extend_from_slice(payload);
    Ok(())
}

fn gain(w: &mut BitWriter, g: Gain) {
    match g {
        Gain::Unity => w.write_bits(0, 2),
        Gain::Silent => w.write_bits(1, 2),
        Gain::Code(c) => {
            w.write_bits(2, 2);
            w.write_bits(u32::from(c.min(0x3FF)), 10);
        }
    }
}

fn zone_gain(w: &mut BitWriter, g: ZoneGain) {
    match g {
        ZoneGain::Zero => w.write_bits(0, 2),
        ZoneGain::Unity => w.write_bits(1, 2),
        ZoneGain::Code(c) => {
            w.write_bits(2, 2);
            w.write_bits(u32::from(c.min(0x3FF)), 10);
        }
    }
}

fn decorrelation(w: &mut BitWriter, d: Decorrelation) {
    match d {
        Decorrelation::None => w.write_bits(0, 2),
        Decorrelation::Max => w.write_bits(1, 2),
        Decorrelation::Coef(c) => {
            w.write_bits(2, 2);
            w.write_bits(u32::from(c), 8);
        }
    }
}

fn description(w: &mut BitWriter, d: &Description) {
    let bits = if d.text.is_some() { d.bits | 0x80 } else { d.bits & 0x7F };
    w.write_bits(u32::from(bits), 8);
    if let Some(t) = &d.text {
        // strict ASCII, 63 characters and the NUL (Annex C)
        for c in t.bytes().filter(|c| (1..0x80).contains(c)).take(63) {
            w.write_bits(u32::from(c), 8);
        }
        w.write_bits(0, 8);
    }
}

fn count(n: usize) -> Result<u32> {
    u32::try_from(n).ok().filter(|&n| n != u32::MAX).ok_or(Error::Invalid("too many elements"))
}

fn bed(b: &Bed, ctx: &Ctx) -> Result<Vec<u8>> {
    let mut w = BitWriter::new();
    plex(&mut w, b.meta_id, 8)?;
    match b.use_case {
        Some(u) => {
            w.write_bit(true);
            w.write_bits(u32::from(u), 8);
        }
        None => w.write_bit(false),
    }
    plex(&mut w, count(b.channels.len())?, 4)?;
    for c in &b.channels {
        plex(&mut w, c.id, 4)?;
        plex(&mut w, c.audio_id, 8)?;
        gain(&mut w, c.gain);
        match c.decorrelation {
            Some(d) => {
                w.write_bit(true);
                w.write_bits(0, 4);
                decorrelation(&mut w, d);
            }
            None => w.write_bit(false),
        }
    }
    w.write_bits(0x180, 10);
    w.align_zero();
    description(&mut w, &b.description);
    let mut kids = Vec::new();
    for c in &b.beds {
        put_element(&mut kids, element::BED_DEFINITION, &bed(c, ctx)?)?;
    }
    for r in &b.remaps {
        put_element(&mut kids, element::BED_REMAP, &remap(r, ctx)?)?;
    }
    plex(&mut w, count(b.beds.len() + b.remaps.len())?, 8)?;
    let mut out = w.finish();
    out.extend(kids);
    Ok(out)
}

fn remap(r: &Remap, ctx: &Ctx) -> Result<Vec<u8>> {
    let mut w = BitWriter::new();
    plex(&mut w, r.meta_id, 8)?;
    w.write_bits(u32::from(r.use_case), 8);
    plex(&mut w, r.sources, 4)?;
    plex(&mut w, r.destinations, 4)?;
    for sb in 0..ctx.rate.sub_blocks() {
        let block = r.blocks.get(sb).and_then(Option::as_ref).or_else(|| if sb == 0 { r.blocks.iter().flatten().next() } else { None });
        if sb > 0 {
            w.write_bit(block.is_some());
        }
        let Some(block) = block else { continue };
        for o in 0..r.destinations as usize {
            plex(&mut w, block.destinations.get(o).copied().unwrap_or(0), 4)?;
            for i in 0..r.sources as usize {
                gain(&mut w, block.gains.get(o).and_then(|row| row.get(i)).copied().unwrap_or(Gain::Silent));
            }
        }
    }
    w.align_zero();
    plex(&mut w, 0, 8)?;
    Ok(w.finish())
}

fn object(o: &Object, ctx: &Ctx) -> Result<Vec<u8>> {
    let mut w = BitWriter::new();
    plex(&mut w, o.meta_id, 8)?;
    plex(&mut w, o.audio_id, 8)?;
    match o.use_case {
        Some(u) => {
            w.write_bit(true);
            w.write_bit(true);
            w.write_bits(u32::from(u), 8);
        }
        None => w.write_bit(false),
    }
    w.write_bit(false);
    for sb in 0..ctx.rate.sub_blocks() {
        let pan = o.pan.get(sb).and_then(Option::as_ref).or_else(|| if sb == 0 { o.pan.iter().flatten().next() } else { None });
        if sb > 0 {
            w.write_bit(pan.is_some());
        }
        let Some(p) = pan else {
            if sb == 0 {
                return Err(Error::Invalid("an Object needs panning for its first sub-block"));
            }
            continue;
        };
        gain(&mut w, p.gain);
        w.write_bits(1, 3);
        for c in p.position {
            w.write_bits(u32::from(c), 16);
        }
        match p.snap {
            Snap::Off => w.write_bit(false),
            Snap::On(tol) => {
                w.write_bit(true);
                match tol {
                    Some(t) => {
                        w.write_bit(true);
                        w.write_bits(u32::from(t.min(4095)), 12);
                    }
                    None => w.write_bit(false),
                }
                w.write_bit(false);
            }
        }
        match &p.zones {
            Some(z) => {
                w.write_bit(true);
                for g in z {
                    zone_gain(&mut w, *g);
                }
            }
            None => w.write_bit(false),
        }
        match p.spread {
            Spread::LowRes(c) => {
                w.write_bits(0, 2);
                w.write_bits(u32::from(c), 8);
            }
            Spread::None => w.write_bits(1, 2),
            Spread::OneD(c) => {
                w.write_bits(2, 2);
                w.write_bits(u32::from(c.min(4095)), 12);
            }
            Spread::ThreeD(c) => {
                w.write_bits(3, 2);
                for v in c {
                    w.write_bits(u32::from(v.min(4095)), 12);
                }
            }
        }
        w.write_bits(0, 4);
        decorrelation(&mut w, p.decorrelation);
    }
    w.align_zero();
    description(&mut w, &o.description);
    let mut kids = Vec::new();
    for c in &o.objects {
        put_element(&mut kids, element::OBJECT_DEFINITION, &object(c, ctx)?)?;
    }
    if let Some(z) = &o.zones19 {
        put_element(&mut kids, element::OBJECT_ZONE_DEFINITION19, &zones19(z, ctx))?;
    }
    plex(&mut w, count(o.objects.len() + usize::from(o.zones19.is_some()))?, 8)?;
    let mut out = w.finish();
    out.extend(kids);
    Ok(out)
}

fn zones19(z: &ObjectZones19, ctx: &Ctx) -> Vec<u8> {
    let mut w = BitWriter::new();
    for sb in 0..ctx.rate.sub_blocks() {
        let block = z.blocks.get(sb).and_then(Option::as_ref).or_else(|| if sb == 0 { z.blocks.iter().flatten().next() } else { None });
        if sb > 0 {
            w.write_bit(block.is_some());
        }
        let gains = match (sb, block) {
            (_, Some(b)) => *b,
            (0, None) => [ZoneGain::Unity; 19],
            _ => continue,
        };
        for g in gains {
            zone_gain(&mut w, g);
        }
    }
    w.finish()
}

fn audio(a: &AudioData, ctx: &Ctx) -> Result<(u32, Vec<u8>)> {
    let n = ctx.rate.samples(ctx.rate96, ctx.index);
    if a.samples.len() != n {
        return Err(Error::Invalid("audio asset sample count does not match the frame"));
    }
    let mut w = BitWriter::new();
    plex(&mut w, a.id, 8)?;
    let mut out = w.finish();
    match a.coding {
        Coding::Pcm => {
            for &s in &a.samples {
                let b = s.to_le_bytes();
                if ctx.bit_depth == 16 {
                    out.extend_from_slice(&b[2..]);
                } else {
                    out.extend_from_slice(&b[1..]);
                }
            }
            Ok((element::AUDIO_DATA_PCM, out))
        }
        Coding::Dlc => {
            let body = dlc::encode(&a.samples, ctx)?;
            let size = u16::try_from(body.len()).map_err(|_| Error::Invalid("DLC element larger than 64 KiB"))?;
            out.extend(size.to_be_bytes());
            out.extend(body);
            Ok((element::AUDIO_DATA_DLC, out))
        }
    }
}

/// Writes `f` as one IABitstream frame (an empty preamble and the IAFrame). PCM assets keep the
/// frame's bit depth (lower bits are dropped); DLC assets are lossless.
pub fn write(f: &Frame) -> Result<Vec<u8>> {
    let rate96 = match f.sample_rate {
        48_000 => false,
        96_000 => true,
        _ => return Err(Error::Unsupported("sample rate other than 48 or 96 kHz")),
    };
    let depth_code = match f.bit_depth {
        16 => 0,
        24 => 1,
        _ => return Err(Error::Unsupported("bit depth other than 16 or 24")),
    };
    if f.frame_index > 4 {
        return Err(Error::Invalid("FrameIndex above 4"));
    }
    let ctx = Ctx { rate: f.frame_rate, rate96, bit_depth: f.bit_depth, index: if f.frame_rate.has_sequence() { f.frame_index } else { 0 } };
    let mut kids = Vec::new();
    let mut n = 0;
    if f.frame_rate.has_sequence() {
        put_element(&mut kids, element::FRAME_SEQUENCE, &[ctx.index])?;
        n += 1;
    }
    for b in &f.beds {
        put_element(&mut kids, element::BED_DEFINITION, &bed(b, &ctx)?)?;
        n += 1;
    }
    for o in &f.objects {
        put_element(&mut kids, element::OBJECT_DEFINITION, &object(o, &ctx)?)?;
        n += 1;
    }
    for a in &f.audio {
        let (id, payload) = audio(a, &ctx)?;
        put_element(&mut kids, id, &payload)?;
        n += 1;
    }
    if let Some(t) = &f.authoring_tool {
        let mut b: Vec<u8> = t.bytes().filter(|c| (1..0x80).contains(c)).collect();
        b.push(0);
        put_element(&mut kids, element::AUTHORING_TOOL_INFO, &b)?;
        n += 1;
    }
    for u in &f.user_data {
        let mut b = u.user_id.to_vec();
        b.extend_from_slice(&u.bytes);
        put_element(&mut kids, element::USER_DATA, &b)?;
        n += 1;
    }
    let mut w = BitWriter::new();
    w.write_bits(u32::from(f.version), 8);
    w.write_bits(u32::from(rate96), 2);
    w.write_bits(depth_code, 2);
    w.write_bits(u32::from(f.frame_rate.code()), 4);
    plex(&mut w, f.max_rendered, 8)?;
    w.align_zero();
    plex(&mut w, count(n)?, 8)?;
    let mut payload = w.finish();
    payload.extend(kids);
    let mut iaframe = Vec::new();
    put_element(&mut iaframe, element::IA_FRAME, &payload)?;
    let mut out = vec![0x01, 0, 0, 0, 0, 0x02];
    out.extend(u32::try_from(iaframe.len()).map_err(|_| Error::Invalid("IAFrame larger than 4 GiB"))?.to_be_bytes());
    out.extend(iaframe);
    Ok(out)
}
