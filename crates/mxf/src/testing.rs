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
