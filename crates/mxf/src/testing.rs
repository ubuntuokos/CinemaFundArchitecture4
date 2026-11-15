//! Synthetic MXF files for tests (here and in dependent crates with the `test-util` feature): a
//! small writer for just enough of ST 377-1 to exercise the demuxer without external tools.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub fn ul(b: &[u8]) -> [u8; 16] {
    let mut k = [0u8; 16];
    k[..b.len()].copy_from_slice(b);
    k
}

pub fn ber(n: usize) -> Vec<u8> {
    let mut v = vec![0x83];
    v.extend_from_slice(&(n as u32).to_be_bytes()[1..]);
    v
}

pub fn klv(out: &mut Vec<u8>, key: [u8; 16], v: &[u8]) {
    out.extend_from_slice(&key);
    out.extend_from_slice(&ber(v.len()));
    out.extend_from_slice(v);
}

pub struct LocalSet(pub Vec<u8>);
impl LocalSet {
    pub fn new(uid: u8) -> Self {
        let mut s = LocalSet(Vec::new());
        s.p(0x3C0A, &uuid(uid));
        s
    }
    pub fn p(&mut self, tag: u16, v: &[u8]) -> &mut Self {
        self.0.extend_from_slice(&tag.to_be_bytes());
        self.0.extend_from_slice(&(v.len() as u16).to_be_bytes());
        self.0.extend_from_slice(v);
        self
    }
}

pub fn uuid(n: u8) -> [u8; 16] {
    let mut u = [0xAAu8; 16];
    u[15] = n;
    u
}

pub fn umid(n: u8) -> [u8; 32] {
    let mut u = [0x55u8; 32];
    u[31] = n;
    u
}

pub fn batch(items: &[&[u8]]) -> Vec<u8> {
    let mut v = (items.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(&(items.first().map_or(0, |i| i.len()) as u32).to_be_bytes());
    for i in items {
        v.extend_from_slice(i);
    }
    v
}

pub fn set_key(t: u8) -> [u8; 16] {
    ul(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x01, 0x01, 0x01, 0x01, t, 0x00])
}

pub const PICTURE_DEF: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x02, 0x01, 0, 0, 0];
pub const SOUND_DEF: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x02, 0x02, 0, 0, 0];
pub const TC_DEF: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x01, 0x01, 0, 0, 0];
pub const OP1A: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x01, 0x09, 0x00];
pub const PIC_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x15, 0x01, 0x05, 0x00];
pub const SND_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x16, 0x01, 0x03, 0x00];

pub fn partition(kind: u8, this: u64, body_sid: u32, index_sid: u32, hbc: u64) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&1u16.to_be_bytes());
    v.extend_from_slice(&3u16.to_be_bytes());
    v.extend_from_slice(&1u32.to_be_bytes());
    v.extend_from_slice(&this.to_be_bytes());
    v.extend_from_slice(&0u64.to_be_bytes());
    v.extend_from_slice(&0u64.to_be_bytes());
    v.extend_from_slice(&hbc.to_be_bytes());
    v.extend_from_slice(&0u64.to_be_bytes());
    v.extend_from_slice(&index_sid.to_be_bytes());
    v.extend_from_slice(&0u64.to_be_bytes());
    v.extend_from_slice(&body_sid.to_be_bytes());
    v.extend_from_slice(&OP1A);
    v.extend_from_slice(&batch(&[]));
    let mut out = Vec::new();
    klv(&mut out, ul(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, kind, 0x04, 0x00]), &v);
    out
}

pub fn track(uid: u8, id: u32, number: u32, seq: u8) -> Vec<u8> {
    let mut s = LocalSet::new(uid);
    s.p(0x4801, &id.to_be_bytes()).p(0x4804, &number.to_be_bytes()).p(0x4B01, &[0, 0, 0, 25, 0, 0, 0, 1]).p(0x4B02, &0i64.to_be_bytes()).p(0x4803, &uuid(seq));
    s.0
}

pub fn sequence(uid: u8, def: [u8; 16], dur: i64, comp: u8) -> Vec<u8> {
    let mut s = LocalSet::new(uid);
    s.p(0x0201, &def).p(0x0202, &dur.to_be_bytes()).p(0x1001, &batch(&[&uuid(comp)]));
    s.0
}

pub fn source_clip(uid: u8, def: [u8; 16], dur: i64, pkg: [u8; 32], track: u32) -> Vec<u8> {
    let mut s = LocalSet::new(uid);
    s.p(0x0201, &def).p(0x0202, &dur.to_be_bytes()).p(0x1201, &0i64.to_be_bytes()).p(0x1101, &pkg).p(0x1102, &track.to_be_bytes());
    s.0
}

pub const DATA_DEF: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x02, 0x03, 0, 0, 0];
pub const TT_CONTAINER: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0A, 0x0D, 0x01, 0x03, 0x01, 0x02, 0x13, 0x01, 0x01];
pub const TT_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x17, 0x01, 0x0B, 0x01];
pub const GS_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x0C, 0x0D, 0x01, 0x05, 0x09, 0x01, 0x00, 0x00, 0x00];

pub fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

/// A partition pack with a status byte and essence container labels.
pub fn partition_with(kind: u8, status: u8, this: u64, body_sid: u32, hbc: u64, containers: &[[u8; 16]]) -> Vec<u8> {
    let mut p = partition(kind, this, body_sid, 0, hbc);
    // partition() writes key(16) + BER(4) + 80 fixed bytes + an empty batch (8)
    p[14] = status;
    p.truncate(p.len() - 8);
    let refs: Vec<&[u8]> = containers.iter().map(|c| c.as_slice()).collect();
    p.extend_from_slice(&batch(&refs));
    let len = p.len() - 20;
    p[16..20].copy_from_slice(&ber(len));
    p
}

/// An ST 429-5 timed text track file: the XML clip-wrapped in body SID 1 and each resource in its
/// own generic stream partition (body SIDs 2, 3…).
pub fn build_timed_text(xml: &[u8], resources: &[(&str, &[u8])]) -> Vec<u8> {
    let mut md = Vec::new();
    let dynamic: [(u16, [u8; 8]); 7] = [
        (0xFF01, [0x01, 0x01, 0x15, 0x12, 0, 0, 0, 0]),
        (0xFF02, [0x04, 0x09, 0x05, 0x00, 0, 0, 0, 0]),
        (0xFF03, [0x01, 0x02, 0x01, 0x05, 0x01, 0, 0, 0]),
        (0xFF04, [0x06, 0x01, 0x01, 0x04, 0x06, 0x10, 0, 0]),
        (0xFF05, [0x01, 0x01, 0x15, 0x13, 0, 0, 0, 0]),
        (0xFF06, [0x04, 0x09, 0x02, 0x01, 0, 0, 0, 0]),
        (0xFF07, [0x01, 0x03, 0x04, 0x04, 0, 0, 0, 0]),
    ];
    let items: Vec<Vec<u8>> = dynamic
        .iter()
        .map(|(t, item)| {
            let mut v = t.to_be_bytes().to_vec();
            v.extend_from_slice(&[0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x0C]);
            v.extend_from_slice(item);
            v
        })
        .collect();
    let refs: Vec<&[u8]> = items.iter().map(Vec::as_slice).collect();
    klv(&mut md, ul(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x05, 0x01, 0x00]), &batch(&refs));
    let mut pre = LocalSet::new(1);
    pre.p(0x3B03, &uuid(2)).p(0x3B09, &OP1A);
    klv(&mut md, set_key(0x2F), &pre.0);
    let mut mp = LocalSet::new(3);
    mp.p(0x4401, &umid(1)).p(0x4403, &batch(&[&uuid(10)]));
    klv(&mut md, set_key(0x36), &mp.0);
    klv(&mut md, set_key(0x3B), &track(10, 2, 0, 20));
    klv(&mut md, set_key(0x0F), &sequence(20, DATA_DEF, 240, 30));
    klv(&mut md, set_key(0x11), &source_clip(30, DATA_DEF, 240, umid(2), 2));
    let mut fp = LocalSet::new(4);
    fp.p(0x4401, &umid(2)).p(0x4403, &batch(&[&uuid(11)])).p(0x4701, &uuid(40));
    klv(&mut md, set_key(0x37), &fp.0);
    klv(&mut md, set_key(0x3B), &track(11, 2, 0x1701_0B01, 21));
    klv(&mut md, set_key(0x0F), &sequence(21, DATA_DEF, 240, 31));
    klv(&mut md, set_key(0x11), &source_clip(31, DATA_DEF, 240, [0; 32], 0));
    let sub_uids: Vec<[u8; 16]> = (0..resources.len()).map(|i| uuid(50 + i as u8)).collect();
    let sub_refs: Vec<&[u8]> = sub_uids.iter().map(|u| u.as_slice()).collect();
    let mut desc = LocalSet::new(40);
    desc.p(0x3001, &[0, 0, 0, 24, 0, 0, 0, 1])
        .p(0x3002, &240i64.to_be_bytes())
        .p(0x3004, &TT_CONTAINER)
        .p(0xFF01, &[0x11; 16])
        .p(0xFF02, &utf16("UTF-8"))
        .p(0xFF03, &utf16("http://www.smpte-ra.org/schemas/428-7/2010/DCST"))
        .p(0xFF04, &batch(&sub_refs));
    klv(&mut md, set_key(0x64), &desc.0);
    for (i, (mime, _)) in resources.iter().enumerate() {
        let mut s = LocalSet::new(50 + i as u8);
        s.p(0xFF05, &[0x20 + i as u8; 16]).p(0xFF06, &utf16(mime)).p(0xFF07, &(2 + i as u32).to_be_bytes());
        klv(&mut md, set_key(0x65), &s.0);
    }
    let mut ecd = LocalSet::new(5);
    ecd.p(0x2701, &umid(2)).p(0x3F07, &1u32.to_be_bytes()).p(0x3F06, &0u32.to_be_bytes());
    klv(&mut md, set_key(0x23), &ecd.0);

    let mut f = partition_with(2, 0x04, 0, 0, md.len() as u64, &[TT_CONTAINER]);
    f.extend_from_slice(&md);
    let at = f.len() as u64;
    f.extend_from_slice(&partition_with(3, 0x04, at, 1, 0, &[TT_CONTAINER]));
    klv(&mut f, TT_KEY, xml);
    for (i, (_, data)) in resources.iter().enumerate() {
        let at = f.len() as u64;
        f.extend_from_slice(&partition_with(3, 0x11, at, 2 + i as u32, 0, &[]));
        klv(&mut f, GS_KEY, data);
    }
    let at = f.len() as u64;
    f.extend_from_slice(&partition_with(4, 0x04, at, 0, 0, &[TT_CONTAINER]));
    f
}

/// The ANC packet essence container (ST 436-1 Table 1).
pub const ANC_CONTAINER: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x02, 0x0E, 0x00, 0x00];
/// The frame-wrapped ANC element key (ST 436-1 Table 6).
pub const ANC_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x17, 0x01, 0x02, 0x01];

/// An ANC element value (ST 436-1 Table 7) of VANC frame packets on line 9: each packet's words
/// (DID, SDID, DC, user data) in the 8-bit luma coding (4), or the 10-bit coding (7) with even
/// parity bits and a checksum when `ten_bit`; payload arrays padded to four bytes (Annex B.3).
pub fn anc_element(packets: &[Vec<u8>], ten_bit: bool) -> Vec<u8> {
    let mut v = (packets.len() as u16).to_be_bytes().to_vec();
    for words in packets {
        v.extend_from_slice(&9u16.to_be_bytes());
        v.push(0x01);
        let (coding, count, mut payload) = if ten_bit {
            let parity = |b: u8| -> u16 {
                let p = (b.count_ones() & 1) as u16;
                b as u16 | p << 8 | (p ^ 1) << 9
            };
            let mut w: Vec<u16> = words.iter().map(|&b| parity(b)).collect();
            let sum = w.iter().fold(0u16, |a, &x| (a + (x & 0x1ff)) & 0x1ff);
            w.push(sum | ((!sum >> 8) & 1) << 9);
            let n = w.len();
            let mut p = Vec::new();
            for c in w.chunks(3) {
                let s = |i: usize| c.get(i).copied().unwrap_or(0) as u32;
                p.extend_from_slice(&(s(0) << 22 | s(1) << 12 | s(2) << 2).to_be_bytes());
            }
            (7u8, n, p)
        } else {
            (4u8, words.len(), words.clone())
        };
        while payload.len() % 4 != 0 {
            payload.push(0);
        }
        v.push(coding);
        v.extend_from_slice(&(count as u16).to_be_bytes());
        v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        v.extend_from_slice(&1u32.to_be_bytes());
        v.extend_from_slice(&payload);
    }
    v
}

/// The VBI line essence container (ST 436-1 Table 1, mapping 0Dh).
pub const VBI_CONTAINER: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x02, 0x0D, 0x00, 0x00];
/// The frame-wrapped VI element key (ST 436-1 Table 3).
pub const VI_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x17, 0x01, 0x01, 0x01];

/// A VI element value (ST 436-1 Table 4) of `(line, wrapping, coding, samples)` lines, the
/// samples given as 10-bit values: the 1-bit codings (1–3) store white for samples of 502 and
/// up, MSB first; the 8-bit ones (4–6) the high 8 bits; the 10-bit ones (7–9) three per word.
/// Payload arrays are padded to four bytes.
pub fn vi_element(lines: &[(u16, u8, u8, Vec<u16>)]) -> Vec<u8> {
    let mut v = (lines.len() as u16).to_be_bytes().to_vec();
    for (line, wrapping, coding, samples) in lines {
        let mut payload: Vec<u8> = match coding {
            1..=3 => samples.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |b, (k, &x)| if x >= 502 { b | 0x80 >> k } else { b })).collect(),
            7..=9 => samples
                .chunks(3)
                .flat_map(|c| {
                    let s = |i: usize| u32::from(c.get(i).copied().unwrap_or(0));
                    (s(0) << 22 | s(1) << 12 | s(2) << 2).to_be_bytes()
                })
                .collect(),
            _ => samples.iter().map(|&x| (x >> 2) as u8).collect(),
        };
        while !payload.len().is_multiple_of(4) {
            payload.push(0);
        }
        v.extend_from_slice(&line.to_be_bytes());
        v.push(*wrapping);
        v.push(*coding);
        v.extend_from_slice(&(samples.len() as u16).to_be_bytes());
        v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        v.extend_from_slice(&1u32.to_be_bytes());
        v.extend_from_slice(&payload);
    }
    v
}

/// An OP1a file with one ANC data track (ST 436-1) at `rate` (num, den): one frame-wrapped ANC
/// element per frame, `frames[i]` being frame `i`'s element value (see [`anc_element`]).
pub fn build_anc(rate: (u32, u32), frames: &[Vec<u8>]) -> Vec<u8> {
    build_436(rate, frames, false)
}

/// An OP1a file with one VBI data track (ST 436-1) at `rate`: one frame-wrapped VI element per
/// frame, `frames[i]` being frame `i`'s element value (see [`vi_element`]).
pub fn build_vi(rate: (u32, u32), frames: &[Vec<u8>]) -> Vec<u8> {
    build_436(rate, frames, true)
}

fn build_436(rate: (u32, u32), frames: &[Vec<u8>], vbi: bool) -> Vec<u8> {
    let (container, key, desc_kind, number) = if vbi { (VBI_CONTAINER, VI_KEY, 0x5B, 0x1701_0101u32) } else { (ANC_CONTAINER, ANC_KEY, 0x5C, 0x1701_0201) };
    let anc_def = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x02, 0x03, 0, 0, 0];
    let dur = frames.len() as i64;
    let mut r = rate.0.to_be_bytes().to_vec();
    r.extend_from_slice(&rate.1.to_be_bytes());
    let mut md = Vec::new();
    klv(&mut md, ul(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x05, 0x01, 0x00]), &batch(&[]));
    let mut pre = LocalSet::new(1);
    pre.p(0x3B03, &uuid(2)).p(0x3B09, &OP1A);
    klv(&mut md, set_key(0x2F), &pre.0);
    let mut mp = LocalSet::new(3);
    mp.p(0x4401, &umid(1)).p(0x4403, &batch(&[&uuid(10)]));
    klv(&mut md, set_key(0x36), &mp.0);
    let rated_track = |uid: u8, number: u32, seq: u8| {
        let mut s = LocalSet::new(uid);
        s.p(0x4801, &2u32.to_be_bytes()).p(0x4804, &number.to_be_bytes()).p(0x4B01, &r).p(0x4B02, &0i64.to_be_bytes()).p(0x4803, &uuid(seq));
        s.0
    };
    klv(&mut md, set_key(0x3B), &rated_track(10, 0, 20));
    klv(&mut md, set_key(0x0F), &sequence(20, anc_def, dur, 30));
    klv(&mut md, set_key(0x11), &source_clip(30, anc_def, dur, umid(2), 2));
    let mut fp = LocalSet::new(4);
    fp.p(0x4401, &umid(2)).p(0x4403, &batch(&[&uuid(11)])).p(0x4701, &uuid(40));
    klv(&mut md, set_key(0x37), &fp.0);
    klv(&mut md, set_key(0x3B), &rated_track(11, number, 21));
    klv(&mut md, set_key(0x0F), &sequence(21, anc_def, dur, 31));
    klv(&mut md, set_key(0x11), &source_clip(31, anc_def, dur, [0; 32], 0));
    let mut desc = LocalSet::new(40);
    desc.p(0x3001, &r).p(0x3002, &dur.to_be_bytes()).p(0x3004, &container);
    klv(&mut md, set_key(desc_kind), &desc.0);
    let mut ecd = LocalSet::new(5);
    ecd.p(0x2701, &umid(2)).p(0x3F07, &1u32.to_be_bytes()).p(0x3F06, &0u32.to_be_bytes());
    klv(&mut md, set_key(0x23), &ecd.0);

    let mut f = partition_with(2, 0x04, 0, 0, md.len() as u64, &[container]);
    f.extend_from_slice(&md);
    let at = f.len() as u64;
    f.extend_from_slice(&partition_with(3, 0x04, at, 1, 0, &[container]));
    for v in frames {
        klv(&mut f, key, v);
    }
    let at = f.len() as u64;
    f.extend_from_slice(&partition_with(4, 0x04, at, 0, 0, &[container]));
    f
}

/// An OP1a file with one AAC sound track (ST 381-4) at edit rate `rate`: AAC essence container
/// mapping kind `mapping` (17h ADTS, 18h LATM/LOAS), `elements` frame wrapped (one element per
/// edit unit) or, with `clip`, the one element clip wrapped; a generic sound descriptor with the
/// MPEG-4 AAC coding label, `sample_rate` and `channels`.
pub fn build_aac(rate: (u32, u32), sample_rate: u32, channels: u32, mapping: u8, elements: &[Vec<u8>], clip: bool) -> Vec<u8> {
    let container = ul(&[0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0D, 0x0D, 0x01, 0x03, 0x01, 0x02, mapping, if clip { 0x02 } else { 0x01 }, 0x00]);
    let coding = ul(&[0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0D, 0x04, 0x02, 0x02, 0x02, 0x04, 0x04, 0x01, 0x00]);
    let element_type = if clip { 0x06 } else { 0x05 };
    let key = ul(&[0x06, 0x0E, 0x2B, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x16, 0x01, element_type, 0x01]);
    let number = u32::from_be_bytes([0x16, 0x01, element_type, 0x01]);
    let dur = if clip { 1 } else { elements.len() as i64 };
    let mut r = rate.0.to_be_bytes().to_vec();
    r.extend_from_slice(&rate.1.to_be_bytes());
    let mut md = Vec::new();
    klv(&mut md, ul(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x05, 0x01, 0x00]), &batch(&[]));
    let mut pre = LocalSet::new(1);
    pre.p(0x3B03, &uuid(2)).p(0x3B09, &OP1A);
    klv(&mut md, set_key(0x2F), &pre.0);
    let mut mp = LocalSet::new(3);
    mp.p(0x4401, &umid(1)).p(0x4403, &batch(&[&uuid(10)]));
    klv(&mut md, set_key(0x36), &mp.0);
    let rated_track = |uid: u8, number: u32, seq: u8| {
        let mut s = LocalSet::new(uid);
        s.p(0x4801, &2u32.to_be_bytes()).p(0x4804, &number.to_be_bytes()).p(0x4B01, &r).p(0x4B02, &0i64.to_be_bytes()).p(0x4803, &uuid(seq));
        s.0
    };
    klv(&mut md, set_key(0x3B), &rated_track(10, 0, 20));
    klv(&mut md, set_key(0x0F), &sequence(20, SOUND_DEF, dur, 30));
    klv(&mut md, set_key(0x11), &source_clip(30, SOUND_DEF, dur, umid(2), 2));
    let mut fp = LocalSet::new(4);
    fp.p(0x4401, &umid(2)).p(0x4403, &batch(&[&uuid(11)])).p(0x4701, &uuid(40));
    klv(&mut md, set_key(0x37), &fp.0);
    klv(&mut md, set_key(0x3B), &rated_track(11, number, 21));
    klv(&mut md, set_key(0x0F), &sequence(21, SOUND_DEF, dur, 31));
    klv(&mut md, set_key(0x11), &source_clip(31, SOUND_DEF, dur, [0; 32], 0));
    let mut desc = LocalSet::new(40);
    let mut sr = sample_rate.to_be_bytes().to_vec();
    sr.extend_from_slice(&1u32.to_be_bytes());
    desc.p(0x3001, &r).p(0x3002, &dur.to_be_bytes()).p(0x3004, &container).p(0x3D03, &sr).p(0x3D07, &channels.to_be_bytes()).p(0x3D06, &coding);
    klv(&mut md, set_key(0x42), &desc.0);
    let mut ecd = LocalSet::new(5);
    ecd.p(0x2701, &umid(2)).p(0x3F07, &1u32.to_be_bytes()).p(0x3F06, &0u32.to_be_bytes());
    klv(&mut md, set_key(0x23), &ecd.0);

    let mut f = partition_with(2, 0x04, 0, 0, md.len() as u64, &[container]);
    f.extend_from_slice(&md);
    let at = f.len() as u64;
    f.extend_from_slice(&partition_with(3, 0x04, at, 1, 0, &[container]));
    for v in elements {
        klv(&mut f, key, v);
    }
    let at = f.len() as u64;
    f.extend_from_slice(&partition_with(4, 0x04, at, 0, 0, &[container]));
    f
}

/// A material sequence component of [`build_edit_list`]: a source clip into file package `pkg`
/// (0 or 1) from `start` for `dur` edit units, or a filler.
#[derive(Clone, Copy)]
pub enum Comp {
    Clip { pkg: u8, start: i64, dur: i64 },
    Filler(i64),
}

/// An OP3a file: two file packages with the same track ids and numbers in their own essence
/// containers (body SIDs 1 and 3, index SIDs 2 and 4), `n[p]` frame-wrapped content packages
/// each (picture `pic(p, i)`; 16-bit stereo sound of 1920 frames, sample k of package p =
/// p·10000 + k on the left, its negation on the right), and a material package whose picture and
/// sound sequences are `comps`. Each package has a footer index with temporal offsets `to` and
/// flags `flags`.
pub fn build_edit_list(comps: &[Comp], n: [usize; 2], pic: impl Fn(u8, usize) -> Vec<u8>, to: &[i8], flags: &[u8]) -> Vec<u8> {
    let op3a = ul(&[0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x03, 0x01, 0x09, 0x00]);
    let clip = |uid: u8, def: [u8; 16], dur: i64, start: i64, pkg: [u8; 32], track: u32| {
        let mut s = LocalSet::new(uid);
        s.p(0x0201, &def).p(0x0202, &dur.to_be_bytes()).p(0x1201, &start.to_be_bytes()).p(0x1101, &pkg).p(0x1102, &track.to_be_bytes());
        s.0
    };
    let mut md = Vec::new();
    klv(&mut md, ul(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x05, 0x01, 0x00]), &batch(&[]));
    let mut pre = LocalSet::new(1);
    pre.p(0x3B03, &uuid(2)).p(0x3B09, &op3a);
    klv(&mut md, set_key(0x2F), &pre.0);
    let mut cs = LocalSet::new(2);
    cs.p(0x1901, &batch(&[&uuid(3), &uuid(4), &uuid(5)]));
    klv(&mut md, set_key(0x18), &cs.0);
    // material package: picture and sound sequences of the components
    let mut mp = LocalSet::new(3);
    mp.p(0x4401, &umid(1)).p(0x4402, &[0, b'E', 0, b'L']).p(0x4403, &batch(&[&uuid(10), &uuid(11)]));
    klv(&mut md, set_key(0x36), &mp.0);
    let total = comps.iter().fold(0i64, |a, c| match c {
        Comp::Clip { dur, .. } | Comp::Filler(dur) => a.saturating_add(*dur),
    });
    for (k, (def, first_uid, track_id)) in [(PICTURE_DEF, 60u8, 2u32), (SOUND_DEF, 80, 3)].into_iter().enumerate() {
        let k = k as u8;
        klv(&mut md, set_key(0x3B), &track(10 + k, 1 + track_id - 1, 0, 20 + k));
        let uids: Vec<[u8; 16]> = (0..comps.len()).map(|j| uuid(first_uid + j as u8)).collect();
        let refs: Vec<&[u8]> = uids.iter().map(|u| &u[..]).collect();
        let mut seq = LocalSet::new(20 + k);
        seq.p(0x0201, &def).p(0x0202, &total.to_be_bytes()).p(0x1001, &batch(&refs));
        klv(&mut md, set_key(0x0F), &seq.0);
        for (j, c) in comps.iter().enumerate() {
            let uid = first_uid + j as u8;
            match *c {
                Comp::Clip { pkg, start, dur } => klv(&mut md, set_key(0x11), &clip(uid, def, dur, start, umid(2 + pkg), track_id)),
                Comp::Filler(dur) => {
                    let mut f = LocalSet::new(uid);
                    f.p(0x0201, &def).p(0x0202, &dur.to_be_bytes());
                    klv(&mut md, set_key(0x09), &f.0);
                }
            }
        }
    }
    // file packages
    for p in 0..2u8 {
        let d = n[p as usize] as i64;
        let mut fp = LocalSet::new(4 + p);
        fp.p(0x4401, &umid(2 + p)).p(0x4403, &batch(&[&uuid(13 + 2 * p), &uuid(14 + 2 * p)])).p(0x4701, &uuid(40 + 3 * p));
        klv(&mut md, set_key(0x37), &fp.0);
        klv(&mut md, set_key(0x3B), &track(13 + 2 * p, 2, 0x1501_0500, 23 + 2 * p));
        klv(&mut md, set_key(0x0F), &sequence(23 + 2 * p, PICTURE_DEF, d, 33 + 2 * p));
        klv(&mut md, set_key(0x11), &source_clip(33 + 2 * p, PICTURE_DEF, d, [0; 32], 0));
        klv(&mut md, set_key(0x3B), &track(14 + 2 * p, 3, 0x1601_0300, 24 + 2 * p));
        klv(&mut md, set_key(0x0F), &sequence(24 + 2 * p, SOUND_DEF, d, 34 + 2 * p));
        klv(&mut md, set_key(0x11), &source_clip(34 + 2 * p, SOUND_DEF, d, [0; 32], 0));
        let mut mdesc = LocalSet::new(40 + 3 * p);
        mdesc.p(0x3001, &[0, 0, 0, 25, 0, 0, 0, 1]).p(0x3F01, &batch(&[&uuid(41 + 3 * p), &uuid(42 + 3 * p)]));
        klv(&mut md, set_key(0x44), &mdesc.0);
        let mut cdci = LocalSet::new(41 + 3 * p);
        cdci.p(0x3006, &2u32.to_be_bytes())
            .p(0x3203, &64u32.to_be_bytes())
            .p(0x3202, &32u32.to_be_bytes())
            .p(0x320E, &[0, 0, 0, 2, 0, 0, 0, 1])
            .p(0x3301, &8u32.to_be_bytes())
            .p(0x3201, &ul(&[0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0A, 0x04, 0x01, 0x02, 0x02, 0x03, 0x06, 0x03, 0x00]));
        klv(&mut md, set_key(0x28), &cdci.0);
        let mut wave = LocalSet::new(42 + 3 * p);
        wave.p(0x3006, &3u32.to_be_bytes())
            .p(0x3D03, &[0, 0, 0xBB, 0x80, 0, 0, 0, 1])
            .p(0x3D07, &2u32.to_be_bytes())
            .p(0x3D01, &16u32.to_be_bytes())
            .p(0x3D0A, &4u16.to_be_bytes());
        klv(&mut md, set_key(0x48), &wave.0);
        let mut ecd = LocalSet::new(6 + p);
        ecd.p(0x2701, &umid(2 + p)).p(0x3F07, &(1 + 2 * p as u32).to_be_bytes()).p(0x3F06, &(2 + 2 * p as u32).to_be_bytes());
        klv(&mut md, set_key(0x23), &ecd.0);
    }
    let mut f = partition(2, 0, 0, 0, md.len() as u64);
    f.extend_from_slice(&md);
    let mut all_offsets = Vec::new();
    for p in 0..2u8 {
        let body_at = f.len() as u64;
        f.extend_from_slice(&partition(3, body_at, 1 + 2 * p as u32, 0, 0));
        let base = f.len() as u64;
        let mut offsets = Vec::new();
        for i in 0..n[p as usize] {
            offsets.push(f.len() as u64 - base);
            klv(&mut f, PIC_KEY, &pic(p, i));
            let mut snd = Vec::new();
            for k in 0..1920 {
                let s = (p as usize * 10000 + i * 1920 + k) as i16;
                snd.extend_from_slice(&s.to_le_bytes());
                snd.extend_from_slice(&s.wrapping_neg().to_le_bytes());
            }
            klv(&mut f, SND_KEY, &snd);
        }
        all_offsets.push(offsets);
    }
    let footer_at = f.len() as u64;
    f.extend_from_slice(&partition(4, footer_at, 0, 2, 0));
    for (p, offsets) in all_offsets.iter().enumerate() {
        let mut ix = LocalSet::new(8 + p as u8);
        let entries: Vec<Vec<u8>> = offsets
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let mut e = vec![to.get(i).copied().unwrap_or(0) as u8, 0, flags.get(i).copied().unwrap_or(0)];
                e.extend_from_slice(&o.to_be_bytes());
                e
            })
            .collect();
        let refs: Vec<&[u8]> = entries.iter().map(Vec::as_slice).collect();
        ix.p(0x3F0B, &[0, 0, 0, 25, 0, 0, 0, 1])
            .p(0x3F0C, &0i64.to_be_bytes())
            .p(0x3F0D, &(offsets.len() as i64).to_be_bytes())
            .p(0x3F06, &(2 + 2 * p as u32).to_be_bytes())
            .p(0x3F07, &(1 + 2 * p as u32).to_be_bytes())
            .p(0x3F0A, &batch(&refs));
        klv(&mut f, ul(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x10, 0x01, 0x00]), &ix.0);
    }
    f
}

// ---------- ST 429-6 encrypted track files ----------

const TRIPLET_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x04, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x02, 0x7E, 0x01, 0x00];

fn item(out: &mut Vec<u8>, v: &[u8]) {
    out.extend_from_slice(&ber(v.len()));
    out.extend_from_slice(v);
}

/// IV, then E(Check Value), the plaintext header and E(rest + RFC 2898 padding), one CBC chain.
fn esv(plain: &[u8], po: usize, key: &[u8; 16], iv: [u8; 16]) -> Vec<u8> {
    use aes::cipher::{BlockCipherEncrypt, KeyInit};
    let c = aes::Aes128::new_from_slice(key).unwrap();
    let mut body = crate::crypt::CHECK_VALUE.to_vec();
    body.extend_from_slice(&plain[po..]);
    let pad = 16 - (plain.len() - po) % 16;
    body.extend(std::iter::repeat_n(pad as u8, pad));
    let mut prev = iv;
    let mut enc = Vec::new();
    for b in body.chunks_exact(16) {
        let x: [u8; 16] = std::array::from_fn(|i| b[i] ^ prev[i]);
        let mut blk = aes::Block::from(x);
        c.encrypt_block(&mut blk);
        prev = blk.into();
        enc.extend_from_slice(&prev);
    }
    let mut out = iv.to_vec();
    out.extend_from_slice(&enc[..16]);
    out.extend_from_slice(&plain[..po]);
    out.extend_from_slice(&enc[16..]);
    out
}

/// An Encrypted Triplet value (ST 429-6) wrapping `plain` (source key `source`) under content key
/// `key`, plaintext offset `po`, with a TrackFile ID, sequence number and HMAC-SHA1 MIC when `mic_key`.
pub fn encrypted_triplet(key: &[u8; 16], context: &[u8; 16], source: [u8; 16], plain: &[u8], po: usize, seq: u64, mic_key: Option<[u8; 16]>) -> Vec<u8> {
    use hmac::{Hmac, KeyInit, Mac};
    let mut v = Vec::new();
    item(&mut v, context);
    item(&mut v, &(po as u64).to_be_bytes());
    item(&mut v, &source);
    item(&mut v, &(plain.len() as u64).to_be_bytes());
    let mic_from = v.len() + 4;
    let iv: [u8; 16] = std::array::from_fn(|i| (i as u8).wrapping_mul(17) ^ seq as u8);
    item(&mut v, &esv(plain, po, key, iv));
    match mic_key {
        Some(k) => {
            item(&mut v, &[0x7F; 16]);
            item(&mut v, &seq.to_be_bytes());
            v.extend_from_slice(&ber(20));
            let mut m = Hmac::<sha1::Sha1>::new_from_slice(&k).unwrap();
            m.update(&v[mic_from..]);
            v.extend_from_slice(&m.finalize().into_bytes());
        }
        None => {
            for _ in 0..3 {
                v.extend_from_slice(&ber(0));
            }
        }
    }
    v
}

/// Rewrites a plaintext track file as an encrypted one (ST 429-6 clauses 6-9): every essence
/// triplet (and generic stream data element) becomes an Encrypted Triplet, the primer gains the context's dynamic tags and a
/// Cryptographic Context set follows it.
pub fn encrypt_track_file(f: &[u8], key: &[u8; 16], key_id: &[u8; 16], context: &[u8; 16], po: usize, mic_key: Option<[u8; 16]>) -> Vec<u8> {
    let mic = mic_key.is_some();
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut seq = 1u64;
    while pos + 17 <= f.len() {
        let kbytes: [u8; 16] = f[pos..pos + 16].try_into().unwrap();
        let (len, n) = crate::klv::ber_length(&f[pos + 16..]).unwrap();
        let v = &f[pos + 16 + n..pos + 16 + n + len as usize];
        let k = crate::Ul(kbytes);
        if k.0[4] == 0x02 && k.0[5] == 0x05 && k.0[8..16] == [0x0D, 0x01, 0x02, 0x01, 0x01, 0x05, 0x01, 0x00] {
            // primer: append five entries
            let count = u32::from_be_bytes(v[..4].try_into().unwrap());
            let dynamic: [(u16, [u8; 16]); 5] = [
                (0xFF71, [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x09, 0x01, 0x01, 0x15, 0x11, 0, 0, 0, 0]),
                (0xFF72, [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x09, 0x06, 0x01, 0x01, 0x02, 0x02, 0, 0, 0]),
                (0xFF73, [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x09, 0x02, 0x09, 0x03, 0x01, 0x01, 0, 0, 0]),
                (0xFF74, [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x09, 0x02, 0x09, 0x03, 0x02, 0x01, 0, 0, 0]),
                (0xFF75, [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x09, 0x02, 0x09, 0x03, 0x01, 0x02, 0, 0, 0]),
            ];
            let mut p = (count + 5).to_be_bytes().to_vec();
            p.extend_from_slice(&v[4..]);
            for (tag, ul) in dynamic {
                p.extend_from_slice(&tag.to_be_bytes());
                p.extend_from_slice(&ul);
            }
            klv(&mut out, kbytes, &p);
            // the Cryptographic Context set
            let mut s = LocalSet::new(0xC5);
            s.p(0xFF71, context);
            s.p(0xFF72, &[0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x07, 0x0D, 0x01, 0x03, 0x01, 0x02, 0x0C, 0x01, 0x00]);
            s.p(0xFF73, &[0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x07, 0x02, 0x09, 0x02, 0x01, 0x01, 0x00, 0x00, 0x00]);
            s.p(0xFF74, &if mic { [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x07, 0x02, 0x09, 0x02, 0x02, 0x01, 0x00, 0x00, 0x00] } else { [0; 16] });
            s.p(0xFF75, key_id);
            let set_key = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x04, 0x01, 0x02, 0x02, 0x00, 0x00];
            klv(&mut out, set_key, &s.0);
        } else if (k.0[8..12] == [0x0D, 0x01, 0x03, 0x01] && matches!(k.0[12], 0x15..=0x17)) || k.0[8..13] == [0x0D, 0x01, 0x05, 0x09, 0x01] {
            // picture, sound and data (timed text) elements, and generic stream data (timed text
            // resources: ST 429-5 encrypts them with the document)
            let t = encrypted_triplet(key, context, k.0, v, po.min(v.len()), seq, mic_key);
            seq += 1;
            klv(&mut out, TRIPLET_KEY, &t);
        } else {
            out.extend_from_slice(&f[pos..pos + 16 + n + len as usize]);
        }
        pos += 16 + n + len as usize;
    }
    out
}

/// A 16-byte test key made at run time (SHA-1 of `label`, the process id and the clock), so
/// that no test carries a fixed cryptographic key.
pub fn test_key(label: &str) -> [u8; 16] {
    use sha1::Digest as _;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut h = sha1::Sha1::new();
    h.update(label.as_bytes());
    h.update(std::process::id().to_le_bytes());
    h.update(nanos.to_le_bytes());
    h.finalize()[..16].try_into().unwrap()
}
