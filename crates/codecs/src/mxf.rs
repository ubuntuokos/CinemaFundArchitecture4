//! MXF media source (`cfa4-mxf` demux, GOP-aware video via [`crate::gop`]).
//!
//! - Video: H.264 / AVC-Intra (Annex B byte stream, our decoder), MPEG-2 (D-10 / IMX, XDCAM HD /
//!   HD422; our decoder), VC-3 (DNxHD/DNxHR) and ProRes through the decoder factories, JPEG 2000
//!   (broadcast and DCP picture tracks, X'Y'Z' included; [`crate::j2k`]), DV, uncompressed
//!   pictures (ST 384, unpacked by `cfa4_mxf::uncompressed`; 8-bit R'G'B'(A) as RGBA, deeper
//!   R'G'B' as BT.709 Y'CbCr 4:4:4). Other codings (MPEG-4 visual…) are identified and reported
//!   as unsupported; the file still opens so its audio can be used.
//! - Presentation order: the index table's temporal offsets; for AVC whose index marks B pictures
//!   but carries no temporal offsets, the picture order counts of the slice headers (8.2.1).
//! - Audio: every PCM sound track (Broadcast Wave / AES3, ST 382; ST 331 AES3 elements in D-10)
//!   is presented as one multichannel stream, sample-exact. Without PCM, an Immersive Audio
//!   (IAB, ST 2098-2) track of a DCP or IMF package is decoded with `cfa4-iab` and rendered to
//!   7.1 (L R C LFE Lrs Rrs Lss Rss).
//! - Start timecode: the material package's timecode track.
//! - OP-Atom clips over several files (Avid, P2): [`MxfSource::open_group`] opens the picture
//!   file and the sound files of one clip as one source (the engine finds them by material package).

use std::sync::{Arc, Mutex, PoisonError};

use cfa4_frame::{AudioBuffer, VideoFrame};
use cfa4_isobmff::{FourCc, SampleEntry};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_mxf::{Codec, MxfFile, Timecode, TrackKind};
use cfa4_time::{FrameRate, Tick};

use crate::gop::{GopCache, VideoSamples};
use crate::video::VideoDecoder;
use crate::{CodecError, make_video_decoder};

impl cfa4_mxf::ByteSource for crate::Src {
    fn len(&self) -> u64 {
        self.0.len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        self.0.read_at(offset, buf)
    }
}

pub fn sniff(head: &[u8]) -> bool {
    cfa4_mxf::sniff(head)
}

pub struct MxfSource {
    info: MediaInfo,
    bytes: crate::Src,
    file: MxfFile,
    vtrack: Option<usize>,
    /// Presentation position of each stored picture (overrides the container's when the order
    /// comes from the bitstream).
    vpts: Vec<i64>,
    /// Display position → stored picture.
    vorder: Vec<usize>,
    /// First presented display position, and the number presented.
    vfirst: i64,
    vcount: i64,
    video: GopCache,
    /// Why the video cannot be decoded (no decoder for the coding).
    unsupported: Option<String>,
    /// MPEG-2: the first sequence header (primes decoders that start without one).
    mpeg2_header: Vec<u8>,
    /// How the JPEG 2000 picture track's components are read.
    j2k_hints: crate::j2k::J2kHints,
    /// Uncompressed pictures (ST 384): the stored layout, the descriptor and the frames' colour.
    raw: Option<(cfa4_mxf::uncompressed::Layout, cfa4_mxf::PictureInfo, cfa4_color::ColorInfo)>,
    /// The other OP-Atom files of the clip (ST 390: one essence track per file, one material
    /// package): sound tracks may live there. Member 0 is `bytes`/`file`, member `k` is
    /// `companions[k - 1]`.
    companions: Vec<(crate::Src, MxfFile)>,
    /// Sound tracks combined into one stream: (member, track, first stored sample frame).
    atracks: Vec<(usize, usize, u64)>,
    /// Presented sample frames.
    aframes: u64,
    /// An IAB track rendered to loudspeakers, when there is no PCM.
    iab: Option<IabAudio>,
}

/// An Immersive Audio Bitstream track, rendered frame by frame.
struct IabAudio {
    track: usize,
    rate: cfa4_iab::FrameRate,
    rate96: bool,
    /// FrameIndex of stored frame 0 (the five-frame sample cadence of the 1001 rates).
    index0: u8,
    /// First presented stored frame and the number presented.
    first: usize,
    count: usize,
    layout: cfa4_iab::Layout,
    /// The last rendered frame (stored index).
    cache: Mutex<Option<(usize, Arc<Vec<Vec<f32>>>)>>,
}

impl IabAudio {
    /// Samples of stored frame `j`.
    fn frame_samples(&self, j: usize) -> u64 {
        self.rate.samples(self.rate96, ((usize::from(self.index0) + j) % 5) as u8) as u64
    }
    /// Samples before presented frame `k`.
    fn start_of(&self, k: usize) -> u64 {
        let cycle: u64 = (0..5).map(|i| self.frame_samples(self.first + i)).sum();
        let rest: u64 = (0..k % 5).map(|i| self.frame_samples(self.first + i)).sum();
        (k / 5) as u64 * cycle + rest
    }
    /// The presented frame holding sample `s`.
    fn frame_of(&self, s: u64) -> usize {
        let cycle = self.start_of(5).max(1);
        let mut k = (s / cycle) as usize * 5;
        while k < self.count && self.start_of(k + 1) <= s {
            k += 1;
        }
        k
    }
    /// Loudspeaker feeds of presented frame `k`; silence (and a warning) for a damaged frame.
    fn rendered(&self, file: &MxfFile, bytes: &crate::Src, k: usize) -> Arc<Vec<Vec<f32>>> {
        let j = self.first + k;
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((at, out)) = cache.as_ref()
            && *at == j
        {
            return out.clone();
        }
        let n = self.frame_samples(j) as usize;
        let out = file
            .read_sample(bytes, self.track, j)
            .map_err(|e| e.to_string())
            .and_then(|b| cfa4_iab::parse(&b).map_err(|e| e.to_string()))
            .and_then(|f| {
                // a frame whose rate disagrees with the track would shift every later frame:
                // it plays as silence of the track's cadence instead
                if f.frame_rate != self.rate || f.rate96() != self.rate96 {
                    return Err(format!("frame rate {:?} differs from the track's {:?}", f.frame_rate, self.rate));
                }
                cfa4_iab::render(&f, &self.layout).map_err(|e| e.to_string())
            })
            .unwrap_or_else(|e| {
                log::warn!("IAB frame {j}: {e}");
                vec![vec![0.0; n]; self.layout.channels()]
            });
        // every frame is exactly its cadence length, whatever the payload held
        let out: Vec<Vec<f32>> = out
            .into_iter()
            .map(|mut c| {
                c.resize(n, 0.0);
                c
            })
            .collect();
        let out = Arc::new(out);
        *cache = Some((j, out.clone()));
        out
    }
}

/// Picture order counts of AVC access units (Annex B), in stored order, as presentation
/// positions: the rank of (IDR period, POC) (8.2.1.1, picture order count type 0; types 1 and 2
/// never reorder for the streams written by AVC encoders we have seen, so they keep stored order).
pub fn avc_presentation_order(heads: &[Vec<u8>]) -> Option<Vec<i64>> {
    use cfa4_bitstream::{annexb_nals, unescape_rbsp};
    use cfa4_h264::params::{Pps, Sps};
    use cfa4_h264::slice::{NalHeader, SliceHeader};
    let mut spss: Vec<Option<Sps>> = vec![None; 32];
    let mut ppss: Vec<Option<Pps>> = vec![None; 256];
    let (mut prev_msb, mut prev_lsb) = (0i64, 0i64);
    let mut period = 0i64;
    let mut keys: Vec<(i64, i64)> = Vec::with_capacity(heads.len());
    for (i, h) in heads.iter().enumerate() {
        let mut key = None;
        for nal in annexb_nals(h) {
            let Ok(hdr) = NalHeader::parse(nal[0]) else { continue };
            let rbsp = unescape_rbsp(&nal[1..]);
            match hdr.nal_unit_type {
                7 => {
                    let sps = Sps::parse(&rbsp).ok()?;
                    let id = sps.id as usize;
                    *spss.get_mut(id)? = Some(sps);
                }
                8 => {
                    let pps = Pps::parse(&rbsp, &spss).ok()?;
                    let id = pps.id as usize;
                    *ppss.get_mut(id)? = Some(pps);
                }
                1 | 5 => {
                    let mut sps_of = None;
                    let (sh, _, _) = SliceHeader::parse(&rbsp, hdr, |id| {
                        let pps = ppss.get(id as usize).and_then(Option::as_ref).ok_or(cfa4_h264::Error::MissingParameterSet("PPS".into()))?;
                        let sps = spss.get(pps.sps_id as usize).and_then(Option::as_ref).ok_or(cfa4_h264::Error::MissingParameterSet("SPS".into()))?;
                        sps_of = Some(sps);
                        Ok((pps, sps))
                    })
                    .ok()?;
                    let sps = sps_of?;
                    if sh.idr {
                        if i > 0 {
                            period += 1;
                        }
                        prev_msb = 0;
                        prev_lsb = 0;
                    }
                    let poc = match sps.pic_order_cnt_type {
                        0 => {
                            let max = 1i64 << sps.log2_max_poc_lsb;
                            let lsb = sh.pic_order_cnt_lsb as i64;
                            let msb = if lsb < prev_lsb && prev_lsb - lsb >= max / 2 {
                                prev_msb + max
                            } else if lsb > prev_lsb && lsb - prev_lsb > max / 2 {
                                prev_msb - max
                            } else {
                                prev_msb
                            };
                            let top = msb + lsb;
                            let poc = if sh.field_pic { top } else { top.min(top + sh.delta_pic_order_cnt_bottom as i64) };
                            if hdr.nal_ref_idc != 0 {
                                prev_msb = msb;
                                prev_lsb = lsb;
                            }
                            poc
                        }
                        _ => i as i64,
                    };
                    if sh.has_mmco5() {
                        // memory_management_control_operation 5: later pictures start a new period
                        period += 1;
                        prev_msb = 0;
                        prev_lsb = 0;
                        key = Some((period - 1, poc));
                    } else {
                        key = Some((period, poc));
                    }
                    break;
                }
                _ => {}
            }
        }
        keys.push(key?);
    }
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by_key(|&i| (keys[i], i));
    let mut pts = vec![0i64; keys.len()];
    for (rank, &i) in order.iter().enumerate() {
        pts[i] = rank as i64;
    }
    Some(pts)
}

/// ProRes FourCC from the RDD 44 coding label profile byte.
fn prores_fourcc(profile: Option<u8>) -> FourCc {
    FourCc(match profile {
        Some(1) => *b"apco",
        Some(2) => *b"apcs",
        Some(4) => *b"apch",
        Some(5) => *b"ap4h",
        Some(6) => *b"ap4x",
        _ => *b"apcn",
    })
}

/// The item of a SMPTE RP 224 colorimetry label (06 0E 2B 34 04 01 01 vv 04 01 01 01 `group` `item`
/// 00 00) of `group` (1 transfer characteristic, 2 coding equations, 3 colour primaries).
fn colorimetry_item(ul: Option<cfa4_mxf::Ul>, group: u8) -> Option<u8> {
    let ul = ul?;
    (ul.0.get(..4) == Some(&[0x06, 0x0E, 0x2B, 0x34][..]) && ul.0.get(8..13) == Some(&[0x04, 0x01, 0x01, 0x01, group][..]))
        .then(|| ul.0.get(13).copied())
        .flatten()
}

/// The colorimetry the picture descriptor signals that CFA4 has a curve, gamut or matrix for.
fn descriptor_colorimetry(p: &cfa4_mxf::PictureInfo) -> crate::j2k::Signalled {
    use cfa4_color::{Matrix, Primaries, Transfer};
    crate::j2k::Signalled {
        transfer: match colorimetry_item(p.transfer_characteristic, 1) {
            Some(1..=5 | 9) => Some(Transfer::Bt709),
            Some(6) => Some(Transfer::Linear),
            Some(8) => Some(Transfer::Srgb),
            Some(0x0A) => Some(Transfer::Pq),
            Some(0x0B) => Some(Transfer::Hlg),
            _ => None,
        },
        primaries: match colorimetry_item(p.color_primaries, 3) {
            Some(1) => Some(Primaries::Bt601_525),
            Some(2) => Some(Primaries::Bt601_625),
            Some(3) => Some(Primaries::Bt709),
            Some(4) => Some(Primaries::Bt2020),
            Some(6) => Some(Primaries::P3D65),
            _ => None,
        },
        matrix: match colorimetry_item(p.coding_equations, 2) {
            Some(1) => Some(Matrix::Bt601),
            Some(2) => Some(Matrix::Bt709),
            Some(6) => Some(Matrix::Bt2020Ncl),
            _ => None,
        },
    }
}

fn prores_name(profile: Option<u8>) -> &'static str {
    match profile {
        Some(1) => "Apple ProRes 422 Proxy",
        Some(2) => "Apple ProRes 422 LT",
        Some(3) => "Apple ProRes 422",
        Some(4) => "Apple ProRes 422 HQ",
        Some(5) => "Apple ProRes 4444",
        Some(6) => "Apple ProRes 4444 XQ",
        _ => "Apple ProRes",
    }
}

/// The start timecode as a frame count at `rate` (the media's frame rate).
fn start_frames(tc: &Timecode, rate: FrameRate) -> i64 {
    let base = tc.rounded_base.max(1) as i64;
    // the real rate of the timecode: the media rate when it rounds to the base
    let (num, den) = if (rate.as_f64().round() as i64) == base {
        (rate.num, rate.den)
    } else if tc.drop_frame {
        (base * 1000, 1001)
    } else {
        (base, 1)
    };
    // frames at the tc rate → frames at `rate` (exact when they are the same rate)
    let n = tc.start as i128 * rate.num as i128 * den as i128;
    let d = num as i128 * rate.den as i128;
    ((n + d / 2) / d) as i64
}

impl MxfSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> crate::Result<Self> {
        Self::open_reader(name, Arc::new(cfa4_media::reader::MemReader(bytes)))
    }

    /// Open from a random-access reader: only the KLV headers, metadata and index are read now.
    pub fn open_reader(name: &str, reader: cfa4_media::SharedReader) -> crate::Result<Self> {
        Self::open_group(name, vec![reader], 0)
    }

    /// Open the OP-Atom files of one clip (ST 390: each file holds one essence track; the files
    /// share the material package) as one source: the picture from the file that has one, and the
    /// PCM sound tracks of every file, in the order given, as one multichannel stream. Reader
    /// `asked` is the file asked for: it must open; a companion that does not is skipped.
    pub fn open_group(name: &str, readers: Vec<cfa4_media::SharedReader>, asked: usize) -> crate::Result<Self> {
        let mut members: Vec<(crate::Src, MxfFile)> = Vec::new();
        for (i, reader) in readers.into_iter().enumerate() {
            let bytes = crate::Src(reader);
            match cfa4_mxf::open(&bytes) {
                Ok(f) => {
                    for w in &f.warnings {
                        log::warn!("{name}: {w}");
                    }
                    members.push((bytes, f));
                }
                Err(e) if i == asked => {
                    return Err(match e {
                        cfa4_mxf::Error::Unsupported(s) => CodecError::Unsupported(format!("MXF: {s}")),
                        other => CodecError::Container(other.to_string()),
                    });
                }
                Err(e) => log::warn!("{name}: companion OP-Atom file {i} skipped: {e}"),
            }
        }
        let primary = members.iter().position(|(_, f)| f.track_of_kind(TrackKind::Picture).is_some()).unwrap_or(0);
        if primary >= members.len() {
            return Err(CodecError::Container("MXF: no file to open".into()));
        }
        let (bytes, file) = members.remove(primary);
        let companions = members;
        let vtrack = file.track_of_kind(TrackKind::Picture);
        let pcm_tracks = |f: &MxfFile| -> Vec<usize> {
            f.tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| t.kind == TrackKind::Sound && !t.chunks.is_empty() && t.sound_format != cfa4_mxf::SoundFormat::Other)
                .map(|(i, _)| i)
                .collect()
        };
        let atracks: Vec<(usize, usize)> = std::iter::once(&file)
            .chain(companions.iter().map(|c| &c.1))
            .enumerate()
            .flat_map(|(m, f)| pcm_tracks(f).into_iter().map(move |t| (m, t)))
            .collect();
        let member_file = |m: usize| if m == 0 { Some(&file) } else { companions.get(m - 1).map(|c| &c.1) };
        // IAB, when there is no PCM: its first frame gives the rates
        let iab = if atracks.is_empty() {
            file.tracks.iter().enumerate().filter(|(_, t)| t.codec == Codec::Iab && !t.samples.is_empty()).find_map(|(i, t)| {
                let head = file.read_sample(&bytes, i, 0).ok()?;
                let f = cfa4_iab::parse(&head).map_err(|e| log::warn!("{name}: IAB track {}: {e}", t.track_id)).ok()?;
                let first = usize::try_from(t.first_edit_unit()).unwrap_or(0).min(t.samples.len());
                let avail = t.samples.len() - first;
                let count = t.duration.and_then(|d| usize::try_from(d).ok()).map_or(avail, |d| d.min(avail));
                Some(IabAudio {
                    track: i,
                    rate: f.frame_rate,
                    rate96: f.rate96(),
                    index0: f.frame_index,
                    first,
                    count,
                    layout: cfa4_iab::Layout::surround_7_1(),
                    cache: Mutex::new(None),
                })
            })
        } else {
            None
        };
        if vtrack.is_none() && atracks.is_empty() && iab.is_none() {
            return Err(CodecError::Unsupported("MXF: no playable picture or PCM sound tracks".into()));
        }
        // Video
        let mut video = None;
        let mut unsupported = None;
        let (mut vpts, mut vorder, mut vfirst, mut vcount) = (Vec::new(), Vec::new(), 0, 0);
        let mut explicit_color = None;
        let mut mpeg2_header = Vec::new();
        let mut j2k_hints = crate::j2k::J2kHints::default();
        let mut raw = None;
        if let Some(vi) = vtrack {
            let t = &file.tracks[vi];
            let p = t.picture.clone().unwrap_or_default();
            vpts = t.samples.iter().map(|s| s.pts).collect();
            vorder = t.display_order.clone();
            if t.needs_reorder && matches!(t.codec, Codec::Avc { .. }) {
                let heads: Vec<Vec<u8>> = (0..t.samples.len())
                    .map(|i| {
                        let s = t.samples[i];
                        cfa4_media::reader::read_range(&*bytes.0, s.offset, (s.size as usize).min(16 << 10)).unwrap_or_default()
                    })
                    .collect();
                match avc_presentation_order(&heads) {
                    Some(pts) => {
                        vorder = vec![0; pts.len()];
                        for (s, &d) in pts.iter().enumerate() {
                            vorder[d as usize] = s;
                        }
                        vpts = pts;
                    }
                    None => log::warn!("{name}: B pictures without temporal offsets and unreadable picture order: using stored order"),
                }
            }
            vfirst = t.first_edit_unit().min(vorder.len() as i64);
            let avail = vorder.len() as i64 - vfirst;
            vcount = t.duration.map_or(avail, |d| d.min(avail)).max(0);
            let rate = if t.edit_rate.is_valid() { FrameRate::new(t.edit_rate.num as i64, t.edit_rate.den as i64) } else { FrameRate::FPS_25 };
            // MPEG-2 (D-10 / IMX, XDCAM): the sequence header of the first picture
            let mpeg = (t.codec == Codec::Mpeg2)
                .then(|| file.read_sample(&bytes, vi, 0).ok())
                .flatten()
                .and_then(|s| Some((cfa4_mpeg2v::probe(&s)?, cfa4_mpeg2v::scan_access_unit(&s), crate::video::mpeg2_sequence_header(&s)?)));
            // JPEG 2000: the main header of the first picture
            j2k_hints =
                crate::j2k::J2kHints { rgb: p.rgb(), full_range: p.full_range(), separate_fields: p.frame_layout == 1, signalled: descriptor_colorimetry(&p) };
            let j2k = (t.codec == Codec::Jpeg2000)
                .then(|| file.read_sample(&bytes, vi, 0).ok())
                .flatten()
                .and_then(|s| cfa4_j2k::parse_header(cfa4_j2k::codestream(&s).ok()?).ok())
                .map(|mut h| {
                    if j2k_hints.separate_fields {
                        // woven fields have every chroma line (4:2:0 fields make a 4:2:2 frame)
                        h.components.iter_mut().for_each(|c| c.dy = 1);
                    }
                    (crate::j2k::components(&h, j2k_hints), h)
                });
            let (w, h) = match (&mpeg, &j2k) {
                (Some((info, _, _)), _) => (info.width, info.height),
                (_, Some((_, hd))) => (hd.width(), if j2k_hints.separate_fields { hd.height().saturating_mul(2) } else { hd.height() }),
                _ => (p.frame_width(), p.frame_height()),
            };
            let par = if p.aspect_ratio.is_valid() && w > 0 && h > 0 {
                let n = p.aspect_ratio.num as u64 * h as u64;
                let d = p.aspect_ratio.den as u64 * w as u64;
                let g = gcd(n, d).max(1);
                ((n / g) as u32, (d / g) as u32)
            } else {
                (1, 1)
            };
            // DV: format, sampling, aspect and colour are in the first frame's header and VAUX packs
            let dv = (t.codec == Codec::Dv).then(|| file.read_sample(&bytes, vi, 0).ok()).flatten().and_then(|s| crate::video::dv_stream_info(&s));
            let codec = match t.codec {
                Codec::Dv if dv.is_some() => dv.as_ref().map(|d| d.codec.clone()).unwrap_or_default(),
                Codec::Vc3 => file
                    .read_sample(&bytes, vi, 0)
                    .ok()
                    .and_then(|f| cfa4_dnx::probe(&f).ok())
                    .map(|h| cfa4_dnx::cid_name(h.cid))
                    .unwrap_or_else(|| "DNxHD/DNxHR".into()),
                Codec::ProRes { profile } => prores_name(profile).into(),
                c @ Codec::Avc { .. } => c.name().into(),
                Codec::Mpeg2 if mpeg.is_some() => mpeg.as_ref().map(|m| m.0.codec_name()).unwrap_or_default(),
                Codec::Uncompressed => match cfa4_mxf::uncompressed::Layout::of(&p) {
                    Ok(l) => {
                        let name = l.name();
                        raw = Some((l, p.clone(), cfa4_color::ColorInfo::REC709));
                        name
                    }
                    Err(e) => {
                        unsupported = Some(e.to_string());
                        "Uncompressed (unsupported)".into()
                    }
                },
                Codec::Jpeg2000 => match &j2k {
                    Some((Ok(_), hd)) if hd.digital_cinema() => "JPEG 2000 (digital cinema)".into(),
                    Some((Ok(_), _)) => "JPEG 2000".into(),
                    Some((Err(e), _)) => {
                        unsupported = Some(e.to_string());
                        "JPEG 2000 (unsupported)".into()
                    }
                    None => {
                        unsupported = Some("JPEG 2000: unreadable first picture".into());
                        "JPEG 2000 (unreadable)".into()
                    }
                },
                other => {
                    let why = format!("{} video in MXF (CFA4 has no {} decoder)", other.name(), other.name());
                    unsupported = Some(why);
                    format!("{} (unsupported)", other.name())
                }
            };
            let chroma = match (p.horizontal_subsampling, p.vertical_subsampling) {
                (2, 2) => "4:2:0",
                (2, 1) => "4:2:2",
                (1, 1) => "4:4:4",
                _ => "",
            };
            let mut pixel_format = if p.component_depth > 0 && !chroma.is_empty() { format!("YUV {chroma} {}-bit", p.component_depth) } else { String::new() };
            if let Some((info, au, header)) = &mpeg {
                let fo = au.pictures.first().and_then(|(ph, pce)| {
                    let pce = pce.as_ref()?;
                    let _ = ph;
                    (!pce.progressive_frame).then_some(if pce.picture_structure == 2 || (pce.picture_structure == 3 && !pce.top_field_first) {
                        cfa4_mpeg2v::FieldOrder::BottomFirst
                    } else {
                        cfa4_mpeg2v::FieldOrder::TopFirst
                    })
                });
                pixel_format = crate::video::mpeg2_pixel_format(info, fo);
                mpeg2_header = header.clone();
            }
            let mut color = cfa4_color::ColorInfo { matrix: cfa4_frame::default_matrix(w, h), ..cfa4_color::ColorInfo::REC709 };
            if let Some((Ok(c), hd)) = &j2k {
                // the decoder sets each frame's colour from what it decoded (R'G'B' and X'Y'Z'
                // become Y'CbCr) and the descriptor's colorimetry in the hints
                color = crate::j2k::color(*c, w, h, j2k_hints);
                let depth = hd.components.first().map_or(8, |c| c.precision);
                pixel_format = match c {
                    crate::j2k::Components::Ycbcr(ch) => format!("YUV {} {depth}-bit", ch.label()),
                    crate::j2k::Components::Gray => format!("Gray {depth}-bit"),
                    crate::j2k::Components::Rgb => format!("RGB {depth}-bit"),
                    crate::j2k::Components::Xyz => format!("X'Y'Z' {depth}-bit"),
                };
            } else if p.full_range() {
                color.range = cfa4_color::Range::Full;
                explicit_color = Some(color);
            }
            let mut has_alpha = p.alpha_depth > 0;
            if let Some((l, _, _)) = &raw {
                let chroma = match l.subsampling() {
                    (2, 2) => "4:2:0",
                    (2, 1) => "4:2:2",
                    _ => "4:4:4",
                };
                pixel_format =
                    if l.rgb() { format!("{} {}-bit", if l.alpha() { "RGBA" } else { "RGB" }, l.depth()) } else { format!("YUV {chroma} {}-bit", l.depth()) };
                has_alpha = l.alpha();
                // the frames carry their colour: the descriptor's colorimetry; R'G'B' deeper than
                // 8 bits becomes video-range BT.709 Y'CbCr 4:4:4
                let s = descriptor_colorimetry(&p);
                if l.rgb() {
                    color = cfa4_color::ColorInfo { matrix: cfa4_color::Matrix::Bt709, ..cfa4_color::ColorInfo::REC709 };
                } else if let Some(m) = s.matrix {
                    color.matrix = m;
                }
                color.transfer = s.transfer.unwrap_or(color.transfer);
                color.primaries = s.primaries.unwrap_or(color.primaries);
                explicit_color = None;
            }
            let mut par = par;
            if let Some(d) = &dv {
                pixel_format = d.pixel_format.clone();
                if !p.aspect_ratio.is_valid() {
                    par = d.par;
                }
                if explicit_color.is_none() {
                    color = d.color;
                }
            }
            let secs = vcount as f64 / rate.as_f64().max(1e-9);
            let bytes_total: u64 = t.samples.iter().map(|s| s.size as u64).sum();
            if let Some(r) = raw.as_mut() {
                r.2 = color;
            }
            video = Some(VideoStreamInfo {
                width: w,
                height: h,
                frame_rate: rate,
                par,
                codec,
                pixel_format,
                color,
                has_alpha,
                bitrate: (secs > 0.0).then(|| (bytes_total as f64 * 8.0 / (t.samples.len().max(1) as f64 / rate.as_f64().max(1e-9))) as u64),
                hdr: None,
            });
        }
        // Audio: all PCM sound tracks of the first track's rate, channels in track order
        let mut audio = None;
        let mut combined = Vec::new();
        let mut aframes = u64::MAX;
        if let Some(t0) = atracks.first().and_then(|&(m, t)| member_file(m)?.tracks.get(t)) {
            let si0 = t0.sound.clone().unwrap_or_default();
            let codec0 = t0.codec.name();
            let rate = si0.sample_rate;
            let mut channels: u32 = 0;
            for &(m, ai) in &atracks {
                let Some(t) = member_file(m).and_then(|f| f.tracks.get(ai)) else { continue };
                let Some(si) = t.sound.as_ref() else { continue };
                if si.sample_rate != rate {
                    continue;
                }
                let (n, d) = t.samples_per_edit_unit();
                let first = (t.first_edit_unit() as i128 * n as i128 / d.max(1) as i128) as u64;
                let stored = t.stored_sample_frames().saturating_sub(first);
                let presented = t.duration.map_or(stored, |du| ((du as i128 * n as i128 / d.max(1) as i128) as u64).min(stored));
                aframes = aframes.min(presented);
                let next =
                    channels.checked_add(si.channels.max(1)).ok_or_else(|| CodecError::Unsupported("MXF: combined audio channel count overflows".into()))?;
                if next > cfa4_media::MAX_AUDIO_CHANNELS {
                    return Err(CodecError::Unsupported(format!("MXF: combined audio exceeds {} channels", cfa4_media::MAX_AUDIO_CHANNELS)));
                }
                channels = next;
                combined.push((m, ai, first));
            }
            let sr = if rate.is_valid() { (rate.num as f64 / rate.den as f64).round() as u32 } else { 48_000 };
            if let Ok((sr, channels)) = cfa4_media::bounded_audio_dimensions(sr, channels) {
                audio = Some(AudioStreamInfo { sample_rate: sr, channels, codec: codec0.into(), bits_per_sample: Some(si0.bits), speakers: Vec::new() });
            }
        }
        if let Some(ia) = &iab {
            aframes = ia.start_of(ia.count);
            let sr = if ia.rate96 { 96_000 } else { 48_000 };
            audio = Some(AudioStreamInfo {
                sample_rate: sr,
                channels: ia.layout.channels() as u32,
                codec: "IAB".into(),
                bits_per_sample: Some(24),
                speakers: Vec::new(),
            });
        }
        if aframes == u64::MAX {
            aframes = 0;
        }
        let duration = match (&video, &audio) {
            (Some(v), _) => v.frame_rate.tick_of(vcount),
            (None, Some(a)) => Tick::from_units(aframes as i64, a.sample_rate as i64),
            _ => Tick::ZERO,
        };
        let mut info = MediaInfo {
            name: name.to_string(),
            kind: if video.is_some() { MediaKind::Movie } else { MediaKind::AudioOnly },
            duration,
            video,
            audio_streams: audio.into_iter().collect(),
            container: if companions.is_empty() {
                format!("MXF {}", file.operational_pattern.name())
            } else {
                format!("MXF {} ({} files)", file.operational_pattern.name(), companions.len() + 1)
            },
            start_timecode: None,
            file_size: Some(companions.iter().fold(bytes.0.len(), |n, c| n.saturating_add(c.0.0.len()))),
        };
        info.start_timecode = file.timecode.as_ref().map(|tc| start_frames(tc, info.frame_rate()));
        Ok(Self {
            info,
            bytes,
            vtrack,
            vpts,
            vorder,
            vfirst,
            vcount,
            video: GopCache::new(explicit_color),
            unsupported,
            mpeg2_header,
            j2k_hints,
            raw,
            atracks: combined,
            companions,
            aframes,
            iab,
            file,
        })
    }

    /// The demuxed file (tracks, partitions, index, timecode).
    pub fn file(&self) -> &MxfFile {
        &self.file
    }
}

/// Uncompressed pictures (ST 384): every edit unit unpacked by [`cfa4_mxf::uncompressed`].
struct RawDecoder {
    layout: cfa4_mxf::uncompressed::Layout,
    info: cfa4_mxf::PictureInfo,
    color: cfa4_color::ColorInfo,
}

impl VideoDecoder for RawDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> crate::Result<Vec<crate::video::DecodedFrame>> {
        let pic = cfa4_mxf::uncompressed::decode(&self.layout, &self.info, sample).map_err(|e| match e {
            cfa4_mxf::Error::Unsupported(s) => CodecError::Unsupported(s),
            other => CodecError::Decode(other.to_string()),
        })?;
        Ok(vec![crate::video::DecodedFrame { pts, frame: raw_frame(pic, self.color), draft: false }])
    }
    fn flush(&mut self) -> Vec<crate::video::DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {}
    fn name(&self) -> &str {
        "CFA4 uncompressed"
    }
    fn intra_only(&self) -> bool {
        true
    }
    fn is_random_access(&self, _sample: &[u8]) -> Option<bool> {
        Some(true)
    }
}

/// An unpacked uncompressed picture as a [`VideoFrame`] cut to its display rectangle: Y'CbCr as
/// stored (samples under 8 bits widened to 8), 8-bit R'G'B'(A) as RGBA, deeper R'G'B' as
/// video-range BT.709 Y'CbCr 4:4:4 with straight alpha.
fn raw_frame(pic: cfa4_mxf::uncompressed::Picture, color: cfa4_color::ColorInfo) -> VideoFrame {
    use cfa4_frame::{Chroma, PixelData, Region};
    let (w, h) = (pic.width, pic.height);
    let [mut p0, mut p1, mut p2] = pic.planes;
    let mut alpha = pic.alpha;
    let mut bits = pic.depth;
    if bits < 8 {
        for p in [&mut p0, &mut p1, &mut p2].into_iter().chain(alpha.as_mut()) {
            p.iter_mut().for_each(|v| *v <<= 8 - bits);
        }
        bits = 8;
    }
    let data = if pic.rgb && bits == 8 {
        let mut rgba = Vec::with_capacity(p0.len() * 4);
        for (i, ((r, g), b)) in p0.iter().zip(&p1).zip(&p2).enumerate() {
            let a = alpha.as_ref().and_then(|a| a.get(i)).map_or(255, |&a| a as u8);
            rgba.extend_from_slice(&[*r as u8, *g as u8, *b as u8, a]);
        }
        PixelData::Rgba8(Arc::new(rgba))
    } else {
        if pic.rgb {
            crate::j2k::rgb_to_ycbcr(&mut p0, &mut p1, &mut p2, bits);
        }
        let chroma = match pic.subsampling {
            (2, 2) => Chroma::C420,
            (2, 1) => Chroma::C422,
            _ => Chroma::C444,
        };
        if bits == 8 {
            let to8 = |p: Vec<u16>| Arc::new(p.into_iter().map(|v| v as u8).collect::<Vec<u8>>());
            PixelData::Yuv8 { planes: [to8(p0), to8(p1), to8(p2)], chroma, alpha: alpha.map(to8) }
        } else {
            PixelData::Yuv16 { planes: [Arc::new(p0), Arc::new(p1), Arc::new(p2)], chroma, bits, alpha: alpha.map(Arc::new) }
        }
    };
    let frame = VideoFrame { width: w, height: h, data, color, par: (1, 1), pts: Tick::ZERO };
    let (x, y, dw, dh) = pic.display;
    frame.cropped(Region { x: x as usize, y: y as usize, w: dw as usize, h: dh as usize })
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// The MXF picture track as a [`VideoSamples`] table (pts in display positions).
struct MxfVideo<'a> {
    src: &'a MxfSource,
    track: usize,
}

impl VideoSamples for MxfVideo<'_> {
    fn count(&self) -> usize {
        self.src.vpts.len()
    }
    fn pts(&self, i: usize) -> i64 {
        self.src.vpts[i]
    }
    fn sync_before(&self, i: usize) -> usize {
        // a leading picture of an open GOP (stored after its random-access picture, shown
        // before it) is predicted from the previous GOP too
        let track = &self.src.file.tracks[self.track];
        let pts = &self.src.vpts;
        crate::gop::open_gop_sync(i, |i| track.sync_before(i), |i, key| matches!((pts.get(i), pts.get(key)), (Some(a), Some(b)) if a < b))
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        usize::try_from(t).ok().and_then(|t| self.src.vorder.get(t).copied())
    }
    fn read(&self, i: usize) -> crate::Result<Vec<u8>> {
        self.src.file.read_sample(&self.src.bytes, self.track, i).map_err(|e| CodecError::Container(e.to_string()))
    }
    fn make_decoder(&self) -> crate::Result<Box<dyn VideoDecoder>> {
        if let Some(why) = &self.src.unsupported {
            return Err(CodecError::Unsupported(why.clone()));
        }
        let t = &self.src.file.tracks[self.track];
        let v = self.src.info.video.as_ref().ok_or_else(|| CodecError::Unsupported("no video track".into()))?;
        let (w, h) = (v.width.min(u16::MAX as u32) as u16, v.height.min(u16::MAX as u32) as u16);
        match t.codec {
            Codec::Avc { .. } => Ok(Box::new(crate::video::H264Decoder::annexb())),
            Codec::Mpeg2 => Ok(Box::new(crate::video::Mpeg2Decoder::new(self.src.mpeg2_header.clone()))),
            Codec::Vc3 => make_video_decoder(&SampleEntry::dnx(FourCc(*b"AVdh"), w, h)),
            Codec::ProRes { profile } => make_video_decoder(&SampleEntry::prores(prores_fourcc(profile), w, h)),
            Codec::Jpeg2000 => Ok(Box::new(crate::j2k::J2kDecoder::new(self.src.j2k_hints))),
            Codec::Dv => Ok(Box::new(crate::video::DvDecoder::default())),
            Codec::Uncompressed => match &self.src.raw {
                Some((layout, info, color)) => Ok(Box::new(RawDecoder { layout: layout.clone(), info: info.clone(), color: *color })),
                None => Err(CodecError::Unsupported("uncompressed video in MXF without a known layout".into())),
            },
            other => Err(CodecError::Unsupported(format!("{} video in MXF", other.name()))),
        }
    }
}

impl MediaSource for MxfSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> Result<Arc<VideoFrame>, MediaError> {
        let ti = self.vtrack.ok_or(MediaError::NoStream("video"))?;
        if let Some(why) = &self.unsupported {
            return Err(MediaError::Unsupported(why.clone()));
        }
        if self.vcount == 0 {
            return Err(MediaError::Decode("MXF: no pictures".into()));
        }
        let rate = self.info.frame_rate();
        let f = rate.frame_at(req.time.max(Tick::ZERO)).clamp(0, self.vcount - 1);
        let late = cfa4_media::cancel::catch_up().map(|m| self.vfirst + rate.frame_at(req.time - m));
        let frame = self.video.frame_late(&MxfVideo { src: self, track: ti }, self.vfirst + f, late)?;
        Ok(frame)
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer, MediaError> {
        let ainfo = self.info.audio().ok_or(MediaError::NoStream("audio"))?;
        let src_rate = ainfo.sample_rate;
        let ch = ainfo.channels.max(1) as usize;
        let exact = src_rate == sample_rate;
        let source_frames = if exact { frames } else { cfa4_media::resample_source_frames(frames, src_rate, sample_rate)? };
        let need =
            if exact { source_frames } else { source_frames.checked_add(2).ok_or_else(|| MediaError::Decode("audio source window overflows".into()))? };
        cfa4_media::validate_audio_buffer_shape(ch, need)?;
        cfa4_media::validate_audio_buffer_shape(ch, frames)?;
        let mut out = AudioBuffer::silence(sample_rate, ch, frames);
        let ratio = src_rate as f64 / sample_rate as f64;
        let s0 = if exact { start } else { (start as f64 * ratio).floor() as i64 };
        let need_i64 = i64::try_from(need).unwrap_or(i64::MAX);
        // clip the request to the presented range [0, aframes)
        let a = s0.max(0);
        let b = s0.saturating_add(need_i64).min(self.aframes as i64);
        let mut src: Vec<Vec<f32>> = vec![vec![0.0; need]; ch];
        if let (Some(ia), true) = (&self.iab, b > a) {
            let mut k = ia.frame_of(a as u64);
            let mut at = ia.start_of(k) as i64;
            while at < b && k < ia.count {
                let out = ia.rendered(&self.file, &self.bytes, k);
                // advance by the track's cadence, as `frame_of` locates frames
                let len = (ia.start_of(k + 1) as i64).saturating_sub(at);
                // the overlap of this frame [at, at + len) with the request [a, b)
                let (lo, hi) = (a.max(at), b.min(at + len));
                for (c, ch) in out.iter().enumerate() {
                    let (Some(dst), Some(s)) = (src.get_mut(c), ch.get((lo - at) as usize..(hi - at).max(lo - at) as usize)) else { continue };
                    if let Some(d) = dst.get_mut((lo - s0) as usize..(lo - s0) as usize + s.len()) {
                        d.copy_from_slice(s);
                    }
                }
                at += len.max(1);
                k += 1;
            }
        } else if b > a {
            let mut c0 = 0;
            for &(m, ti, first) in &self.atracks {
                let (bytes, file) =
                    if m == 0 { (&self.bytes, &self.file) } else { self.companions.get(m - 1).map(|c| (&c.0, &c.1)).ok_or(MediaError::NoStream("audio"))? };
                let pcm = file.read_pcm(bytes, ti, first + a as u64, (b - a) as usize).map_err(|e| MediaError::Decode(e.to_string()))?;
                for (k, p) in pcm.into_iter().enumerate() {
                    if let Some(dst) = src.get_mut(c0 + k) {
                        let at = (a - s0) as usize;
                        dst[at..at + p.len()].copy_from_slice(&p);
                    }
                }
                c0 += file.tracks.get(ti).and_then(|t| t.sound.as_ref()).map_or(1, |s| s.channels.max(1) as usize);
            }
        }
        if exact {
            out.channels = src;
            return Ok(out);
        }
        let frac = s0 as f64 - start as f64 * ratio;
        for k in 0..frames {
            let pos = k as f64 * ratio - frac;
            let i0 = pos.floor().max(0.0) as usize;
            let f = (pos - i0 as f64) as f32;
            for c in 0..ch {
                let a = src[c].get(i0).copied().unwrap_or(0.0);
                let b = src[c].get(i0 + 1).copied().unwrap_or(a);
                out.channels[c][k] = a + (b - a) * f;
            }
        }
        Ok(out)
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<Result<SharedSource, MediaError>> {
    if !sniff(&bytes[..bytes.len().min(70_000)]) {
        return None;
    }
    Some(MxfSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

/// [`cfa4_media::ReaderOpener`] for MXF.
pub fn reader_opener(name: &str, head: &[u8], reader: &cfa4_media::SharedReader) -> Option<Result<SharedSource, MediaError>> {
    if !sniff(head) {
        return None;
    }
    Some(MxfSource::open_reader(name, reader.clone()).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod iab_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecode_frames_at_the_media_rate() {
        let tc = Timecode { start: 90_000, rounded_base: 25, drop_frame: false };
        assert_eq!(start_frames(&tc, FrameRate::FPS_25), 90_000);
        // 50p media with a 25 fps timecode track
        assert_eq!(start_frames(&tc, FrameRate::FPS_50), 180_000);
        // 29.97 DF: the frame count is already at 29.97
        let df = Timecode { start: 107_892, rounded_base: 30, drop_frame: true };
        assert_eq!(start_frames(&df, FrameRate::FPS_29_97), 107_892);
        // audio-only media (default 23.976): one hour of DF timecode is one real hour minus 3.6 s
        let f = start_frames(&df, FrameRate::FPS_23_976);
        let secs = f as f64 * 1001.0 / 24000.0;
        assert!((secs - 107_892.0 * 1001.0 / 30000.0).abs() < 0.05, "{secs}");
    }

    /// SMPTE RP 224 colorimetry labels: BT.2020 / PQ is read, unknown labels are not.
    #[test]
    fn descriptor_colorimetry_is_read() {
        use cfa4_color::{Matrix, Primaries, Transfer};
        let label = |v: u8, group: u8, item: u8| Some(cfa4_mxf::Ul([0x06, 0x0E, 0x2B, 0x34, 4, 1, 1, v, 4, 1, 1, 1, group, item, 0, 0]));
        let p = cfa4_mxf::PictureInfo {
            transfer_characteristic: label(0x0D, 1, 0x0A),
            coding_equations: label(0x0D, 2, 6),
            color_primaries: label(0x0D, 3, 4),
            ..Default::default()
        };
        let s = descriptor_colorimetry(&p);
        assert_eq!((s.transfer, s.primaries, s.matrix), (Some(Transfer::Pq), Some(Primaries::Bt2020), Some(Matrix::Bt2020Ncl)));
        let p = cfa4_mxf::PictureInfo { transfer_characteristic: label(1, 1, 0x7F), ..Default::default() };
        assert_eq!(descriptor_colorimetry(&p), crate::j2k::Signalled::default());
    }

    fn picture(depth: u32, rgb: bool, planes: [Vec<u16>; 3], alpha: Option<Vec<u16>>) -> cfa4_mxf::uncompressed::Picture {
        cfa4_mxf::uncompressed::Picture { width: 2, height: 1, depth, subsampling: (1, 1), rgb, planes, alpha, display: (0, 0, 2, 1) }
    }

    #[test]
    fn uncompressed_rgb_frames() {
        use cfa4_frame::PixelData;
        // 8-bit R'G'B'A stays RGBA
        let f = raw_frame(picture(8, true, [vec![1, 2], vec![3, 4], vec![5, 6]], Some(vec![7, 8])), cfa4_color::ColorInfo::REC709);
        let PixelData::Rgba8(px) = &f.data else { panic!("RGBA expected") };
        assert_eq!(**px, vec![1, 3, 5, 7, 2, 4, 6, 8]);
        // without alpha: opaque
        let f = raw_frame(picture(8, true, [vec![1, 2], vec![3, 4], vec![5, 6]], None), cfa4_color::ColorInfo::REC709);
        let PixelData::Rgba8(px) = &f.data else { panic!("RGBA expected") };
        assert_eq!(px[3], 255);
        // 10-bit white and black become video-range Y'CbCr
        let f = raw_frame(picture(10, true, [vec![1023, 0], vec![1023, 0], vec![1023, 0]], None), cfa4_color::ColorInfo::REC709);
        let PixelData::Yuv16 { planes, bits: 10, chroma: cfa4_frame::Chroma::C444, .. } = &f.data else { panic!("10-bit 4:4:4 expected") };
        assert_eq!((planes[0][0], planes[0][1], planes[1][0], planes[2][1]), (940, 64, 512, 512));
        // under 8 bits widens to 8; the display rectangle crops
        let mut p = picture(4, false, [vec![15, 1], vec![8, 8], vec![8, 8]], None);
        p.display = (1, 0, 1, 1);
        let f = raw_frame(p, cfa4_color::ColorInfo::REC709);
        let PixelData::Yuv8 { planes, .. } = &f.data else { panic!("8-bit Y'CbCr expected") };
        assert_eq!((f.width, f.height, planes[0][0], planes[1][0]), (1, 1, 16, 128));
    }

    /// An Avid / P2 style clip: a picture file and two mono sound files sharing the material
    /// package, opened as one source with the sound files' channels in the order given.
    #[test]
    fn op_atom_files_of_one_clip_open_as_one_source() {
        use cfa4_mxf::{FrameInfo, MxfWriter, OpAtomPcm, PackageIds, Pattern, PictureCoding, PictureDesc, Rational, Umid, WriterConfig, write_opatom_pcm};
        let clip = PackageIds::from_seed("clip", "Clip");
        let mut cfg = WriterConfig::new(Pattern::OpAtom, Rational::new(25, 1), PackageIds { file: Umid::from_seed(b"v1"), ..clip.clone() });
        cfg.picture = Some(PictureDesc::new(PictureCoding::Vc3 { cid: 1272 }, 64, 36));
        let mut w = MxfWriter::new(std::io::Cursor::new(Vec::new()), cfg).unwrap();
        for i in 0..10u8 {
            let mut f = vec![0x00, 0x00, 0x02, 0x80, 0x01];
            f.extend(std::iter::repeat_n(i, 600));
            w.push_picture(&f, FrameInfo::intra()).unwrap();
        }
        let v: Arc<[u8]> = w.finish().unwrap().into_inner().into();
        let pcm = |seed: &[u8], value: i32| -> Arc<[u8]> {
            let ids = PackageIds { file: Umid::from_seed(seed), ..clip.clone() };
            let opts = OpAtomPcm { sample_rate: 48_000, bits: 16, channels: 1, edit_rate: Some(Rational::new(25, 1)), ids, timecode: None };
            write_opatom_pcm(&opts, &vec![value; 19_200]).unwrap().into()
        };
        let (a1, a2) = (pcm(b"a1", 8192), pcm(b"a2", -16384));
        let reader = |b: &Arc<[u8]>| Arc::new(cfa4_media::reader::MemReader(b.clone())) as cfa4_media::SharedReader;
        // asked for the second sound file; given in name order A01, A02, V01
        let src = MxfSource::open_group("clipA02.mxf", vec![reader(&a1), reader(&a2), reader(&v)], 1).unwrap();
        let info = src.info();
        assert_eq!(info.kind, MediaKind::Movie);
        assert_eq!(info.container, "MXF OP-Atom (3 files)");
        assert_eq!(info.video.as_ref().map(|v| (v.width, v.height)), Some((64, 36)));
        let a = info.audio().unwrap();
        assert_eq!((a.sample_rate, a.channels), (48_000, 2));
        assert_eq!(info.duration, FrameRate::FPS_25.tick_of(10));
        let buf = src.audio(100, 4, 48_000).unwrap();
        assert_eq!(buf.channels[0], vec![0.25; 4]);
        assert_eq!(buf.channels[1], vec![-0.5; 4]);
        // a damaged companion is skipped; a damaged asked-for file fails
        let junk: Arc<[u8]> = Arc::from(vec![0u8; 100]);
        let src = MxfSource::open_group("clipV01.mxf", vec![reader(&junk), reader(&v), reader(&a2)], 1).unwrap();
        assert_eq!(src.info().audio().unwrap().channels, 1);
        assert!(MxfSource::open_group("x.mxf", vec![reader(&junk), reader(&v)], 0).is_err());
        assert!(MxfSource::open_group("x.mxf", Vec::new(), 0).is_err());
    }
}
