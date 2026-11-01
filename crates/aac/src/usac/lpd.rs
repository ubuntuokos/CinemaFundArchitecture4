//! LPD core (ACELP / TCX) of USAC. Not implemented yet.

use cfa4_bitstream::BitReader;

use crate::{Error, Result};

/// `fac_data()` coefficients (AVQ-coded).
pub fn fac_decoding(_br: &mut BitReader, _lfac: usize, _out: &mut [i32]) -> Result<()> {
    Err(Error::Unsupported("USAC LPD core (FAC data)"))
}
