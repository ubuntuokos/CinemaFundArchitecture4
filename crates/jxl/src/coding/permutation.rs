// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-coding 1.0.1.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

use crate::bitstream::Bitstream;

/// Read a permutation from the entropy encoded stream.
pub fn read_permutation(bitstream: &mut Bitstream, decoder: &mut crate::coding::Decoder, size: u32, skip: u32) -> crate::coding::CodingResult<Vec<usize>> {
    let end = decoder.read_varint(bitstream, get_context(size))?;
    if end > size - skip {
        crate::tracing::error!(size, skip, end, "Invalid permutation");
        return Err(crate::coding::Error::InvalidPermutation);
    }

    let mut lehmer = vec![0u32; end as usize];
    let mut prev_val = 0u32;
    for (idx, val) in lehmer.iter_mut().enumerate() {
        let idx = idx as u32;
        *val = decoder.read_varint(bitstream, get_context(prev_val))?;
        if *val >= size - skip - idx {
            crate::tracing::error!(idx = idx + skip, size, lehmer = *val, "Invalid permutation");
            return Err(crate::coding::Error::InvalidPermutation);
        }
        prev_val = *val;
    }

    let mut temp = ((skip as usize)..(size as usize)).collect::<Vec<_>>();
    let mut permutation = Vec::with_capacity(size as usize);
    for idx in 0..skip {
        permutation.push(idx as usize);
    }
    for idx in lehmer {
        permutation.push(temp.remove(idx as usize));
    }
    permutation.extend(temp);

    Ok(permutation)
}

fn get_context(x: u32) -> u32 {
    crate::coding::add_log2_ceil(x).min(7)
}
