//! Average-bitrate control. Picture targets come from the global complexities of I, P and B
//! pictures as in MPEG-2 Test Model 5 (ISO/IEC JTC1/SC29/WG11/N0400, §10 step 1); the picture
//! quantiser follows from its type's complexity (bits × quantiser) and the target, and is only
//! nudged within the picture when the bits so far drift from the target, so quality stays even
//! across the picture. Optional adaptive quantisation by spatial activity (TM5 step 3).

const KP: f64 = 1.0;
const KB: f64 = 1.4;

/// Picture coding type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    I = 0,
    P = 1,
    B = 2,
}

pub(super) struct Rc {
    bit_rate: f64,
    pic_rate: f64,
    /// Bits left in the GOP.
    r: f64,
    /// Global complexities (bits × average quantiser_scale) of I, P and B pictures.
    x: [f64; 3],
    /// P and B pictures still to code in this GOP.
    np: u32,
    nb: u32,
    /// Whether the complexities were measured (not TM5's initial guesses).
    measured: [bool; 3],
    // the picture being coded
    kind: Kind,
    target: f64,
    q_pic: f64,
    /// Expected share of the target spent before each macroblock (and 1 at the end).
    profile: Vec<f64>,
    avg_act: f64,
    aq: bool,
    q_sum: f64,
    q_count: f64,
}

impl Rc {
    pub fn new(bit_rate: u64, pic_rate: f64, aq: bool) -> Rc {
        let bit_rate = (bit_rate as f64).max(1.0);
        let pic_rate = if pic_rate.is_finite() && pic_rate > 0.0 { pic_rate } else { 25.0 };
        Rc {
            bit_rate,
            pic_rate,
            r: 0.0,
            // TM5's initial complexities (in its linear quantiser units, ×2 for quantiser_scale)
            x: [2.0 * 160.0 * bit_rate / 115.0, 2.0 * 60.0 * bit_rate / 115.0, 2.0 * 42.0 * bit_rate / 115.0],
            np: 0,
            nb: 0,
            measured: [false; 3],
            kind: Kind::I,
            target: 1.0,
            q_pic: 8.0,
            profile: Vec::new(),
            avg_act: 1.0,
            aq,
            q_sum: 0.0,
            q_count: 0.0,
        }
    }

    /// A GOP starts: one I picture, then `np` P and `nb` B pictures.
    pub fn start_gop(&mut self, np: u32, nb: u32) {
        self.r += self.bit_rate * (1 + np + nb) as f64 / self.pic_rate;
        self.np = np;
        self.nb = nb;
    }

    /// A picture starts; `weights`: each macroblock's expected relative cost.
    pub fn start_picture(&mut self, kind: Kind, weights: &[f64], avg_act: f64) {
        let floor = self.bit_rate / (8.0 * self.pic_rate);
        let (np, nb) = (self.np as f64, self.nb as f64);
        let [xi, xp, xb] = self.x.map(|x| x.max(1.0));
        // TM5 step 1: the bits left shared by the pictures left, weighed by complexity
        let t = match kind {
            Kind::I => self.r / (1.0 + np * xp / (xi * KP) + nb * xb / (xi * KB)),
            Kind::P => self.r / (np + nb * KP * xb / (KB * xp)).max(1.0),
            Kind::B => self.r / (nb + np * KB * xp / (KP * xb)).max(1.0),
        };
        self.target = t.max(floor);
        self.kind = kind;
        self.q_pic = (self.x[kind as usize] / self.target).clamp(1.0, 112.0);
        let total = weights.iter().sum::<f64>();
        self.profile.clear();
        let mut acc = 0.0;
        for w in weights {
            self.profile.push(if total > 0.0 { acc / total } else { 0.0 });
            acc += w;
        }
        self.avg_act = if avg_act.is_finite() && avg_act > 0.0 { avg_act } else { 1.0 };
        self.q_sum = 0.0;
        self.q_count = 0.0;
    }

    /// The quantiser_scale (1..=112) for macroblock `j` after `bits` bits of this picture.
    pub fn scale(&mut self, j: usize, bits: u64, act: f32) -> f64 {
        // drift from the target's pro-rata share, relative to the whole target
        let share = self.profile.get(j).copied().unwrap_or(1.0);
        let drift = (bits as f64 - self.target * share) / self.target;
        let mut q = self.q_pic * (2.0 * drift.clamp(-0.5, 0.5)).exp2();
        if self.aq {
            let act = act as f64;
            q *= (2.0 * act + self.avg_act) / (act + 2.0 * self.avg_act);
        }
        let q = q.clamp(1.0, 112.0);
        self.q_sum += q;
        self.q_count += 1.0;
        q
    }

    /// The complexity of this picture type is still a guess.
    pub fn needs_trial(&self) -> bool {
        !self.measured[self.kind as usize]
    }

    /// A trial run of this picture took `bits`: set the picture quantiser from them.
    pub fn calibrate(&mut self, bits: u64) {
        let x = (bits as f64 * self.average_scale()).max(1.0);
        self.q_pic = (x / self.target).clamp(1.0, 112.0);
        self.measured[self.kind as usize] = true;
        self.q_sum = 0.0;
        self.q_count = 0.0;
    }

    /// The picture took `bits`.
    pub fn end_picture(&mut self, bits: u64) {
        let bits = bits as f64;
        self.r -= bits;
        let x = (bits * self.average_scale()).max(1.0);
        self.measured[self.kind as usize] = true;
        self.x[self.kind as usize] = x;
        match self.kind {
            Kind::I => {}
            Kind::P => self.np = self.np.saturating_sub(1),
            Kind::B => self.nb = self.nb.saturating_sub(1),
        }
    }

    /// The average quantiser_scale of the last picture.
    pub fn average_scale(&self) -> f64 {
        if self.q_count > 0.0 { self.q_sum / self.q_count } else { self.q_pic }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overshoot_raises_the_quantiser_within_the_picture() {
        let mut rc = Rc::new(4_000_000, 25.0, false);
        rc.start_gop(11, 0);
        rc.start_picture(Kind::I, &[1.0; 100], 10.0);
        let q0 = rc.scale(0, 0, 10.0);
        let on_track = rc.scale(50, (rc.target / 2.0) as u64, 10.0);
        let over = rc.scale(50, rc.target as u64, 10.0);
        let under = rc.scale(50, 0, 10.0);
        assert!((q0 - on_track).abs() < 1e-3);
        assert!(over > on_track * 1.5 && under < on_track / 1.5);
        // adaptive quantisation: busy macroblocks get coarser quantisers
        let mut rc = Rc::new(4_000_000, 25.0, true);
        rc.start_gop(11, 0);
        rc.start_picture(Kind::I, &[1.0; 100], 10.0);
        assert!(rc.scale(0, 0, 40.0) > rc.scale(0, 0, 10.0));
    }
}
