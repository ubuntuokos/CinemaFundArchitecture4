//! USAC / xHE-AAC decoder (ISO/IEC 23003-3, audio object type 42).
//!
//! The decoding processes are ported from libxaac, the Android xHE-AAC decoder
//! (<https://android.googlesource.com/platform/external/libxaac>), and converted from its fixed-point
//! arithmetic to floating point:
//!
//! > Copyright (C) 2018 The Android Open Source Project. Licensed under the Apache License,
//! > Version 2.0 (the "License"); you may not use this file except in compliance with the License.
//! > You may obtain a copy of the License at <http://www.apache.org/licenses/LICENSE-2.0>.
//! > Unless required by applicable law or agreed to in writing, software distributed under the
//! > License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND,
//! > either express or implied. See the License for the specific language governing permissions
//! > and limitations under the License. Originally developed and contributed by Ittiam Systems Pvt.
//! > Ltd, Bangalore.
//!
//! Supported: FD core (arithmetic-coded spectrum, noise filling, TNS, all window sequences), M/S
//! and complex-prediction stereo, mono/stereo channel configurations, `AudioPreRoll`, extension
//! elements (skipped; the uniDrc payloads are collected for [`crate::drc`]). Not yet supported:
//! eSBR (`coreSbrFrameLengthIndex` 2–4), MPEG Surround 2-1-2, the LPD core, time-warped MDCT and
//! 768-sample frames; such streams are rejected with [`crate::Error::Unsupported`].

mod arith;
#[rustfmt::skip]
#[allow(clippy::unreadable_literal)]
mod arith_tables;
pub mod config;
mod fd;
mod lpd;
mod synth;

use cfa4_bitstream::BitReader;

pub use config::UsacConfig;
use config::{Element, ID_EXT_ELE_AUDIOPREROLL, ID_EXT_ELE_UNI_DRC};
use fd::{Bands, CplxState, FdChannel, FdParams, FdState, IcsInfo, StereoMask, Tns, TnsPresent};
use synth::{FilterBank, WinSeq};

use crate::tables::{swb_offsets_long, swb_offsets_short, table_index_for_rate, tns_max_bands};
use crate::{Error, Result};

/// Per-channel decoder state.
struct Channel {
    fd: FdState,
    last_seq: WinSeq,
    prev_kbd: bool,
    overlap: Vec<f32>,
}

/// Per-element state.
enum ElementState {
    Sce,
    Cpe(Box<CplxState>),
    Lfe,
    Ext,
}

/// Decoder of raw USAC access units (`UsacFrame()`) into planar f32 at the output rate.
pub struct UsacDecoder {
    cfg: UsacConfig,
    n: usize,
    long: Bands,
    short: Bands,
    bank: FilterBank,
    channels: Vec<Channel>,
    elements: Vec<ElementState>,
    /// uniDrcGain() payloads of the last access unit.
    drc_gain_payloads: Vec<Vec<u8>>,
    preroll_drc_gain_payloads: Vec<Option<Vec<u8>>>,
    /// An AudioPreRoll configuration was already applied.
    preroll_seen: bool,
}

impl UsacDecoder {
    /// Create a decoder from an `AudioSpecificConfig` with audio object type 42.
    pub fn new(asc: &[u8]) -> Result<UsacDecoder> {
        Self::from_config(UsacConfig::from_asc(asc)?)
    }

    pub fn from_config(cfg: UsacConfig) -> Result<UsacDecoder> {
        if cfg.core_sbr_frame_length_index >= 2 {
            return Err(Error::Unsupported("USAC with eSBR (coreSbrFrameLengthIndex 2-4)"));
        }
        let n = cfg.core_frame_length();
        if n != 1024 {
            return Err(Error::Unsupported("USAC 768-sample frames"));
        }
        for e in &cfg.elements {
            match e {
                Element::Sce { tw_mdct: true, .. } | Element::Cpe { tw_mdct: true, .. } => return Err(Error::Unsupported("USAC time-warped MDCT")),
                Element::Cpe { stereo_config_index, .. } if *stereo_config_index > 0 => return Err(Error::Unsupported("USAC MPEG Surround 2-1-2")),
                _ => {}
            }
        }
        if cfg.core_channels() != cfg.num_out_channels {
            return Err(Error::Unsupported("USAC element layout that differs from the output channels"));
        }
        let ti = table_index_for_rate(cfg.core_sample_rate());
        let long = Bands { offsets: swb_offsets_long(ti), tns_max: tns_max_bands(ti, false) };
        let short = Bands { offsets: swb_offsets_short(ti), tns_max: tns_max_bands(ti, true) };
        let mut channels = Vec::new();
        let mut elements = Vec::new();
        for e in &cfg.elements {
            let (st, seeds): (ElementState, &[u32]) = match e {
                Element::Sce { .. } => (ElementState::Sce, &[0x3039]),
                Element::Cpe { .. } => (ElementState::Cpe(Box::new(CplxState::new(n))), &[0x3039, 0x10932]),
                Element::Lfe => (ElementState::Lfe, &[0x3039]),
                Element::Ext(_) => (ElementState::Ext, &[]),
            };
            for &seed in seeds {
                channels.push(Channel { fd: FdState { arith: Default::default(), seed }, last_seq: WinSeq::OnlyLong, prev_kbd: false, overlap: vec![0.0; n] });
            }
            elements.push(st);
        }
        Ok(UsacDecoder {
            bank: FilterBank::new(n),
            cfg,
            n,
            long,
            short,
            channels,
            elements,
            drc_gain_payloads: Vec::new(),
            preroll_drc_gain_payloads: Vec::new(),
            preroll_seen: false,
        })
    }

    pub fn config(&self) -> &UsacConfig {
        &self.cfg
    }
    pub fn sample_rate(&self) -> u32 {
        self.cfg.sample_rate
    }
    pub fn channels(&self) -> usize {
        self.cfg.num_out_channels
    }
    /// Output samples per channel of each access unit.
    pub fn frame_length(&self) -> usize {
        self.cfg.output_frame_length()
    }
    /// `uniDrcGain()` payloads carried by the last decoded access unit.
    pub fn drc_gain_payloads(&self) -> &[Vec<u8>] {
        &self.drc_gain_payloads
    }

    /// `uniDrcGain()` payload of each pre-roll access unit decoded (and discarded) by the last
    /// [`Self::decode`] call, in decoding order; the DRC decoder must see them to stay in step.
    pub fn preroll_drc_gain_payloads(&self) -> &[Option<Vec<u8>>] {
        &self.preroll_drc_gain_payloads
    }

    /// Clear all signal state (e.g. after a seek; the next access unit should be an independent
    /// frame).
    pub fn reset(&mut self) {
        self.preroll_seen = false;
        for c in &mut self.channels {
            c.overlap.fill(0.0);
            c.last_seq = WinSeq::OnlyLong;
            c.prev_kbd = false;
            c.fd.arith = Default::default();
        }
        for e in &mut self.elements {
            if let ElementState::Cpe(cs) = e {
                **cs = CplxState::new(self.n);
            }
        }
    }

    /// Decode one access unit into planar PCM (±1.0 full scale), [`Self::frame_length`] samples
    /// per channel.
    pub fn decode(&mut self, au: &[u8]) -> Result<Vec<Vec<f32>>> {
        let mut pre = Vec::new();
        // AudioPreRoll of the first (immediate playout) frame: decode the pre-roll access units to
        // prime the decoder and discard their output.
        if !self.preroll_seen
            && let Some(prerolls) = self.preroll_units(au)?
        {
            self.preroll_seen = true;
            for p in prerolls {
                self.decode_frame(&p)?;
                pre.push(self.drc_gain_payloads.first().cloned());
            }
        }
        let out = self.decode_frame(au)?;
        self.preroll_drc_gain_payloads = pre;
        Ok(out)
    }

    /// The pre-roll access units of an `AudioPreRoll()` in the first extension element, if any.
    fn preroll_units(&self, au: &[u8]) -> Result<Option<Vec<Vec<u8>>>> {
        let Some(Element::Ext(x)) = self.cfg.elements.first() else { return Ok(None) };
        if x.ext_type != ID_EXT_ELE_AUDIOPREROLL {
            return Ok(None);
        }
        let mut br = BitReader::new(au);
        br.skip(1)?; // usacIndependencyFlag
        let Some(payload) = read_ext_payload(&mut br, x.default_length, x.payload_frag)? else { return Ok(None) };
        let mut br = BitReader::new(&payload);
        let config_len = config::escaped_value(&mut br, 4, 4, 8)? as usize;
        br.skip(config_len * 8)?;
        let _apply_crossfade = br.read_flag()?;
        br.skip(1)?; // reserved
        let num = config::escaped_value(&mut br, 2, 4, 0)? as usize;
        let mut units = Vec::with_capacity(num);
        for _ in 0..num {
            let len = config::escaped_value(&mut br, 16, 16, 0)? as usize;
            if len * 8 > br.bits_left() {
                return Err(Error::Bitstream("AudioPreRoll access unit exceeds the payload"));
            }
            let mut v = Vec::with_capacity(len);
            for _ in 0..len {
                v.push(br.read_bits(8)? as u8);
            }
            units.push(v);
        }
        Ok(Some(units))
    }

    fn decode_frame(&mut self, au: &[u8]) -> Result<Vec<Vec<f32>>> {
        let mut br = BitReader::new(au);
        let indep = br.read_flag()?;
        self.drc_gain_payloads.clear();
        let mut out: Vec<Vec<f32>> = Vec::with_capacity(self.channels.len());
        let mut ch = 0;
        for ei in 0..self.cfg.elements.len() {
            match &self.cfg.elements[ei] {
                Element::Sce { noise_filling, .. } => {
                    let nf = *noise_filling;
                    out.push(self.decode_single(&mut br, ch, nf, indep, false)?);
                    ch += 1;
                }
                Element::Lfe => {
                    out.push(self.decode_single(&mut br, ch, false, indep, true)?);
                    ch += 1;
                }
                Element::Cpe { noise_filling, .. } => {
                    let nf = *noise_filling;
                    let [l, r] = self.decode_pair(&mut br, ei, ch, nf, indep)?;
                    out.push(l);
                    out.push(r);
                    ch += 2;
                }
                Element::Ext(x) => {
                    let (t, dl, frag) = (x.ext_type, x.default_length, x.payload_frag);
                    if let Some(p) = read_ext_payload(&mut br, dl, frag)?
                        && t == ID_EXT_ELE_UNI_DRC
                    {
                        self.drc_gain_payloads.push(p);
                    }
                }
            }
        }
        Ok(out)
    }

    /// SCE or LFE.
    fn decode_single(&mut self, br: &mut BitReader, ch: usize, nf: bool, indep: bool, lfe: bool) -> Result<Vec<f32>> {
        let core_mode = if lfe { false } else { br.read_flag()? };
        if core_mode {
            return Err(Error::Unsupported("USAC LPD core"));
        }
        let tns_present = if !lfe && br.read_flag()? { TnsPresent::Own } else { TnsPresent::No };
        let p = FdParams { n: self.n, long: &self.long, short: &self.short, noise_filling: nf, indep, tw_mdct: false };
        let c = self.channels.get_mut(ch).ok_or(Error::Bitstream("channel index"))?;
        let mut fdc = fd::fd_channel_stream(br, &p, &mut c.fd, c.last_seq, None, tns_present, None, None)?;
        if lfe && fdc.info.seq != WinSeq::OnlyLong {
            return Err(Error::Bitstream("LFE with a window sequence other than ONLY_LONG"));
        }
        if fdc.fac.is_some() {
            return Err(Error::Unsupported("USAC LPD core (FAC)"));
        }
        if let Some(t) = &fdc.tns {
            let (bands, nw) = self.bands(&fdc.info);
            t.apply(&mut fdc.spec, &fdc.info, bands, nw);
        }
        Ok(self.synth(ch, &fdc))
    }

    fn bands(&self, info: &IcsInfo) -> (&Bands, usize) {
        if info.seq.is_short() { (&self.short, self.n / 8) } else { (&self.long, self.n) }
    }

    fn synth(&mut self, ch: usize, fdc: &FdChannel) -> Vec<f32> {
        let mut out = vec![0f32; self.n];
        if let Some(c) = self.channels.get_mut(ch) {
            self.bank.synth(&fdc.spec, fdc.info.seq, fdc.info.kbd, c.prev_kbd, &mut c.overlap, &mut out);
            c.prev_kbd = fdc.info.kbd;
            c.last_seq = fdc.info.seq;
        }
        out
    }

    fn decode_pair(&mut self, br: &mut BitReader, ei: usize, ch: usize, nf: bool, indep: bool) -> Result<[Vec<f32>; 2]> {
        let core0 = br.read_flag()?;
        let core1 = br.read_flag()?;
        if core0 || core1 {
            return Err(Error::Unsupported("USAC LPD core"));
        }
        let n = self.n;
        let (nbl, nbs) = (self.long.offsets.len() - 1, self.short.offsets.len() - 1);
        let tns_active = br.read_flag()?;
        let common_window = br.read_flag()?;
        let mut common = None;
        let mut max_sfb_r = None;
        let mut mask = StereoMask::None;
        let mut max_sfb_ste = 0;
        let ElementState::Cpe(cs) = self.elements.get_mut(ei).ok_or(Error::Bitstream("element index"))? else {
            return Err(Error::Bitstream("element state mismatch"));
        };
        let last = self.channels.get(ch).map(|c| c.last_seq).unwrap_or_default();
        if common_window {
            let info = IcsInfo::parse(br, last, nbl, nbs)?;
            let common_max_sfb = br.read_flag()?;
            let mr = if common_max_sfb {
                info.max_sfb
            } else {
                let v = br.read_bits(if info.seq.is_short() { 4 } else { 6 })? as usize;
                v.min(if info.seq.is_short() { nbs } else { nbl })
            };
            max_sfb_ste = info.max_sfb.max(mr);
            let nb = if info.seq.is_short() { nbs } else { nbl };
            mask = fd::read_stereo_mask(br, &info, max_sfb_ste, nb, indep, cs)?;
            common = Some(info);
            max_sfb_r = Some(mr);
        } else {
            cs.reset_alpha();
        }
        let mut tns_present = [TnsPresent::No; 2];
        let mut common_tns: Option<Tns> = None;
        let mut tns_on_lr = false;
        if tns_active {
            let common_tns_flag = if common_window { br.read_flag()? } else { false };
            tns_on_lr = br.read_flag()?;
            if common_tns_flag {
                let info = common.ok_or(Error::Bitstream("common TNS without common window"))?;
                let nb = if info.seq.is_short() { nbs } else { nbl };
                common_tns = Some(Tns::parse(br, &info, nb)?);
                tns_present = [TnsPresent::Common; 2];
            } else if br.read_flag()? {
                tns_present = [TnsPresent::Own; 2];
            } else {
                let r = br.read_flag()?;
                tns_present = if r { [TnsPresent::No, TnsPresent::Own] } else { [TnsPresent::Own, TnsPresent::No] };
            }
        }
        let p = FdParams { n, long: &self.long, short: &self.short, noise_filling: nf, indep, tw_mdct: false };
        let (c0, c1) = match self.channels.get_mut(ch..ch + 2) {
            Some([a, b]) => (a, b),
            _ => return Err(Error::Bitstream("channel index")),
        };
        let mut l = fd::fd_channel_stream(br, &p, &mut c0.fd, c0.last_seq, common, tns_present[0], common_tns, None)?;
        let mut r = fd::fd_channel_stream(br, &p, &mut c1.fd, c1.last_seq, common, tns_present[1], common_tns, max_sfb_r)?;
        if l.fac.is_some() || r.fac.is_some() {
            return Err(Error::Unsupported("USAC LPD core (FAC)"));
        }
        let prev_kbd = c0.prev_kbd;
        // previous frame's downmix, from the saved L/R spectra (ixheaacd_cplx_prev_mdct_dmx)
        let (bands_l, nwl) = if l.info.seq.is_short() { (&self.short, n / 8) } else { (&self.long, n) };
        let ElementState::Cpe(cs) = self.elements.get_mut(ei).ok_or(Error::Bitstream("element index"))? else {
            return Err(Error::Bitstream("element state mismatch"));
        };
        let pred_dir = matches!(&mask, StereoMask::Cplx(cp) if cp.pred_dir);
        let f = if pred_dir { -1.0 } else { 1.0 };
        let offs = n - nwl;
        let dmx_prev: Vec<f32> = (0..nwl).map(|i| 0.5 * (cs.save[0][offs + i] + f * cs.save[1][offs + i])).collect();
        if !tns_on_lr {
            for c in [&mut l, &mut r] {
                if let Some(t) = &c.tns {
                    let (b, nw) = if c.info.seq.is_short() { (&self.short, n / 8) } else { (&self.long, n) };
                    t.apply(&mut c.spec, &c.info, b, nw);
                }
            }
        }
        match &mask {
            StereoMask::None => {}
            StereoMask::Ms(used) => fd::apply_ms(&mut l.spec, &mut r.spec, &l.info, bands_l, nwl, used, max_sfb_ste),
            StereoMask::Cplx(cp) => fd::apply_cplx_pred(&mut l.spec, &mut r.spec, &l.info, bands_l, n, cp, &dmx_prev, prev_kbd),
        }
        if tns_on_lr {
            for c in [&mut l, &mut r] {
                if let Some(t) = &c.tns {
                    let (b, nw) = if c.info.seq.is_short() { (&self.short, n / 8) } else { (&self.long, n) };
                    t.apply(&mut c.spec, &c.info, b, nw);
                }
            }
        }
        // ixheaacd_usac_cplx_save_prev: keep the last window of this frame
        let so = n - nwl;
        cs.save[0][so..].copy_from_slice(&l.spec[so..]);
        cs.save[1][so..].copy_from_slice(&r.spec[so..]);
        let a = self.synth(ch, &l);
        let b = self.synth(ch + 1, &r);
        Ok([a, b])
    }
}

/// `UsacExtElement()`: returns the payload when present (fragmentation is not reassembled; a
/// fragmented payload is returned as its fragments).
fn read_ext_payload(br: &mut BitReader, default_length: u32, frag: bool) -> Result<Option<Vec<u8>>> {
    if !br.read_flag()? {
        return Ok(None);
    }
    let len = if br.read_flag()? {
        default_length as usize
    } else {
        let mut l = br.read_bits(8)? as usize;
        if l == 255 {
            l += br.read_bits(16)? as usize;
            l -= 2;
        }
        l
    };
    if len == 0 {
        return Ok(Some(Vec::new()));
    }
    if frag {
        br.skip(2)?; // usacExtElementStart, usacExtElementStop
    }
    if len * 8 > br.bits_left() {
        return Err(Error::Bitstream("USAC extension payload exceeds the access unit"));
    }
    let mut v = Vec::with_capacity(len);
    for _ in 0..len {
        v.push(br.read_bits(8)? as u8);
    }
    Ok(Some(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// Random access units and corrupted configurations decode to errors or finite PCM, never a
    /// panic.
    #[test]
    fn hostile_input_does_not_panic() {
        let ascs = [
            hex("f94643222cc04c0085a00113840020000250011972c000"),
            hex("f94623213cc0d3c10000000a47a522008240c0020908009dfc010000c0085a00113840020000244011732c00"),
        ];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for i in 0..400 {
            let mut asc = ascs[i % 2].clone();
            if i >= 2 {
                let k = rand() as usize;
                let at = 2 + k % (asc.len() - 2);
                if let Some(b) = asc.get_mut(at) {
                    *b ^= 1 << (k % 8);
                }
            }
            let Ok(mut dec) = UsacDecoder::new(&asc) else { continue };
            for _ in 0..8 {
                let au: Vec<u8> = (0..rand() % 600).map(|_| rand() as u8).collect();
                if let Ok(pcm) = dec.decode(&au) {
                    assert!(pcm.iter().flatten().all(|s| s.is_finite()));
                }
            }
        }
    }
}
