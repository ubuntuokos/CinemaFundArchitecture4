//! Average-bitrate control. Picture targets come from the global complexities of I and P
//! pictures as in MPEG-2 Test Model 5 (ISO/IEC JTC1/SC29/WG11/N0400, §10 step 1); the picture
//! quantiser follows from its type's complexity (bits × quantiser) and the target, and is only
//! nudged within the picture when the bits so far drift from the target, so quality stays even
//! across the picture. Optional adaptive quantisation by spatial activity (TM5 step 3).

const KP: f64 = 1.0;

pub(super) struct Rc {
    bit_rate: f64,
    pic_rate: f64,
    /// Bits left in the GOP.
    r: f64,
    /// Global complexities (bits × average quantiser_scale).
    xi: f64,
    xp: f64,
    /// P pictures still to code in this GOP.
    np: u32,
    /// Whether I / P complexities were measured (not TM5's initial guesses).
    measured: [bool; 2],
    // the picture being coded
    intra: bool,
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
            xi: 2.0 * 160.0 * bit_rate / 115.0,
            xp: 2.0 * 60.0 * bit_rate / 115.0,
            np: 0,
            measured: [false; 2],
            intra: true,
            target: 1.0,
            q_pic: 8.0,
            profile: Vec::new(),
            avg_act: 1.0,
            aq,
            q_sum: 0.0,
            q_count: 0.0,
        }
    }

    /// A GOP of `n` pictures starts (one I, then P).
    pub fn start_gop(&mut self, n: u32) {
        self.r += self.bit_rate * n.max(1) as f64 / self.pic_rate;
        self.np = n.saturating_sub(1);
    }

    /// A picture starts; `weights`: each macroblock's expected relative cost.
    pub fn start_picture(&mut self, intra: bool, weights: &[f64], avg_act: f64) {
        let floor = self.bit_rate / (8.0 * self.pic_rate);
        let np = self.np as f64;
        let t = if intra { self.r / (1.0 + np * self.xp / (self.xi * KP)) } else { self.r / np.max(1.0) };
        self.target = t.max(floor);
        self.intra = intra;
        let x = if intra { self.xi } else { self.xp };
        self.q_pic = (x / self.target).clamp(1.0, 112.0);
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
        !self.measured[self.intra as usize]
    }

    /// A trial run of this picture took `bits`: set the picture quantiser from them.
    pub fn calibrate(&mut self, bits: u64) {
        let x = (bits as f64 * self.average_scale()).max(1.0);
        self.q_pic = (x / self.target).clamp(1.0, 112.0);
        self.measured[self.intra as usize] = true;
        self.q_sum = 0.0;
        self.q_count = 0.0;
    }

    /// The picture took `bits`.
    pub fn end_picture(&mut self, bits: u64) {
        let bits = bits as f64;
        self.r -= bits;
        let x = (bits * self.average_scale()).max(1.0);
        self.measured[self.intra as usize] = true;
        if self.intra {
            self.xi = x;
        } else {
            self.xp = x;
            self.np = self.np.saturating_sub(1);
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
        rc.start_gop(12);
        rc.start_picture(true, &[1.0; 100], 10.0);
        let q0 = rc.scale(0, 0, 10.0);
        let on_track = rc.scale(50, (rc.target / 2.0) as u64, 10.0);
        let over = rc.scale(50, rc.target as u64, 10.0);
        let under = rc.scale(50, 0, 10.0);
        assert!((q0 - on_track).abs() < 1e-3);
        assert!(over > on_track * 1.5 && under < on_track / 1.5);
        // adaptive quantisation: busy macroblocks get coarser quantisers
        let mut rc = Rc::new(4_000_000, 25.0, true);
        rc.start_gop(12);
        rc.start_picture(true, &[1.0; 100], 10.0);
        assert!(rc.scale(0, 0, 40.0) > rc.scale(0, 0, 10.0));
    }
}
