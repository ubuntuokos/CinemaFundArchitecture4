// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-render 0.12.4.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

use crate::grid::AlignedGrid;
use crate::image::ExtraChannelType;

/// Renders a spot color channel onto color_channels
pub fn render_spot_color(mut color_channels: [&mut AlignedGrid<f32>; 3], ec_grid: &AlignedGrid<f32>, ec_ty: &ExtraChannelType) -> crate::render::Result<()> {
    let ExtraChannelType::SpotColour { red, green, blue, solidity } = ec_ty else {
        return Err(crate::render::Error::NotSupported("EC type is not SpotColour"));
    };
    if color_channels.len() != 3 {
        return Ok(());
    }

    let spot_colors = [red, green, blue];
    let s = ec_grid.buf();

    (0..3).for_each(|c| {
        let channel = color_channels[c].buf_mut();
        let color = spot_colors[c];
        assert_eq!(channel.len(), s.len());

        (0..channel.len()).for_each(|i| {
            let mix = s[i] * solidity;
            channel[i] = mix * color + (1.0 - mix) * channel[i];
        });
    });
    Ok(())
}
