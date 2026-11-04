//! `UsacConfig()` (ISO/IEC 23003-3 §5.2) carried in an `AudioSpecificConfig` with audio object
//! type 42.
//!
//! Syntax follows libxaac `ixheaacd_init_config.c` (Apache-2.0, see `super` for the notice).

use cfa4_bitstream::BitReader;

use crate::{Error, Result};

/// `usacSamplingFrequencyIndex` table (index 0x1f = explicit 24-bit rate).
const USAC_RATES: [u32; 31] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350, 0, 0, 57600, 51200, 40000, 38400, 34150, 28800, 25600, 20000,
    19200, 17075, 14400, 12800, 9600, 0, 0, 0,
];

/// Audio object type of USAC (xHE-AAC).
pub const AOT_USAC: u8 = 42;

/// `usacExtElementType`.
pub const ID_EXT_ELE_FILL: u32 = 0;
pub const ID_EXT_ELE_MPEGS: u32 = 1;
pub const ID_EXT_ELE_SAOC: u32 = 2;
pub const ID_EXT_ELE_AUDIOPREROLL: u32 = 3;
pub const ID_EXT_ELE_UNI_DRC: u32 = 4;

/// `usacConfigExtType`.
pub const ID_CONFIG_EXT_FILL: u32 = 0;
pub const ID_CONFIG_EXT_LOUDNESS_INFO: u32 = 2;

/// `SbrConfig()` defaults of an element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SbrConfig {
    pub harmonic_sbr: bool,
    pub inter_tes: bool,
    pub pvc: bool,
    pub start_freq: u8,
    pub stop_freq: u8,
    pub header_extra1: bool,
    pub header_extra2: bool,
    pub freq_scale: u8,
    pub alter_scale: bool,
    pub noise_bands: u8,
    pub limiter_bands: u8,
    pub limiter_gains: u8,
    pub interpol_freq: bool,
    pub smoothing_mode: bool,
}

/// `Mps212Config()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mps212Config {
    pub freq_res: u8,
    pub fixed_gain_dmx: u8,
    pub temp_shape_config: u8,
    pub decorr_config: u8,
    pub high_rate_mode: bool,
    pub phase_coding: bool,
    pub ott_bands_phase: Option<u8>,
    pub residual_bands: u8,
    pub pseudo_lr: bool,
    pub env_quant_mode: bool,
}

/// One element of `UsacDecoderConfig()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Element {
    Sce { tw_mdct: bool, noise_filling: bool, sbr: Option<SbrConfig> },
    Cpe { tw_mdct: bool, noise_filling: bool, sbr: Option<SbrConfig>, stereo_config_index: u8, mps: Option<Mps212Config> },
    Lfe,
    Ext(ExtElementConfig),
}

/// `UsacExtElementConfig()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtElementConfig {
    pub ext_type: u32,
    /// `usacExtElementDefaultLength` (0 when not signalled).
    pub default_length: u32,
    pub payload_frag: bool,
    /// The element's configuration bytes (e.g. `uniDrcConfig()`).
    pub config: Vec<u8>,
}

/// Parsed `UsacConfig()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsacConfig {
    /// Output sampling rate.
    pub sample_rate: u32,
    pub core_sbr_frame_length_index: u8,
    pub channel_configuration_index: u8,
    pub num_out_channels: usize,
    pub elements: Vec<Element>,
    /// `usacConfigExtension()` entries: (type, payload).
    pub config_extensions: Vec<(u32, Vec<u8>)>,
}

/// `escapedValue(n1, n2, n3)`.
pub fn escaped_value(br: &mut BitReader, n1: u32, n2: u32, n3: u32) -> Result<u32> {
    let mut v = br.read_bits(n1)?;
    if v == (1 << n1) - 1 {
        let add = if n2 > 0 { br.read_bits(n2)? } else { 0 };
        v += add;
        if n2 > 0 && add == (1 << n2) - 1 && n3 > 0 {
            v += br.read_bits(n3)?;
        }
    }
    Ok(v)
}

fn sbr_config(br: &mut BitReader) -> Result<SbrConfig> {
    let mut c = SbrConfig {
        harmonic_sbr: br.read_flag()?,
        inter_tes: br.read_flag()?,
        pvc: br.read_flag()?,
        start_freq: br.read_bits(4)? as u8,
        stop_freq: br.read_bits(4)? as u8,
        header_extra1: br.read_flag()?,
        header_extra2: br.read_flag()?,
        ..Default::default()
    };
    // SbrDfltHeader() defaults
    c.freq_scale = 2;
    c.alter_scale = true;
    c.noise_bands = 2;
    c.limiter_bands = 2;
    c.limiter_gains = 2;
    c.interpol_freq = true;
    c.smoothing_mode = true;
    if c.header_extra1 {
        c.freq_scale = br.read_bits(2)? as u8;
        c.alter_scale = br.read_flag()?;
        c.noise_bands = br.read_bits(2)? as u8;
    }
    if c.header_extra2 {
        c.limiter_bands = br.read_bits(2)? as u8;
        c.limiter_gains = br.read_bits(2)? as u8;
        c.interpol_freq = br.read_flag()?;
        c.smoothing_mode = br.read_flag()?;
    }
    Ok(c)
}

fn mps212_config(br: &mut BitReader, stereo_config_index: u8) -> Result<Mps212Config> {
    let mut c = Mps212Config {
        freq_res: br.read_bits(3)? as u8,
        fixed_gain_dmx: br.read_bits(3)? as u8,
        temp_shape_config: br.read_bits(2)? as u8,
        decorr_config: br.read_bits(2)? as u8,
        high_rate_mode: br.read_flag()?,
        phase_coding: br.read_flag()?,
        ..Default::default()
    };
    if c.decorr_config > 2 {
        return Err(Error::InvalidConfig("MPS212 bsDecorrConfig out of range"));
    }
    if br.read_flag()? {
        c.ott_bands_phase = Some(br.read_bits(5)? as u8);
    }
    if stereo_config_index > 1 {
        c.residual_bands = br.read_bits(5)? as u8;
        c.pseudo_lr = br.read_flag()?;
    }
    if c.temp_shape_config == 2 {
        c.env_quant_mode = br.read_flag()?;
    }
    Ok(c)
}

impl UsacConfig {
    /// Parse a complete `AudioSpecificConfig` whose audio object type is 42.
    pub fn from_asc(asc: &[u8]) -> Result<UsacConfig> {
        let mut br = BitReader::new(asc);
        let mut aot = br.read_bits(5)? as u8;
        if aot == 31 {
            aot = 32 + br.read_bits(6)? as u8;
        }
        if aot != AOT_USAC {
            return Err(Error::Unsupported("not a USAC AudioSpecificConfig"));
        }
        if br.read_bits(4)? == 15 {
            br.skip(24)?;
        }
        br.skip(4)?; // channelConfiguration (UsacConfig carries its own)
        Self::parse(&mut br)
    }

    /// Parse `UsacConfig()` (also used for the configuration inside an `AudioPreRoll()`).
    pub fn parse(br: &mut BitReader) -> Result<UsacConfig> {
        let sfi = br.read_bits(5)? as usize;
        let sample_rate = if sfi == 0x1f { br.read_bits(24)? } else { USAC_RATES.get(sfi).copied().unwrap_or(0) };
        if sample_rate == 0 || sample_rate > 96_000 {
            return Err(Error::InvalidConfig("USAC sampling frequency"));
        }
        let core_sbr_frame_length_index = br.read_bits(3)? as u8;
        if core_sbr_frame_length_index > 4 {
            return Err(Error::InvalidConfig("reserved coreSbrFrameLengthIndex"));
        }
        let channel_configuration_index = br.read_bits(5)? as u8;
        let num_out_channels = match channel_configuration_index {
            0 => {
                let n = escaped_value(br, 5, 8, 16)? as usize;
                if n == 0 || n > 64 {
                    return Err(Error::InvalidConfig("USAC output channel count"));
                }
                br.skip(5 * n)?; // bsOutputChannelPos
                n
            }
            1 => 1,
            2 | 8 => 2,
            _ => return Err(Error::Unsupported("USAC channel configurations above stereo")),
        };
        let sbr = core_sbr_frame_length_index >= 2;
        let num_elements = escaped_value(br, 4, 8, 16)? as usize + 1;
        if num_elements > 64 {
            return Err(Error::InvalidConfig("too many USAC elements"));
        }
        let mut elements = Vec::with_capacity(num_elements);
        for _ in 0..num_elements {
            let el = match br.read_bits(2)? {
                0 => {
                    let tw_mdct = br.read_flag()?;
                    let noise_filling = br.read_flag()?;
                    let sbr = if sbr { Some(sbr_config(br)?) } else { None };
                    Element::Sce { tw_mdct, noise_filling, sbr }
                }
                1 => {
                    let tw_mdct = br.read_flag()?;
                    let noise_filling = br.read_flag()?;
                    let (sbr, stereo_config_index) = if sbr { (Some(sbr_config(br)?), br.read_bits(2)? as u8) } else { (None, 0) };
                    let mps = if stereo_config_index > 0 { Some(mps212_config(br, stereo_config_index)?) } else { None };
                    Element::Cpe { tw_mdct, noise_filling, sbr, stereo_config_index, mps }
                }
                2 => Element::Lfe,
                _ => {
                    let ext_type = escaped_value(br, 4, 8, 16)?;
                    let len = escaped_value(br, 4, 8, 16)? as usize;
                    if len > 4096 {
                        return Err(Error::InvalidConfig("USAC extension element config too long"));
                    }
                    let default_length = if br.read_flag()? { escaped_value(br, 8, 16, 0)? + 1 } else { 0 };
                    let payload_frag = br.read_flag()?;
                    let mut config = Vec::with_capacity(len);
                    for _ in 0..len {
                        config.push(br.read_bits(8)? as u8);
                    }
                    Element::Ext(ExtElementConfig { ext_type, default_length, payload_frag, config })
                }
            };
            elements.push(el);
        }
        let mut config_extensions = Vec::new();
        if br.read_flag()? {
            let n = escaped_value(br, 2, 4, 8)? as usize + 1;
            for _ in 0..n {
                let t = escaped_value(br, 4, 8, 16)?;
                let len = escaped_value(br, 4, 8, 16)? as usize;
                if len > 4096 {
                    return Err(Error::InvalidConfig("USAC config extension too long"));
                }
                let mut v = Vec::with_capacity(len);
                for _ in 0..len {
                    v.push(br.read_bits(8)? as u8);
                }
                config_extensions.push((t, v));
            }
        }
        let cfg = UsacConfig { sample_rate, core_sbr_frame_length_index, channel_configuration_index, num_out_channels, elements, config_extensions };
        if cfg.core_channels() == 0 {
            return Err(Error::InvalidConfig("USAC config without audio elements"));
        }
        Ok(cfg)
    }

    /// Core coder frame length (`coreCoderFrameLength`): 768 or 1024.
    pub fn core_frame_length(&self) -> usize {
        if matches!(self.core_sbr_frame_length_index, 0 | 2) { 768 } else { 1024 }
    }

    /// Output samples per access unit per channel.
    pub fn output_frame_length(&self) -> usize {
        match self.core_sbr_frame_length_index {
            0 => 768,
            1 => 1024,
            2 | 3 => 2048,
            _ => 4096,
        }
    }

    /// Sampling rate of the core coder.
    pub fn core_sample_rate(&self) -> u32 {
        match self.core_sbr_frame_length_index {
            2 => self.sample_rate * 3 / 8,
            3 => self.sample_rate / 2,
            4 => self.sample_rate / 4,
            _ => self.sample_rate,
        }
    }

    /// Channels produced by the audio elements.
    pub fn core_channels(&self) -> usize {
        self.elements
            .iter()
            .map(|e| match e {
                Element::Sce { .. } | Element::Lfe => 1,
                Element::Cpe { .. } => 2,
                Element::Ext(_) => 0,
            })
            .sum()
    }

    /// The `loudnessInfoSet()` payload of the config extension, if any.
    pub fn loudness_info(&self) -> Option<&[u8]> {
        self.config_extensions.iter().find(|(t, _)| *t == ID_CONFIG_EXT_LOUDNESS_INFO).map(|(_, v)| v.as_slice())
    }

    /// The `uniDrcConfig()` payload of the uniDrc extension element, if any.
    pub fn uni_drc_config(&self) -> Option<&[u8]> {
        self.elements.iter().find_map(|e| match e {
            Element::Ext(x) if x.ext_type == ID_EXT_ELE_UNI_DRC => Some(x.config.as_slice()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// Configurations of the Fraunhofer IIS xHE-AAC test streams.
    #[test]
    fn parses_fraunhofer_configs() {
        let c = UsacConfig::from_asc(&hex("f94643222cc04c0085a00113840020000250011972c000")).unwrap();
        assert_eq!((c.sample_rate, c.core_frame_length(), c.num_out_channels), (48_000, 1024, 2));
        assert_eq!(c.elements.len(), 3);
        assert!(matches!(c.elements[0], Element::Ext(ExtElementConfig { ext_type: ID_EXT_ELE_AUDIOPREROLL, .. })));
        assert!(matches!(c.elements[1], Element::Cpe { stereo_config_index: 0, noise_filling: false, .. }));
        assert_eq!(c.loudness_info().map(<[u8]>::len), Some(13));

        let d = UsacConfig::from_asc(&hex("f94623213cc0d3c10000000a47a522008240c0020908009dfc010000c0085a00113840020000244011732c00")).unwrap();
        assert_eq!(d.num_out_channels, 1);
        assert_eq!(d.uni_drc_config().map(<[u8]>::len), Some(19));
        assert!(matches!(d.elements[2], Element::Sce { .. }));
    }
}
