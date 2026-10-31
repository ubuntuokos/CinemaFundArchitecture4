//! Channel layouts, format conversion (up/downmix) and the surround panner.
//!
//! **Channel order.** 5.1 audio is always in the SMPTE / WAVE order **L, R, C, LFE, Ls, Rs**
//! ([`L`], [`R`], [`C`], [`LFE`], [`LS`], [`RS`]); stereo is L, R; mono is one channel. Codecs with a
//! different native order (AAC: C, L, R, Ls, Rs, LFE) reorder at the codec boundary.
//! 7.1 (DS, the cinema 7.1 of SMPTE ST 428-12) is **L, R, C, LFE, Lss, Rss, Lrs, Rrs** (the side
//! surrounds in the 5.1 surround slots, [`LSS`], [`RSS`], [`LRS`], [`RRS`]); 7.1.2 (9.1OH, the
//! ISDCF Doc 15 IAB bed) adds the overhead **Lts, Rts** ([`LTS`], [`RTS`]).
//!
//! **Downmix** follows ITU-R BS.775 (Table 2, "3/2 source" rows), with `k = 1/√2` (−3 dB):
//!
//! | from → to | equations |
//! |---|---|
//! | 5.1 → 2.0 | `Lo = L + k·C + k·Ls`, `Ro = R + k·C + k·Rs` (LFE omitted) |
//! | 5.1 → 1.0 | `M = k·L + k·R + C + ½·Ls + ½·Rs` |
//! | 2.0 → 1.0 | `M = k·L + k·R` |
//!
//! The stereo downmix used for playback on stereo devices can be changed by the "5.1 Mixdown Type"
//! preference ([`Mixdown`]): Front Only drops the surrounds, the `+ LFE` variants add `k·LFE` to
//! both sides.
//!
//! **Upmix** places the source on the front speakers without inventing content: mono → C (5.1) or
//! both sides at unity (2.0, as mono clips are played everywhere else in CFA4); stereo → L, R.
//!
//! **7.1 and 7.1.2** are reached through 5.1: 5.1 → 7.1 splits each surround to the side and
//! the rear surround at −3 dB (`Lss = Lrs = k·Ls`, as a 5.1 mix plays on a 7.1 DS cinema
//! system); 7.1 → 5.1 is the reverse, `Ls = k·Lss + k·Lrs`, so 5.1 → 7.1 → 5.1 is exact. The
//! overheads fold into the side surrounds at −3 dB (`Lss += k·Lts`); 7.1 → 7.1.2 leaves them
//! silent. Stereo and mono of 7.1 / 7.1.2 are the 5.1 downmix of their 5.1 downmix.
//!
//! **5.1 panner** ([`Pan51`]): a puck at `(x, y)` in the listening square (x −1 left … +1 right,
//! y −1 rear … +1 front). A point source at `(x, y)` is split front/rear and left/right with the
//! sine/cosine constant-power law, and the front image is shared between the phantom L/R pair and
//! the centre speaker by **Center %** (`k_c = center · (1 − |x|)` of the front power goes to C). The
//! gains of a point source always have unit power (`Σ g² = 1`). Multichannel sources keep their
//! layout around the puck: each source channel sits at its nominal speaker position moved by the
//! puck's offset from front-centre (so the default puck (0, 1) at 100 % centre maps 5.1 to itself
//! exactly). Stereo sources sit at the puck ± 1 in x. The LFE output is the source LFE (5.1) or
//! the mean of the source channels (mono/stereo) times the **LFE** gain.
//!
//! **7.1 / 7.1.2 panner** ([`pan_matrix`]): the same puck over three rows of loudspeakers, front
//! (y = 1), side (y = 0) and rear (y = −1), constant power between the two nearest rows; Center %
//! applies to the front row as in 5.1. A 5.1 source's surrounds sit between the side and rear rows
//! (so the default puck splits them −3 dB, as the upmix does), a 7.1 source's at its own rows (the
//! default puck is the identity). The panner is two-dimensional: a 7.1.2 source's overhead
//! channels go to the overheads of a 7.1.2 bus unpanned, and fold into the side surrounds of
//! anything narrower. A source wider than the bus is downmixed to the bus layout first.

use std::f32::consts::{FRAC_1_SQRT_2, FRAC_PI_2, FRAC_PI_4};

/// −3 dB, the BS.775 downmix coefficient.
pub const K: f32 = FRAC_1_SQRT_2;

/// 5.1 channel indices (SMPTE / WAVE order).
pub const L: usize = 0;
pub const R: usize = 1;
pub const C: usize = 2;
pub const LFE: usize = 3;
pub const LS: usize = 4;
pub const RS: usize = 5;

/// 7.1 / 7.1.2 channel indices: the side surrounds share the 5.1 surround slots.
pub const LSS: usize = 4;
pub const RSS: usize = 5;
pub const LRS: usize = 6;
pub const RRS: usize = 7;
/// 7.1.2 overhead (top surround) channels.
pub const LTS: usize = 8;
pub const RTS: usize = 9;

/// Short names of the 5.1 channels in order.
pub const NAMES_51: [&str; 6] = ["L", "R", "C", "LFE", "Ls", "Rs"];
/// Short names of the 7.1 (DS) channels in order.
pub const NAMES_71: [&str; 8] = ["L", "R", "C", "LFE", "Lss", "Rss", "Lrs", "Rrs"];
/// Short names of the 7.1.2 (9.1OH) channels in order.
pub const NAMES_712: [&str; 10] = ["L", "R", "C", "LFE", "Lss", "Rss", "Lrs", "Rrs", "Lts", "Rts"];

/// A channel layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Layout {
    Mono,
    #[default]
    Stereo,
    /// 5.1 in L, R, C, LFE, Ls, Rs order.
    Surround51,
    /// 7.1 DS in L, R, C, LFE, Lss, Rss, Lrs, Rrs order.
    Surround71,
    /// 7.1.2 (9.1OH): 7.1 DS and the overhead Lts, Rts.
    Surround712,
}

impl Layout {
    pub fn channels(self) -> usize {
        match self {
            Layout::Mono => 1,
            Layout::Stereo => 2,
            Layout::Surround51 => 6,
            Layout::Surround71 => 8,
            Layout::Surround712 => 10,
        }
    }
    /// The layout of a plain channel count (1, 2, 6, 8 or 10; anything else is treated as stereo).
    pub fn from_channels(n: usize) -> Layout {
        match n {
            1 => Layout::Mono,
            6 => Layout::Surround51,
            8 => Layout::Surround71,
            10 => Layout::Surround712,
            _ => Layout::Stereo,
        }
    }
    /// Whether this is a surround layout (5.1, 7.1 or 7.1.2), panned with the surround panner.
    pub fn is_surround(self) -> bool {
        self.channels() >= 6
    }
    /// Channel names for display ("L", "R" …).
    pub fn names(self) -> &'static [&'static str] {
        match self {
            Layout::Mono => &["M"],
            Layout::Stereo => &["L", "R"],
            Layout::Surround51 => &NAMES_51,
            Layout::Surround71 => &NAMES_71,
            Layout::Surround712 => &NAMES_712,
        }
    }
}

/// Premiere's "5.1 Mixdown Type" (Preferences ▸ Audio): how 5.1 is folded to stereo for playback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mixdown {
    /// L, R and C only.
    Front,
    /// L, R, C and the surrounds (ITU-R BS.775).
    #[default]
    FrontRear,
    /// L, R, C and LFE.
    FrontLfe,
    /// Everything.
    FrontRearLfe,
}

impl Mixdown {
    pub const ALL: [Mixdown; 4] = [Mixdown::Front, Mixdown::FrontRear, Mixdown::FrontLfe, Mixdown::FrontRearLfe];
    /// Preference value (`front`, `frontRear`, `frontLfe`, `frontRearLfe`).
    pub fn id(self) -> &'static str {
        match self {
            Mixdown::Front => "front",
            Mixdown::FrontRear => "frontRear",
            Mixdown::FrontLfe => "frontLfe",
            Mixdown::FrontRearLfe => "frontRearLfe",
        }
    }
    pub fn from_id(s: &str) -> Option<Mixdown> {
        Self::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(s))
    }
    fn rear(self) -> bool {
        matches!(self, Mixdown::FrontRear | Mixdown::FrontRearLfe)
    }
    fn lfe(self) -> bool {
        matches!(self, Mixdown::FrontLfe | Mixdown::FrontRearLfe)
    }
}

/// Conversion matrix `m[out][in]` from `from` to `to`. Stereo downmixes of 5.1 use `mixdown`
/// ([`Mixdown::FrontRear`] is BS.775).
pub fn matrix(from: Layout, to: Layout, mixdown: Mixdown) -> Vec<Vec<f32>> {
    let (ni, no) = (from.channels(), to.channels());
    if from == to {
        return (0..no).map(|o| (0..ni).map(|i| if i == o { 1.0 } else { 0.0 }).collect()).collect();
    }
    // 7.1 and 7.1.2 go through 5.1 (see the module documentation)
    match (from, to) {
        (Layout::Surround71, Layout::Surround712) => {
            return (0..no).map(|o| (0..ni).map(|i| if i == o { 1.0 } else { 0.0 }).collect()).collect();
        }
        (Layout::Surround712, Layout::Surround71) => {
            let mut m: Vec<Vec<f32>> = (0..no).map(|o| (0..ni).map(|i| if i == o { 1.0 } else { 0.0 }).collect()).collect();
            m[LSS][LTS] = K;
            m[RSS][RTS] = K;
            return m;
        }
        (Layout::Surround71 | Layout::Surround712, _) => {
            let mut down = vec![vec![0.0f32; ni]; 6];
            for c in [L, R, C, LFE] {
                down[c][c] = 1.0;
            }
            down[LS][LSS] = K;
            down[LS][LRS] = K;
            down[RS][RSS] = K;
            down[RS][RRS] = K;
            if from == Layout::Surround712 {
                // the overheads fold into the side surrounds (k), which reach Ls / Rs at k
                down[LS][LTS] = 0.5;
                down[RS][RTS] = 0.5;
            }
            return compose(&matrix(Layout::Surround51, to, mixdown), &down);
        }
        (_, Layout::Surround71 | Layout::Surround712) => {
            let mut up = vec![vec![0.0f32; 6]; no];
            for c in [L, R, C, LFE] {
                up[c][c] = 1.0;
            }
            up[LSS][LS] = K;
            up[LRS][LS] = K;
            up[RSS][RS] = K;
            up[RRS][RS] = K;
            return compose(&up, &matrix(from, Layout::Surround51, mixdown));
        }
        _ => {}
    }
    let mut m = vec![vec![0.0f32; ni]; no];
    match (from, to) {
        (Layout::Mono, Layout::Mono) | (Layout::Stereo, Layout::Stereo) | (Layout::Surround51, Layout::Surround51) => {
            for (i, row) in m.iter_mut().enumerate() {
                row[i] = 1.0;
            }
        }
        (Layout::Mono, Layout::Stereo) => {
            m[0][0] = 1.0;
            m[1][0] = 1.0;
        }
        (Layout::Mono, Layout::Surround51) => m[C][0] = 1.0,
        (Layout::Stereo, Layout::Mono) => m[0] = vec![K, K],
        (Layout::Stereo, Layout::Surround51) => {
            m[L][0] = 1.0;
            m[R][1] = 1.0;
        }
        (Layout::Surround51, Layout::Mono) => {
            m[0][L] = K;
            m[0][R] = K;
            m[0][C] = 1.0;
            m[0][LS] = 0.5;
            m[0][RS] = 0.5;
        }
        (Layout::Surround51, Layout::Stereo) => {
            m[0][L] = 1.0;
            m[1][R] = 1.0;
            m[0][C] = K;
            m[1][C] = K;
            if mixdown.rear() {
                m[0][LS] = K;
                m[1][RS] = K;
            }
            if mixdown.lfe() {
                m[0][LFE] = K;
                m[1][LFE] = K;
            }
        }
        // 7.1 and 7.1.2 returned above
        _ => {}
    }
    m
}

/// The matrix product `a · b` (`a[out][mid]`, `b[mid][in]`).
pub fn compose(a: &[Vec<f32>], b: &[Vec<f32>]) -> Vec<Vec<f32>> {
    let ni = b.first().map_or(0, Vec::len);
    a.iter()
        .map(|row| {
            (0..ni)
                .map(|i| {
                    let v: f32 = row.iter().zip(b).map(|(g, brow)| g * brow.get(i).copied().unwrap_or(0.0)).sum();
                    if v.abs() < 1e-6 {
                        0.0
                    } else if (v - 1.0).abs() < 1e-6 {
                        1.0
                    } else {
                        v
                    }
                })
                .collect()
        })
        .collect()
}

/// Apply a matrix `m[out][in]` to planar `input` (all channels the same length).
pub fn apply_matrix(m: &[Vec<f32>], input: &[&[f32]]) -> Vec<Vec<f32>> {
    let n = input.first().map_or(0, |c| c.len());
    m.iter()
        .map(|row| {
            let mut out = vec![0.0f32; n];
            for (g, x) in row.iter().zip(input) {
                if *g == 0.0 {
                    continue;
                }
                for (o, s) in out.iter_mut().zip(x.iter()) {
                    *o += g * s;
                }
            }
            out
        })
        .collect()
}

/// Convert planar audio from one layout to another ([`matrix`]).
pub fn convert(input: &[&[f32]], from: Layout, to: Layout, mixdown: Mixdown) -> Vec<Vec<f32>> {
    if from == to {
        return input.iter().map(|c| c.to_vec()).collect();
    }
    apply_matrix(&matrix(from, to, mixdown), input)
}

/// Convert owned planar audio whose layout is given by its channel count.
pub fn convert_owned(input: Vec<Vec<f32>>, to: Layout, mixdown: Mixdown) -> Vec<Vec<f32>> {
    let from = Layout::from_channels(input.len());
    if from == to && input.len() == to.channels() {
        return input;
    }
    let refs: Vec<&[f32]> = input.iter().map(Vec::as_slice).collect();
    if input.len() != from.channels() {
        // unusual channel counts: take the first channels that fit, pad with silence
        let n = input.first().map_or(0, Vec::len);
        let mut v: Vec<Vec<f32>> = input.iter().take(to.channels()).cloned().collect();
        v.resize(to.channels(), vec![0.0; n]);
        return v;
    }
    convert(&refs, from, to, mixdown)
}

// ------------------------------------------------------------------------------------- 5.1 panner

/// The 5.1 panner of a track or send feeding a 5.1 bus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pan51 {
    /// −1 (left) … +1 (right).
    pub x: f32,
    /// −1 (rear) … +1 (front).
    pub y: f32,
    /// Center %: 0 … 1, the share of the front image the centre speaker takes at x = 0.
    pub center: f32,
    /// LFE gain (linear).
    pub lfe: f32,
}

impl Default for Pan51 {
    /// Front centre, 100 % centre, LFE at unity.
    fn default() -> Self {
        Pan51 { x: 0.0, y: 1.0, center: 1.0, lfe: 1.0 }
    }
}

/// Gains of a point source at `(x, y)` to the six 5.1 outputs (LFE is 0). `Σ g² = 1`.
pub fn point_gains(x: f32, y: f32, center: f32) -> [f32; 6] {
    let x = x.clamp(-1.0, 1.0);
    let y = y.clamp(-1.0, 1.0);
    let fb = (1.0 - y) * 0.5 * FRAC_PI_2;
    let (f, b) = (fb.cos(), fb.sin());
    let lr = (x + 1.0) * FRAC_PI_4;
    let (gl, gr) = (lr.cos(), lr.sin());
    let kc = (center.clamp(0.0, 1.0) * (1.0 - x.abs())).clamp(0.0, 1.0);
    let ph = (1.0 - kc).sqrt();
    let mut g = [0.0f32; 6];
    g[L] = f * ph * gl;
    g[R] = f * ph * gr;
    g[C] = f * kc.sqrt();
    g[LS] = b * gl;
    g[RS] = b * gr;
    // f32 sin/cos at the quadrant ends are not exactly 0 / 1: snap them so speaker positions
    // route bit-exactly (a 5.1 source through the default panner is the identity)
    for v in g.iter_mut() {
        if v.abs() < 1e-6 {
            *v = 0.0;
        } else if (*v - 1.0).abs() < 1e-6 {
            *v = 1.0;
        }
    }
    g
}

/// Nominal source-channel positions relative to the puck's offset from front-centre, on a 5.1
/// bus (`rows3` false: the surrounds are the rear) or a 7.1 / 7.1.2 bus (`rows3`: front, side and
/// rear rows, a 5.1 surround between the side and the rear). Sources wider than the bus are
/// downmixed before panning, so 7.1 appears only with `rows3`; overheads are not positioned.
fn offsets(from: Layout, rows3: bool) -> &'static [(f32, f32)] {
    match (from, rows3) {
        (Layout::Mono, _) => &[(0.0, 0.0)],
        (Layout::Stereo, _) => &[(-1.0, 0.0), (1.0, 0.0)],
        // L, R, C, LFE (not positioned), Ls, Rs
        (Layout::Surround51, false) => &[(-1.0, 0.0), (1.0, 0.0), (0.0, 0.0), (0.0, 0.0), (-1.0, -2.0), (1.0, -2.0)],
        (Layout::Surround51, true) => &[(-1.0, 0.0), (1.0, 0.0), (0.0, 0.0), (0.0, 0.0), (-1.0, -1.5), (1.0, -1.5)],
        // L, R, C, LFE, Lss, Rss, Lrs, Rrs (and Lts, Rts, not positioned)
        (Layout::Surround71 | Layout::Surround712, _) => {
            &[(-1.0, 0.0), (1.0, 0.0), (0.0, 0.0), (0.0, 0.0), (-1.0, -1.0), (1.0, -1.0), (-1.0, -2.0), (1.0, -2.0), (0.0, 0.0), (0.0, 0.0)]
        }
    }
}

/// Gains of a point source at `(x, y)` to the eight 7.1 outputs (LFE is 0): front (y = 1), side
/// (y = 0) and rear (y = −1) rows, constant power between neighbouring rows. `Σ g² = 1`.
pub fn point_gains_71(x: f32, y: f32, center: f32) -> [f32; 8] {
    let x = x.clamp(-1.0, 1.0);
    let y = y.clamp(-1.0, 1.0);
    let (f, sd, b) = if y >= 0.0 {
        let t = (1.0 - y) * FRAC_PI_2;
        (t.cos(), t.sin(), 0.0)
    } else {
        let t = -y * FRAC_PI_2;
        (0.0, t.cos(), t.sin())
    };
    let lr = (x + 1.0) * FRAC_PI_4;
    let (gl, gr) = (lr.cos(), lr.sin());
    let kc = (center.clamp(0.0, 1.0) * (1.0 - x.abs())).clamp(0.0, 1.0);
    let ph = (1.0 - kc).sqrt();
    let mut g = [0.0f32; 8];
    g[L] = f * ph * gl;
    g[R] = f * ph * gr;
    g[C] = f * kc.sqrt();
    g[LSS] = sd * gl;
    g[RSS] = sd * gr;
    g[LRS] = b * gl;
    g[RRS] = b * gr;
    for v in g.iter_mut() {
        if v.abs() < 1e-6 {
            *v = 0.0;
        } else if (*v - 1.0).abs() < 1e-6 {
            *v = 1.0;
        }
    }
    g
}

/// Panning matrix `m[out][in]` of a `from` source through the surround panner of a `bus` layout
/// (5.1, 7.1 or 7.1.2; a non-surround bus is treated as 5.1). A 5.1 bus is [`pan51_matrix`];
/// a source wider than the bus is downmixed to it first.
pub fn pan_matrix(from: Layout, bus: Layout, p: Pan51) -> Vec<Vec<f32>> {
    let bus = if bus.is_surround() { bus } else { Layout::Surround51 };
    if bus == Layout::Surround51 {
        if from.channels() > 6 {
            return compose(&pan51_matrix(Layout::Surround51, p), &matrix(from, Layout::Surround51, Mixdown::FrontRear));
        }
        return pan51_matrix(from, p);
    }
    if bus == Layout::Surround71 && from == Layout::Surround712 {
        return compose(&pan_matrix(Layout::Surround71, bus, p), &matrix(from, Layout::Surround71, Mixdown::FrontRear));
    }
    let (ni, no) = (from.channels(), bus.channels());
    let mut m = vec![vec![0.0f32; ni]; no];
    let multichannel = from.is_surround();
    for (i, (ox, oy)) in offsets(from, true).iter().enumerate().take(ni) {
        if multichannel && i == LFE {
            m[LFE][i] = p.lfe;
            continue;
        }
        if from == Layout::Surround712 && (i == LTS || i == RTS) {
            // overheads are not panned (the bus is 7.1.2 here)
            m[i][i] = 1.0;
            continue;
        }
        let g = point_gains_71(p.x + ox, p.y + oy, p.center);
        for (o, gain) in g.iter().enumerate() {
            m[o][i] = *gain;
        }
        if !multichannel {
            m[LFE][i] = p.lfe / ni as f32;
        }
    }
    m
}

/// Panning matrix `m[out][in]` (6 outputs) of a `from` source through the 5.1 panner.
pub fn pan51_matrix(from: Layout, p: Pan51) -> Vec<Vec<f32>> {
    if from.channels() > 6 {
        return pan_matrix(from, Layout::Surround51, p);
    }
    let ni = from.channels();
    let mut m = vec![vec![0.0f32; ni]; 6];
    for (i, (ox, oy)) in offsets(from, false).iter().enumerate() {
        if from == Layout::Surround51 && i == LFE {
            m[LFE][i] = p.lfe;
            continue;
        }
        let g = point_gains(p.x + ox, p.y + oy, p.center);
        for (o, row) in m.iter_mut().enumerate() {
            row[i] = g[o];
        }
        if from != Layout::Surround51 {
            m[LFE][i] = p.lfe / ni as f32;
        }
    }
    m
}

/// Peak level (linear) per channel.
pub fn peaks(input: &[&[f32]]) -> Vec<f32> {
    input.iter().map(|c| c.iter().fold(0f32, |a, x| a.max(x.abs()))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn bs775_downmix_coefficients_are_exact() {
        let k = std::f32::consts::FRAC_1_SQRT_2;
        // 5.1 → 2.0 (L, R, C, LFE, Ls, Rs)
        let m = matrix(Layout::Surround51, Layout::Stereo, Mixdown::FrontRear);
        assert_eq!(m[0], vec![1.0, 0.0, k, 0.0, k, 0.0]);
        assert_eq!(m[1], vec![0.0, 1.0, k, 0.0, 0.0, k]);
        // 5.1 → 1.0
        let m = matrix(Layout::Surround51, Layout::Mono, Mixdown::FrontRear);
        assert_eq!(m[0], vec![k, k, 1.0, 0.0, 0.5, 0.5]);
        // 2.0 → 1.0
        assert_eq!(matrix(Layout::Stereo, Layout::Mono, Mixdown::FrontRear)[0], vec![k, k]);
        // mixdown types
        let f = matrix(Layout::Surround51, Layout::Stereo, Mixdown::Front);
        assert_eq!(f[0], vec![1.0, 0.0, k, 0.0, 0.0, 0.0]);
        let fl = matrix(Layout::Surround51, Layout::Stereo, Mixdown::FrontLfe);
        assert_eq!(fl[1], vec![0.0, 1.0, k, k, 0.0, 0.0]);
        let all = matrix(Layout::Surround51, Layout::Stereo, Mixdown::FrontRearLfe);
        assert_eq!(all[0], vec![1.0, 0.0, k, k, k, 0.0]);
        assert_eq!(Mixdown::from_id("frontRearLfe"), Some(Mixdown::FrontRearLfe));
        assert_eq!(Mixdown::default().id(), "frontRear");
    }

    #[test]
    fn upmix_places_on_front_speakers() {
        assert_eq!(matrix(Layout::Mono, Layout::Surround51, Mixdown::FrontRear), vec![vec![0.0], vec![0.0], vec![1.0], vec![0.0], vec![0.0], vec![0.0]]);
        let s = matrix(Layout::Stereo, Layout::Surround51, Mixdown::FrontRear);
        assert_eq!(s[L], vec![1.0, 0.0]);
        assert_eq!(s[R], vec![0.0, 1.0]);
        assert!(s[C..].iter().all(|r| r.iter().all(|g| *g == 0.0)));
        assert_eq!(matrix(Layout::Mono, Layout::Stereo, Mixdown::FrontRear), vec![vec![1.0], vec![1.0]]);
        // identity
        for l in [Layout::Mono, Layout::Stereo, Layout::Surround51] {
            let m = matrix(l, l, Mixdown::Front);
            for (o, row) in m.iter().enumerate() {
                for (i, g) in row.iter().enumerate() {
                    assert_eq!(*g, if o == i { 1.0 } else { 0.0 });
                }
            }
        }
    }

    #[test]
    fn convert_applies_the_matrix_per_sample() {
        let ch: Vec<Vec<f32>> = (0..6).map(|c| vec![(c + 1) as f32 * 0.1; 4]).collect();
        let refs: Vec<&[f32]> = ch.iter().map(Vec::as_slice).collect();
        let st = convert(&refs, Layout::Surround51, Layout::Stereo, Mixdown::FrontRear);
        let k = K;
        assert!(close(st[0][3], 0.1 + k * 0.3 + k * 0.5));
        assert!(close(st[1][0], 0.2 + k * 0.3 + k * 0.6));
        let back = convert_owned(st.clone(), Layout::Stereo, Mixdown::FrontRear);
        assert_eq!(back, st);
        let up = convert_owned(vec![vec![0.5; 3]], Layout::Surround51, Mixdown::FrontRear);
        assert_eq!(up.len(), 6);
        assert_eq!(up[C], vec![0.5; 3]);
        assert_eq!(up[L], vec![0.0; 3]);
    }

    #[test]
    fn panner_point_gains_are_equal_power() {
        for xi in -10..=10 {
            for yi in -10..=10 {
                for c in [0.0, 0.3, 1.0] {
                    let g = point_gains(xi as f32 / 10.0, yi as f32 / 10.0, c);
                    let p: f32 = g.iter().map(|v| v * v).sum();
                    assert!((p - 1.0).abs() < 1e-5, "({xi}, {yi}, {c}) power {p}");
                    assert_eq!(g[LFE], 0.0);
                    assert!(g.iter().all(|v| *v >= -1e-7));
                }
            }
        }
        let k = K;
        // speaker positions
        let at = |x, y, c| point_gains(x, y, c);
        assert!(close(at(0.0, 1.0, 1.0)[C], 1.0));
        assert!(close(at(-1.0, 1.0, 1.0)[L], 1.0));
        assert!(close(at(1.0, 1.0, 0.5)[R], 1.0));
        assert!(close(at(-1.0, -1.0, 1.0)[LS], 1.0));
        assert!(close(at(1.0, -1.0, 1.0)[RS], 1.0));
        // phantom centre at 0 % centre: −3 dB each side
        let g = at(0.0, 1.0, 0.0);
        assert!(close(g[L], k) && close(g[R], k) && g[C] == 0.0);
        // room centre: −3 dB front/rear, then −3 dB left/right
        let g = at(0.0, 0.0, 0.0);
        assert!(close(g[L], 0.5) && close(g[R], 0.5) && close(g[LS], 0.5) && close(g[RS], 0.5));
        // 50 % centre at front centre: half the power to C
        let g = at(0.0, 1.0, 0.5);
        assert!(close(g[C] * g[C], 0.5) && close(g[L] * g[L] + g[R] * g[R], 0.5));
    }

    fn assert_identity(m: &[Vec<f32>]) {
        for (o, row) in m.iter().enumerate() {
            for (i, g) in row.iter().enumerate() {
                assert!(close(*g, if o == i { 1.0 } else { 0.0 }), "out {o} in {i} = {g}");
            }
        }
    }

    #[test]
    fn seven_one_layouts_up_and_downmix_through_5_1() {
        let k = K;
        assert_eq!((Layout::Surround71.channels(), Layout::Surround712.channels()), (8, 10));
        assert_eq!(Layout::from_channels(8), Layout::Surround71);
        assert_eq!(Layout::from_channels(10), Layout::Surround712);
        assert_eq!(Layout::Surround712.names()[LTS], "Lts");
        assert!(Layout::Surround71.is_surround() && !Layout::Stereo.is_surround());
        // 5.1 → 7.1: each surround split −3 dB to the side and the rear
        let up = matrix(Layout::Surround51, Layout::Surround71, Mixdown::FrontRear);
        assert_eq!(up.len(), 8);
        assert!(close(up[LSS][LS], k) && close(up[LRS][LS], k) && close(up[RSS][RS], k) && close(up[RRS][RS], k));
        assert!(close(up[L][L], 1.0) && close(up[LFE][LFE], 1.0) && up[LSS][RS] == 0.0);
        // 7.1 → 5.1 → back is exact
        let down = matrix(Layout::Surround71, Layout::Surround51, Mixdown::FrontRear);
        assert!(close(down[LS][LSS], k) && close(down[LS][LRS], k) && down[LS][RSS] == 0.0);
        assert_identity(&compose(&down, &up));
        // 7.1.2: the overheads fold into the side surrounds; 7.1 → 7.1.2 leaves them silent
        let fold = matrix(Layout::Surround712, Layout::Surround71, Mixdown::FrontRear);
        assert!(close(fold[LSS][LTS], k) && close(fold[RSS][RTS], k) && close(fold[LSS][LSS], 1.0) && fold[LSS][RTS] == 0.0);
        let rise = matrix(Layout::Surround71, Layout::Surround712, Mixdown::FrontRear);
        assert!(rise[LTS].iter().all(|g| *g == 0.0) && close(rise[RRS][RRS], 1.0));
        let d51 = matrix(Layout::Surround712, Layout::Surround51, Mixdown::FrontRear);
        assert!(close(d51[LS][LTS], 0.5) && close(d51[RS][RTS], 0.5));
        // stereo of 7.1 is BS.775 of its 5.1 downmix; stereo → 7.1 is L, R
        let st = matrix(Layout::Surround71, Layout::Stereo, Mixdown::FrontRear);
        assert_eq!(st[0].len(), 8);
        assert!(close(st[0][L], 1.0) && close(st[0][C], k) && close(st[0][LSS], 0.5) && close(st[0][LRS], 0.5) && st[0][RSS] == 0.0);
        let s71 = matrix(Layout::Stereo, Layout::Surround71, Mixdown::FrontRear);
        assert!(close(s71[L][0], 1.0) && close(s71[R][1], 1.0) && s71[C..].iter().all(|r| r.iter().all(|g| *g == 0.0)));
        assert_eq!(matrix(Layout::Mono, Layout::Surround712, Mixdown::FrontRear)[C], vec![1.0]);
        for l in [Layout::Surround71, Layout::Surround712] {
            assert_identity(&matrix(l, l, Mixdown::Front));
        }
        // convert_owned picks the layout from the channel count
        let ten = convert_owned(vec![vec![0.25; 2]; 10], Layout::Surround51, Mixdown::FrontRear);
        assert_eq!(ten.len(), 6);
        assert!(close(ten[LS][0], 0.25 * (2.0 * k + 0.5)));
    }

    #[test]
    fn seven_one_panner() {
        for xi in -10..=10 {
            for yi in -10..=10 {
                let g = point_gains_71(xi as f32 / 10.0, yi as f32 / 10.0, 0.4);
                let p: f32 = g.iter().map(|v| v * v).sum();
                assert!((p - 1.0).abs() < 1e-5, "({xi}, {yi}) power {p}");
                assert_eq!(g[LFE], 0.0);
            }
        }
        // the three rows
        assert!(close(point_gains_71(-1.0, 0.0, 1.0)[LSS], 1.0));
        assert!(close(point_gains_71(1.0, -1.0, 1.0)[RRS], 1.0));
        assert!(close(point_gains_71(0.0, 1.0, 1.0)[C], 1.0));
        let g = point_gains_71(-1.0, -0.5, 1.0);
        assert!(close(g[LSS], K) && close(g[LRS], K));
        // 7.1 and 7.1.2 through the default panner of their own bus are the identity
        assert_identity(&pan_matrix(Layout::Surround71, Layout::Surround71, Pan51::default()));
        assert_identity(&pan_matrix(Layout::Surround712, Layout::Surround712, Pan51::default()));
        // a 5.1 source on a 7.1 bus: the upmix (surrounds −3 dB to side and rear)
        let m = pan_matrix(Layout::Surround51, Layout::Surround71, Pan51::default());
        let up = matrix(Layout::Surround51, Layout::Surround71, Mixdown::FrontRear);
        for o in 0..8 {
            for i in 0..6 {
                assert!(close(m[o][i], up[o][i]), "out {o} in {i}: {} vs {}", m[o][i], up[o][i]);
            }
        }
        // a 5.1 bus is the 5.1 panner; a 7.1 source on it is downmixed first
        assert_eq!(pan_matrix(Layout::Stereo, Layout::Surround51, Pan51::default()), pan51_matrix(Layout::Stereo, Pan51::default()));
        let m = pan_matrix(Layout::Surround71, Layout::Surround51, Pan51::default());
        let down = matrix(Layout::Surround71, Layout::Surround51, Mixdown::FrontRear);
        assert_eq!(m.len(), 6);
        for o in 0..6 {
            for i in 0..8 {
                assert!(close(m[o][i], down[o][i]));
            }
        }
        assert_eq!(pan51_matrix(Layout::Surround71, Pan51::default()), m);
        // 7.1.2 on a 7.1 bus: the overheads fold into the sides
        let m = pan_matrix(Layout::Surround712, Layout::Surround71, Pan51::default());
        assert!(close(m[LSS][LTS], K) && m.len() == 8);
        // mono on a 7.1.2 bus: point source and LFE share, nothing overhead
        let m = pan_matrix(Layout::Mono, Layout::Surround712, Pan51 { lfe: 0.5, ..Pan51::default() });
        assert!(close(m[C][0], 1.0) && close(m[LFE][0], 0.5) && m[LTS][0] == 0.0);
        // every source channel keeps unit power (LFE and overheads aside)
        let p = Pan51 { x: -0.3, y: -0.6, center: 0.5, lfe: 0.0 };
        for l in [Layout::Mono, Layout::Stereo, Layout::Surround51, Layout::Surround71, Layout::Surround712] {
            let m = pan_matrix(l, Layout::Surround712, p);
            for i in 0..l.channels() {
                if l.is_surround() && i == LFE {
                    continue;
                }
                let pw: f32 = m.iter().map(|row| row[i] * row[i]).sum();
                assert!((pw - 1.0).abs() < 1e-5, "{l:?} channel {i}: {pw}");
            }
        }
    }

    #[test]
    fn panner_matrices() {
        // 5.1 through the default panner is the identity
        let m = pan51_matrix(Layout::Surround51, Pan51::default());
        for (o, row) in m.iter().enumerate() {
            for (i, g) in row.iter().enumerate() {
                assert!(close(*g, if o == i { 1.0 } else { 0.0 }), "out {o} in {i} = {g}");
            }
        }
        // stereo at front centre: L → L, R → R
        let m = pan51_matrix(Layout::Stereo, Pan51 { lfe: 0.0, ..Pan51::default() });
        assert!(close(m[L][0], 1.0) && close(m[R][1], 1.0) && m[C][0].abs() < 1e-6 && m[LFE][0] == 0.0);
        // mono at the default puck: centre only; LFE gain = mean of inputs
        let m = pan51_matrix(Layout::Mono, Pan51 { lfe: 0.5, ..Pan51::default() });
        assert!(close(m[C][0], 1.0) && close(m[LFE][0], 0.5));
        let m = pan51_matrix(Layout::Stereo, Pan51 { lfe: 0.5, ..Pan51::default() });
        assert!(close(m[LFE][0], 0.25) && close(m[LFE][1], 0.25));
        // every source channel keeps unit power (LFE aside)
        for l in [Layout::Mono, Layout::Stereo, Layout::Surround51] {
            let p = Pan51 { x: 0.37, y: -0.21, center: 0.6, lfe: 0.0 };
            let m = pan51_matrix(l, p);
            for i in 0..l.channels() {
                if l == Layout::Surround51 && i == LFE {
                    continue;
                }
                let pw: f32 = (0..6).map(|o| m[o][i] * m[o][i]).sum();
                assert!((pw - 1.0).abs() < 1e-5);
            }
        }
        // puck hard rear-left: 5.1 front channels collapse towards the rear left
        let m = pan51_matrix(Layout::Surround51, Pan51 { x: -1.0, y: -1.0, center: 1.0, lfe: 1.0 });
        assert!(close(m[LS][L], 1.0) && close(m[LS][C], 1.0) && close(m[LS][LS], 1.0));
    }
}
