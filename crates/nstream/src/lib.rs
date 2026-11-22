// Ported from FFmpeg n7.1.1: libavformat/brstm.c (Copyright (c) 2012 Paul B Mahol) and
// libavcodec/adpcm.c (the THP / DSP part, Copyright (c) 2001-2003 The FFmpeg project; THP ADPCM
// decoder by Marco Gerards) plus libavcodec/pcm.c (planar PCM); all LGPL-2.1-or-later
// (`COPYING.LGPL-2.1`); see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust support for Nintendo **BRSTM** (Wii), **BFSTM** (Wii U) and **BCSTM** (3DS) audio
//! streams (codec backlog CB-29.2 and CB-30.1): the three share one demuxer in ffmpeg and one
//! layout, a header with the stream description followed by blocks that hold every channel's data
//! one after the other. The sound is 8-bit or 16-bit PCM or Nintendo DSP ADPCM.
//!
//! - [`probe`] and [`parse`] read the header like ffmpeg's `brstm` / `bfstm` demuxers.
//! - [`decode`] walks the blocks and returns every channel's 16-bit samples.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid stream data: {0}")]
    Invalid(&'static str),
    #[error("unsupported stream feature: {0}")]
    Unsupported(&'static str),
    #[error("not a BRSTM / BFSTM / BCSTM stream")]
    NotStream,
}

pub type Result<T> = std::result::Result<T, Error>;

/// The container family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// `RSTM`, Wii.
    Brstm,
    /// `FSTM`, Wii U.
    Bfstm,
    /// `CSTM`, 3DS.
    Bcstm,
}

/// The sound format of the blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Pcm8,
    Pcm16 {
        little_endian: bool,
    },
    /// Nintendo DSP ADPCM (ffmpeg's `adpcm_thp` / `adpcm_thp_le`).
    Dsp {
        little_endian: bool,
    },
}

/// What the header says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stream {
    pub format: Format,
    pub little_endian: bool,
    pub codec: Codec,
    pub channels: u32,
    pub sample_rate: u32,
    /// The loop start in samples, when the stream loops.
    pub loop_start: Option<u32>,
    /// Samples per channel the header announces.
    pub duration: u32,
    pub block_count: u32,
    pub block_size: u32,
    pub samples_per_block: u32,
    pub last_block_used_bytes: u32,
    pub last_block_samples: u32,
    pub last_block_size: u32,
    /// Byte offset of the first block.
    pub data_start: usize,
    /// DSP coefficient tables, 32 bytes per channel.
    pub table: Vec<u8>,
    /// Per block and channel history (`ADPC` / `SEEK` chunk), 4 bytes each.
    pub adpc: Option<Vec<u8>>,
}

/// The container family a header starts like.
pub fn probe(head: &[u8]) -> Option<Format> {
    let bom = head.get(4..6)?;
    if !(bom == [0xFF, 0xFE] || bom == [0xFE, 0xFF]) {
        return None;
    }
    match head.get(..4)? {
        b"RSTM" => Some(Format::Brstm),
        b"FSTM" => Some(Format::Bfstm),
        b"CSTM" => Some(Format::Bcstm),
        _ => None,
    }
}

/// A cursor like ffmpeg's `AVIOContext`: reads past the end give zeros and set `eof`.
struct Reader<'a> {
    d: &'a [u8],
    pos: i64,
    eof: bool,
    le: bool,
}

impl Reader<'_> {
    fn r8(&mut self) -> u32 {
        let b = usize::try_from(self.pos).ok().and_then(|p| self.d.get(p).copied());
        match b {
            Some(b) => {
                self.pos += 1;
                u32::from(b)
            }
            None => {
                self.eof = true;
                0
            }
        }
    }
    fn rb16(&mut self) -> u32 {
        let a = self.r8();
        (a << 8) | self.r8()
    }
    fn r16(&mut self) -> u32 {
        let (a, b) = (self.r8(), self.r8());
        if self.le { (b << 8) | a } else { (a << 8) | b }
    }
    fn r32(&mut self) -> u32 {
        let (a, b) = (self.r16(), self.r16());
        if self.le { (b << 16) | a } else { (a << 16) | b }
    }
    fn tag(&mut self) -> [u8; 4] {
        [self.r8() as u8, self.r8() as u8, self.r8() as u8, self.r8() as u8]
    }
    fn skip(&mut self, n: i64) -> Result<()> {
        self.pos = self.pos.saturating_add(n);
        if self.pos < 0 {
            return Err(Error::Invalid("seek before the start"));
        }
        Ok(())
    }
    /// `avio_skip(pb, target - avio_tell(pb))`.
    fn seek(&mut self, target: i64) -> Result<()> {
        self.skip(target.saturating_sub(self.pos))
    }
    /// `avio_read`: the bytes, or `None` for a short read (the position still advances over what
    /// there was).
    fn read(&mut self, n: usize) -> Option<&[u8]> {
        let start = usize::try_from(self.pos).ok().filter(|&p| p <= self.d.len());
        let Some(start) = start else {
            self.eof = true;
            return None;
        };
        if self.d.len() - start < n {
            self.pos = self.d.len() as i64;
            self.eof = true;
            return None;
        }
        self.pos += n as i64;
        Some(&self.d[start..start + n])
    }
}

/// Read the header and find the first block, as ffmpeg's demuxers do.
pub fn parse(data: &[u8]) -> Result<Stream> {
    let format = probe(data).ok_or(Error::NotStream)?;
    let bfstm = format != Format::Brstm;
    let mut r = Reader { d: data, pos: 4, eof: false, le: false };
    let bom = r.rb16();
    if bom != 0xFEFF && bom != 0xFFFE {
        return Err(Error::Invalid("byte order"));
    }
    let little_endian = bom == 0xFFFE;
    r.le = little_endian;

    let mut start: u32 = 0;
    let pos: i64;
    if !bfstm {
        r.skip(2)?; // major and minor version
        r.skip(4)?; // size of file
        let size = r.r16();
        if size < 14 {
            return Err(Error::Invalid("header size"));
        }
        r.skip(i64::from(size) - 14)?;
        pos = r.pos;
        if &r.tag() != b"HEAD" {
            return Err(Error::Invalid("HEAD chunk"));
        }
    } else {
        let mut info_offset = 0u32;
        let header_size = r.r16();
        r.skip(8)?; // constant and size of file
        let section_count = r.r16();
        r.skip(2)?;
        let mut i = 0;
        while r.pos < i64::from(header_size) && !(start != 0 && info_offset != 0) && i < section_count {
            let flag = r.r16();
            r.skip(2)?;
            match flag {
                0x4000 => {
                    info_offset = r.r32();
                    r.r32();
                }
                0x4001 | 0x4003 => r.skip(8)?,
                0x4002 => {
                    start = r.r32().wrapping_add(8);
                    r.skip(4)?;
                }
                _ => {}
            }
            i += 1;
        }
        if info_offset == 0 || start == 0 {
            return Err(Error::Invalid("section table"));
        }
        r.seek(i64::from(info_offset))?;
        pos = r.pos;
        if &r.tag() != b"INFO" {
            return Err(Error::Invalid("INFO chunk"));
        }
    }

    let size = r.r32();
    if size < 40 {
        return Err(Error::Invalid("INFO size"));
    }
    r.skip(4)?;
    let h1offset = i64::from(r.r32());
    if h1offset > i64::from(size) {
        return Err(Error::Invalid("stream info offset"));
    }
    r.skip(12)?;
    let mut toffset = i64::from(r.r32()) + 16;
    if toffset > i64::from(size) {
        return Err(Error::Invalid("table offset"));
    }
    r.seek(pos + h1offset + 8)?;
    let codec = match r.r8() {
        0 => Codec::Pcm8,
        1 => Codec::Pcm16 { little_endian },
        2 => Codec::Dsp { little_endian },
        _ => return Err(Error::Unsupported("codec")),
    };
    let dsp = matches!(codec, Codec::Dsp { .. });
    let looping = r.r8() != 0;
    let channels = r.r8();
    if channels == 0 {
        return Err(Error::Invalid("channel count"));
    }
    r.skip(1)?;
    let sample_rate = if bfstm { r.r32() } else { r.r16() };
    if sample_rate == 0 || sample_rate > i32::MAX as u32 {
        return Err(Error::Invalid("sample rate"));
    }
    if !bfstm {
        r.skip(2)?;
    }
    let loop_start = if looping {
        Some(r.r32())
    } else {
        r.skip(4)?;
        None
    };
    let duration = r.r32();
    if !bfstm {
        start = r.r32();
    }
    let block_count = r.r32();
    if block_count > u32::from(u16::MAX) {
        return Err(Error::Invalid("too many blocks"));
    }
    let block_size = r.r32();
    if block_size > u32::MAX / channels {
        return Err(Error::Invalid("block size"));
    }
    let samples_per_block = r.r32();
    let last_block_used_bytes = r.r32();
    let last_block_samples = r.r32();
    let last_block_size = r.r32();
    if last_block_size > u32::MAX / channels {
        return Err(Error::Invalid("last block size"));
    }
    if last_block_used_bytes > last_block_size {
        return Err(Error::Invalid("last block used bytes"));
    }

    let ch = channels as usize;
    let mut table = Vec::new();
    if dsp {
        r.seek(pos + toffset)?;
        toffset = if !bfstm { i64::from(r.r32()) + 16 } else { toffset + i64::from(r.r32()) + i64::from(channels) * 8 - 8 };
        if toffset > i64::from(size) {
            return Err(Error::Invalid("coefficient table offset"));
        }
        let mut offsets: Vec<u32> = Vec::new();
        if !bfstm {
            r.seek(pos + toffset - 8 * (i64::from(channels) + 1))?;
            for _ in 0..ch {
                r.skip(4)?;
                offsets.push(r.r32());
            }
            // ffmpeg sorts the entries by offset; the table of the n-th channel is read from the
            // n-th smallest offset
            offsets.sort_unstable();
        }
        r.seek(pos + toffset)?;
        for c in 0..ch {
            if !bfstm {
                r.seek(pos + 16 + i64::from(offsets[c]))?;
            }
            let t = r.read(32).ok_or(Error::Invalid("coefficient table"))?;
            table.extend_from_slice(t);
            if bfstm {
                r.skip(14)?;
            }
        }
    }

    let used = r.pos - pos;
    if i64::from(size) < used {
        return Err(Error::Invalid("INFO size"));
    }
    r.skip(i64::from(size) - used)?;

    let mut adpc: Option<Vec<u8>> = None;
    while !r.eof {
        let chunk = r.tag();
        let csize = r.r32();
        if csize < 8 {
            return Err(Error::Invalid("chunk size"));
        }
        let csize = csize - 8;
        match &chunk {
            b"SEEK" | b"ADPC" if dsp => {
                let asize = block_count as usize * ch * 4;
                if (csize as usize) < asize {
                    return Err(Error::Invalid("ADPC chunk size"));
                }
                if adpc.is_some() {
                    r.skip(i64::from(csize))?;
                } else {
                    let mut v = vec![0u8; asize];
                    if bfstm && !matches!(codec, Codec::Dsp { little_endian: true }) {
                        // big-endian BFSTMs have little-endian SEEK tables
                        for i in (0..asize).step_by(2) {
                            v[i + 1] = r.r8() as u8;
                            v[i] = r.r8() as u8;
                        }
                    } else {
                        let at = usize::try_from(r.pos).ok().filter(|&p| p <= data.len());
                        let have = at.map_or(0, |p| (data.len() - p).min(asize));
                        if let Some(p) = at {
                            v[..have].copy_from_slice(&data[p..p + have]);
                        }
                        r.pos += have as i64;
                        if have < asize {
                            r.eof = true;
                        }
                    }
                    r.skip(i64::from(csize) - asize as i64)?;
                    adpc = Some(v);
                }
            }
            b"DATA" => {
                if i64::from(start) < r.pos || (adpc.is_none() && dsp) {
                    return Err(Error::Invalid("DATA chunk"));
                }
                r.seek(i64::from(start))?;
                if bfstm && dsp {
                    r.skip(24)?;
                }
                return Ok(Stream {
                    format,
                    little_endian,
                    codec,
                    channels,
                    sample_rate,
                    loop_start,
                    duration,
                    block_count,
                    block_size,
                    samples_per_block,
                    last_block_used_bytes,
                    last_block_samples,
                    last_block_size,
                    data_start: r.pos as usize,
                    table,
                    adpc,
                });
            }
            _ => r.skip(i64::from(csize))?,
        }
    }
    Err(Error::Invalid("no DATA chunk"))
}

/// The decoded sound.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decoded {
    /// One vector of 16-bit samples per channel.
    pub channels: Vec<Vec<i16>>,
    /// A block ended before its data did (ffmpeg stops there).
    pub truncated: bool,
}

/// Nintendo DSP ADPCM, ffmpeg's `adpcm_thp` / `adpcm_thp_le` on the packets the demuxer builds.
struct Dsp {
    little_endian: bool,
    history: Option<Vec<(i32, i32)>>,
}

impl Dsp {
    /// `packet`: sizes, coefficient tables, history and then every channel's bytes.
    fn decode(&mut self, packet: &[u8], ch: usize) -> Option<Vec<Vec<i16>>> {
        // ffmpeg's decoder allows at most 14 channels
        if ch == 0 || ch > 14 {
            return None;
        }
        let u32_at = |p: usize| -> Option<u32> {
            let b: [u8; 4] = packet.get(p..p + 4)?.try_into().ok()?;
            Some(if self.little_endian { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
        };
        let s16 = |p: usize| -> i32 {
            let b = [packet.get(p).copied().unwrap_or(0), packet.get(p + 1).copied().unwrap_or(0)];
            i32::from(if self.little_endian { i16::from_le_bytes(b) } else { i16::from_be_bytes(b) })
        };
        // the sample count get_nb_samples derives from the packet size
        let per = packet.len().checked_sub(8 + 36 * ch)? / ch;
        let est = per / 8 * 14 + if per % 8 > 1 { (per % 8 - 1) * 2 } else { 0 };
        let coded = u32_at(4)? as i32;
        if est == 0 || coded <= 0 || coded as usize > est {
            return None;
        }
        let n = coded as usize;
        let mut table = vec![[0i32; 16]; ch];
        let mut p = 8;
        for t in table.iter_mut() {
            for v in t.iter_mut() {
                *v = s16(p);
                p += 2;
            }
        }
        if self.history.is_none() {
            let mut h = Vec::new();
            for _ in 0..ch {
                h.push((s16(p), s16(p + 2)));
                p += 4;
            }
            self.history = Some(h);
        } else {
            p += ch * 4;
        }
        let hist = self.history.as_mut()?;
        let mut out = vec![vec![0i16; n]; ch];
        let get = |p: usize| i32::from(packet.get(p).copied().unwrap_or(0));
        for c in 0..ch {
            let (mut s1, mut s2) = hist[c];
            for i in 0..n.div_ceil(14) {
                let mut byte = get(p);
                p += 1;
                let index = ((byte >> 4) & 7) as usize;
                let exp = (byte & 15) as u32;
                let (f1, f2) = (i64::from(table[c][index * 2]), i64::from(table[c][index * 2 + 1]));
                let mut k = 0;
                while k < 14 && i * 14 + k < n {
                    let nib = if k & 1 == 1 {
                        (byte << 28) >> 28
                    } else {
                        byte = get(p);
                        p += 1;
                        ((byte >> 4) << 28) >> 28
                    };
                    let v = ((i64::from(s1) * f1 + i64::from(s2) * f2) >> 11) as i32;
                    let v = v.wrapping_add(nib.wrapping_mul(1 << exp)).clamp(-32768, 32767);
                    out[c][i * 14 + k] = v as i16;
                    s2 = s1;
                    s1 = v;
                    k += 1;
                }
            }
            hist[c] = (s1, s2);
        }
        Some(out)
    }
}

/// Decode the whole stream the way ffmpeg reads it: block after block until the announced count or
/// the first block that is cut short.
pub fn decode(data: &[u8], s: &Stream) -> Decoded {
    let ch = s.channels as usize;
    let mut out = Decoded { channels: vec![Vec::new(); ch], truncated: false };
    let mut dsp = Dsp { little_endian: matches!(s.codec, Codec::Dsp { little_endian: true }), history: None };
    let mut pos = s.data_start;
    for block in 1..=s.block_count {
        let (mut size, samples, mut skip);
        if block == s.block_count {
            size = s.last_block_used_bytes;
            samples = s.last_block_samples;
            skip = s.last_block_size.wrapping_sub(s.last_block_used_bytes);
            if samples < size.wrapping_mul(14) / 8 {
                let mut adjusted = samples / 14 * 8;
                if samples % 14 != 0 {
                    adjusted = adjusted.wrapping_add((samples % 14).div_ceil(2) + 1);
                }
                skip = skip.wrapping_add(size.wrapping_sub(adjusted));
                size = adjusted;
            }
        } else {
            size = s.block_size;
            samples = s.samples_per_block;
            skip = 0;
        }
        let size = size as usize;
        match s.codec {
            Codec::Dsp { .. } => {
                let Some(adpc) = s.adpc.as_deref() else {
                    out.truncated = true;
                    break;
                };
                // every channel's bytes must be there
                let mut packet = Vec::new();
                let le = dsp.little_endian;
                let put = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&if le { x.to_le_bytes() } else { x.to_be_bytes() });
                put(&mut packet, (size as u32).wrapping_mul(s.channels));
                put(&mut packet, samples);
                packet.extend_from_slice(&s.table);
                let a = 4 * ch * (block as usize - 1);
                packet.extend_from_slice(adpc.get(a..a + 4 * ch).unwrap_or(&[0; 0]));
                let mut ok = true;
                for _ in 0..ch {
                    match data.get(pos..pos.saturating_add(size)).filter(|d| d.len() == size) {
                        Some(d) => packet.extend_from_slice(d),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                    pos = pos.saturating_add(size).saturating_add(skip as usize);
                }
                if !ok {
                    out.truncated = true;
                    break;
                }
                // a block that does not decode adds nothing, as in ffmpeg
                if let Some(part) = dsp.decode(&packet, ch) {
                    for (dst, src) in out.channels.iter_mut().zip(part) {
                        dst.extend(src);
                    }
                }
            }
            Codec::Pcm8 | Codec::Pcm16 { .. } => {
                let total = size.saturating_mul(ch);
                let Some(d) = data.get(pos..pos.saturating_add(total)).filter(|d| d.len() == total) else {
                    out.truncated = true;
                    break;
                };
                pos += total;
                let bps = if matches!(s.codec, Codec::Pcm8) { 1 } else { 2 };
                // planar: n samples of each channel, the channels one after the other
                let n = total / (bps * ch);
                for (c, dst) in out.channels.iter_mut().enumerate() {
                    for i in 0..n {
                        let at = (c * n + i) * bps;
                        dst.push(match s.codec {
                            Codec::Pcm8 => i16::from(d[at] as i8) << 8,
                            Codec::Pcm16 { little_endian: true } => i16::from_le_bytes([d[at], d[at + 1]]),
                            _ => i16::from_be_bytes([d[at], d[at + 1]]),
                        });
                    }
                }
            }
        }
    }
    out
}
