//! `Parameters`, quantization table sets and the configuration record (RFC 9043 sections 4.1-4.3).

use crate::error::{Error, Result, ensure};
use crate::range::{RangeDecoder, StateTables};
use crate::tables::{CONTEXT_SIZE, MAX_CONTEXT_INPUTS, MAX_CONTEXTS, MAX_QUANT_TABLE_SETS};

/// One quantization table set: five 256-entry tables (section 3.4) and its context count.
#[derive(Clone, Debug)]
pub(crate) struct QuantTableSet {
    pub tables: [[i32; 256]; MAX_CONTEXT_INPUTS],
    pub context_count: usize,
    /// `initial_state[ i ]` (section 4.2.15), one row per context; `None`: all 128.
    pub initial_states: Option<Vec<[u8; CONTEXT_SIZE]>>,
}

/// The decoded `Parameters` (section 4.2).
#[derive(Clone, Debug)]
pub(crate) struct Parameters {
    pub version: u32,
    /// Read for completeness; no decoding behaviour depends on it.
    #[allow(dead_code)]
    pub micro_version: u32,
    pub coder_type: u32,
    /// The state tables the slices use (custom ones for `coder_type == 2`).
    pub states: StateTables,
    pub colorspace_type: u32,
    pub bits_per_raw_sample: u32,
    pub chroma_planes: bool,
    pub log2_h_chroma_subsample: u32,
    pub log2_v_chroma_subsample: u32,
    pub extra_plane: bool,
    pub num_h_slices: u32,
    pub num_v_slices: u32,
    pub quant_table_sets: Vec<QuantTableSet>,
    pub ec: u32,
    pub intra: u32,
}

impl Parameters {
    /// Section 4.7.1.
    pub fn primary_color_count(&self) -> usize {
        1 + if self.chroma_planes { 2 } else { 0 } + usize::from(self.extra_plane)
    }

    /// Section 4.6.5.
    pub fn quant_table_set_index_count(&self) -> usize {
        1 + usize::from(self.chroma_planes || self.version <= 3) + usize::from(self.extra_plane)
    }

    /// The context group (quantization table set index slot) of plane `p` (section 3.6): Cb and
    /// Cr share one.
    pub fn plane_group(&self, p: usize) -> usize {
        match p {
            0 => 0,
            1 | 2 if self.chroma_planes => 1,
            _ => {
                if self.version <= 3 || self.chroma_planes {
                    2
                } else {
                    1
                }
            }
        }
    }
}

fn ur(rc: &mut RangeDecoder, t: &StateTables, st: &mut [u8; CONTEXT_SIZE]) -> Result<u32> {
    u32::try_from(rc.get_symbol(t, st, false)).map_err(|_| Error::Invalid("negative unsigned symbol".into()))
}

/// Section 4.1: `QuantizationTableSet( i )`.
fn read_quant_table_set(rc: &mut RangeDecoder, t: &StateTables) -> Result<QuantTableSet> {
    let mut tables = [[0i32; 256]; MAX_CONTEXT_INPUTS];
    let mut scale: i64 = 1;
    for table in tables.iter_mut() {
        // each table has its own initial states, all 128
        let mut st = [128u8; CONTEXT_SIZE];
        let mut v: i64 = 0;
        let mut k = 0usize;
        while k < 128 {
            let len = ur(rc, t, &mut st)? as usize + 1;
            ensure!(len <= 128 - k, "quantization table run of {len} past entry 128");
            for e in table.iter_mut().skip(k).take(len) {
                *e = i32::try_from(scale * v).map_err(|_| Error::Invalid("quantization value overflow".into()))?;
            }
            k += len;
            v += 1;
        }
        for k in 1..128 {
            table[256 - k] = -table[k];
        }
        table[128] = -table[127];
        scale = scale.saturating_mul(2 * v - 1);
        ensure!(scale <= 2 * MAX_CONTEXTS as i64, "too many contexts");
    }
    let context_count = usize::try_from((scale + 1) / 2).map_err(|_| Error::Invalid("context count".into()))?;
    ensure!(context_count <= MAX_CONTEXTS, "context_count {context_count} > {MAX_CONTEXTS}");
    Ok(QuantTableSet { tables, context_count, initial_states: None })
}

/// Section 4.2: `Parameters( )`, read with the default state table.
pub(crate) fn read_parameters(rc: &mut RangeDecoder) -> Result<Parameters> {
    let t = StateTables::default_tables();
    // "Parameters" has its own initial states, all 128 (one set for every field)
    let mut st = [128u8; CONTEXT_SIZE];
    let version = ur(rc, &t, &mut st)?;
    match version {
        0 | 1 | 3 => {}
        2 => return Err(Error::Unsupported("FFV1 version 2 (experimental)".into())),
        v => return Err(Error::Unsupported(format!("FFV1 version {v}"))),
    }
    let micro_version = if version >= 3 { ur(rc, &t, &mut st)? } else { 0 };
    let coder_type = ur(rc, &t, &mut st)?;
    ensure!(coder_type <= 2, "coder_type {coder_type}");
    let mut delta = [0i32; 256];
    if coder_type > 1 {
        for d in delta.iter_mut().skip(1) {
            *d = rc.get_symbol(&t, &mut st, true);
        }
    }
    let states = StateTables::new(&delta).ok_or_else(|| Error::Invalid("state transition delta out of range".into()))?;
    let colorspace_type = ur(rc, &t, &mut st)?;
    if colorspace_type > 1 {
        return Err(Error::Unsupported(format!("colorspace_type {colorspace_type}")));
    }
    let mut bits_per_raw_sample = if version >= 1 { ur(rc, &t, &mut st)? } else { 8 };
    if bits_per_raw_sample == 0 {
        bits_per_raw_sample = 8;
    }
    ensure!(bits_per_raw_sample <= 16, "bits_per_raw_sample {bits_per_raw_sample}");
    let chroma_planes = rc.get_rac(&t, &mut st[0]);
    let log2_h_chroma_subsample = ur(rc, &t, &mut st)?;
    let log2_v_chroma_subsample = ur(rc, &t, &mut st)?;
    ensure!(log2_h_chroma_subsample <= 4 && log2_v_chroma_subsample <= 4, "chroma subsampling 2^{log2_h_chroma_subsample} x 2^{log2_v_chroma_subsample}");
    let extra_plane = rc.get_rac(&t, &mut st[0]);
    let (mut num_h_slices, mut num_v_slices, mut quant_table_set_count) = (1, 1, 1);
    if version >= 3 {
        num_h_slices = ur(rc, &t, &mut st)?.saturating_add(1);
        num_v_slices = ur(rc, &t, &mut st)?.saturating_add(1);
        quant_table_set_count = ur(rc, &t, &mut st)?;
    }
    ensure!(num_h_slices <= 256 && num_v_slices <= 256, "{num_h_slices} x {num_v_slices} slices");
    ensure!((1..=MAX_QUANT_TABLE_SETS as u32).contains(&quant_table_set_count), "quant_table_set_count {quant_table_set_count}");
    let mut quant_table_sets = Vec::new();
    for _ in 0..quant_table_set_count {
        quant_table_sets.push(read_quant_table_set(rc, &t)?);
    }
    let (mut ec, mut intra) = (0, 0);
    if version >= 3 {
        for set in quant_table_sets.iter_mut() {
            if rc.get_rac(&t, &mut st[0]) {
                // initial_state_delta[ i ][ j ][ k ], with k as the context index
                let mut ks = [[128u8; CONTEXT_SIZE]; CONTEXT_SIZE];
                let mut rows: Vec<[u8; CONTEXT_SIZE]> = Vec::with_capacity(set.context_count);
                for j in 0..set.context_count {
                    let mut row = [0u8; CONTEXT_SIZE];
                    for (k, r) in row.iter_mut().enumerate() {
                        let pred = if j > 0 { rows.get(j - 1).map_or(128, |p| i32::from(p[k])) } else { 128 };
                        let d = rc.get_symbol(&t, &mut ks[k], true);
                        *r = (pred.wrapping_add(d) & 255) as u8;
                    }
                    rows.push(row);
                }
                set.initial_states = Some(rows);
            }
        }
        ec = ur(rc, &t, &mut st)?;
        intra = ur(rc, &t, &mut st)?;
    }
    if colorspace_type == 1 {
        ensure!(chroma_planes && log2_h_chroma_subsample == 0 && log2_v_chroma_subsample == 0, "RGB with chroma subsampling or no chroma planes");
    }
    Ok(Parameters {
        version,
        micro_version,
        coder_type,
        states,
        colorspace_type,
        bits_per_raw_sample,
        chroma_planes,
        log2_h_chroma_subsample,
        log2_v_chroma_subsample,
        extra_plane,
        num_h_slices,
        num_v_slices,
        quant_table_sets,
        ec,
        intra,
    })
}

/// CRC-32 with the IEEE polynomial 0x04C11DB7, initial value 0, no inversion, MSB first
/// (section 4.9.3). A block that ends in its parity has a remainder of 0.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = (i as u32) << 24;
            for _ in 0..8 {
                c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04C1_1DB7 } else { c << 1 };
            }
            *e = c;
        }
        t
    });
    data.iter().fold(0u32, |crc, &b| (crc << 8) ^ table[usize::from(((crc >> 24) as u8) ^ b)])
}

/// Section 4.3: `ConfigurationRecord( NumBytes )`.
pub(crate) fn read_configuration_record(rec: &[u8]) -> Result<Parameters> {
    ensure!(rec.len() > 4, "configuration record of {} bytes", rec.len());
    if crc32(rec) != 0 {
        return Err(Error::Crc("configuration record".into()));
    }
    let body = rec.get(..rec.len() - 4).unwrap_or_default();
    let mut rc = RangeDecoder::new(body);
    let p = read_parameters(&mut rc)?;
    ensure!(p.version >= 3, "configuration record with FFV1 version {}", p.version);
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_block_with_parity_is_zero() {
        let mut d = b"FFV1 configuration".to_vec();
        let c = crc32(&d);
        d.extend_from_slice(&c.to_be_bytes());
        assert_eq!(crc32(&d), 0);
    }

    #[test]
    fn garbage_records_are_errors() {
        for n in 0..64u8 {
            let rec: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37) ^ n).collect();
            let _ = read_configuration_record(&rec);
        }
    }
}
