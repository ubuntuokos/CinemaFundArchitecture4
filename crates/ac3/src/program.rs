//! E-AC-3 programs of more than 5.1 channels (ATSC A/52:2012 Annex E §E2.3.1.1-2, §E2.3.1.8,
//! Table E2.5): an independent substream followed by dependent substreams whose channels replace
//! or extend the independent substream's.
//!
//! Each channel of a substream has a loudspeaker location. Without a custom channel map the
//! locations follow acmod and lfeon; with one (`chanmape`), the coded channels take the locations
//! of the set `chanmap` bits in order, a pair bit taking two channels. A dependent channel at a
//! location the program already has replaces it (a 7.1 program sends a 5.1 core whose surrounds
//! carry a downmix, and the dependent substream sends the real side surrounds); a new location is
//! added. The program is output in [`Location`] order.

use crate::{Error, NFCHANS, Result};

/// Loudspeaker locations, in output order. The first six are the WAV / SMPTE 5.1 order the
/// decoder has always used (L R C LFE Ls Rs), so programs of 5.1 or fewer channels keep their
/// order. With both surround pairs, the rear pair comes first, as in the WAV / SMPTE order the
/// other decoders give 7.1 (back pair, then side pair): L R C LFE Lrs Rrs Ls Rs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Location {
    L,
    R,
    C,
    Lfe,
    /// Rear surround pair.
    Lrs,
    Rrs,
    Ls,
    Rs,
    /// Centre surround (also the single surround S of acmod 2/1 and 3/1).
    Cs,
    /// Left / right of centre pair.
    Lc,
    Rc,
    /// Surround direct pair.
    Lsd,
    Rsd,
    /// Wide pair.
    Lw,
    Rw,
    /// Vertical height pair.
    Vhl,
    Vhr,
    /// Vertical height centre.
    Vhc,
    /// Top centre surround.
    Ts,
    /// Top surround pair.
    Lts,
    Rts,
    Lfe2,
}

use Location as Loc;

/// Locations of the `chanmap` bits (Table E2.5), bit 0 being the most significant bit of the
/// 16-bit field.
const CHANMAP: [&[Location]; 16] = [
    &[Loc::L],
    &[Loc::C],
    &[Loc::R],
    &[Loc::Ls],
    &[Loc::Rs],
    &[Loc::Lc, Loc::Rc],
    &[Loc::Lrs, Loc::Rrs],
    &[Loc::Cs],
    &[Loc::Ts],
    &[Loc::Lsd, Loc::Rsd],
    &[Loc::Lw, Loc::Rw],
    &[Loc::Vhl, Loc::Vhr],
    &[Loc::Vhc],
    &[Loc::Lts, Loc::Rts],
    &[Loc::Lfe2],
    &[Loc::Lfe],
];

/// Locations of the coded channels (coded order, LFE last) of a substream without a custom
/// channel map (Table 5.8). acmod 0 (1+1, two independent mono programs) has no locations.
pub(crate) fn acmod_locations(acmod: usize, lfeon: bool) -> Result<Vec<Location>> {
    let mut v: Vec<Location> = match acmod {
        1 => vec![Loc::C],
        2 => vec![Loc::L, Loc::R],
        3 => vec![Loc::L, Loc::C, Loc::R],
        4 => vec![Loc::L, Loc::R, Loc::Cs],
        5 => vec![Loc::L, Loc::C, Loc::R, Loc::Cs],
        6 => vec![Loc::L, Loc::R, Loc::Ls, Loc::Rs],
        7 => vec![Loc::L, Loc::C, Loc::R, Loc::Ls, Loc::Rs],
        _ => return Err(Error::Unsupported("dual-mono (1+1) E-AC-3 substream in a multichannel program".into())),
    };
    if lfeon {
        v.push(Loc::Lfe);
    }
    Ok(v)
}

/// Locations of the coded channels (coded order, LFE last) of a dependent substream: from its
/// custom channel map, else from acmod and lfeon.
pub(crate) fn dependent_locations(acmod: usize, lfeon: bool, chanmap: Option<u16>) -> Result<Vec<Location>> {
    let Some(map) = chanmap else {
        return acmod_locations(acmod, lfeon);
    };
    let locs: Vec<Location> = (0..16).filter(|&bit| map & (0x8000 >> bit) != 0).flat_map(|bit| CHANMAP[bit].iter().copied()).collect();
    if locs.len() != NFCHANS[acmod & 7] + lfeon as usize {
        return Err(Error::Invalid("E-AC-3 channel map does not match the substream's channels"));
    }
    Ok(locs)
}

/// The channels of a program being assembled, one per location.
pub(crate) struct Program {
    chans: Vec<(Location, Vec<f32>)>,
}

impl Program {
    /// The independent substream's channels in coded order (LFE last) at `locs`.
    pub(crate) fn new(locs: &[Location], pcm: Vec<Vec<f32>>) -> Program {
        Program { chans: locs.iter().copied().zip(pcm).collect() }
    }

    /// Whether the program has `loc`.
    pub(crate) fn has(&self, loc: Location) -> bool {
        self.chans.iter().any(|(l, _)| *l == loc)
    }

    /// Put `pcm` at `loc`, replacing the channel there. Channels of another length than the
    /// program's (a dependent substream that disagrees with its independent substream) are padded
    /// with silence or cut, so every channel stays as long as the first.
    pub(crate) fn put(&mut self, loc: Location, mut pcm: Vec<f32>) {
        let n = self.chans.first().map_or(pcm.len(), |(_, c)| c.len());
        pcm.resize(n, 0.0);
        match self.chans.iter_mut().find(|(l, _)| *l == loc) {
            Some((_, c)) => *c = pcm,
            None => self.chans.push((loc, pcm)),
        }
    }

    /// Samples per channel.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.chans.first().map_or(0, |(_, c)| c.len())
    }

    /// The channels in [`Location`] order, with their locations.
    pub(crate) fn finish(mut self) -> (Vec<Location>, Vec<Vec<f32>>) {
        self.chans.sort_by_key(|(l, _)| *l);
        self.chans.into_iter().unzip()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chanmap_assigns_locations_in_bit_order_with_pairs() {
        // Ls, Rs and the Lrs / Rrs pair: the four channels of a 2/2 dependent substream
        let locs = dependent_locations(6, false, Some(0x1A00)).unwrap();
        assert_eq!(locs, [Loc::Ls, Loc::Rs, Loc::Lrs, Loc::Rrs]);
        // the Lrs / Rrs pair alone in a 2/0 substream
        assert_eq!(dependent_locations(2, false, Some(0x0200)).unwrap(), [Loc::Lrs, Loc::Rrs]);
        // LFE is the last bit and the last coded channel
        assert_eq!(dependent_locations(1, true, Some(0x0100 | 0x0001)).unwrap(), [Loc::Cs, Loc::Lfe]);
    }

    #[test]
    fn chanmap_with_the_wrong_channel_count_is_an_error() {
        assert!(dependent_locations(6, false, Some(0x0200)).is_err());
        assert!(dependent_locations(2, false, Some(0)).is_err());
        assert!(dependent_locations(7, true, Some(0xFFFF)).is_err());
    }

    #[test]
    fn without_chanmap_the_locations_follow_acmod() {
        assert_eq!(dependent_locations(7, true, None).unwrap(), [Loc::L, Loc::C, Loc::R, Loc::Ls, Loc::Rs, Loc::Lfe]);
        assert!(acmod_locations(0, false).is_err());
    }

    #[test]
    fn programs_of_up_to_51_keep_the_wav_order() {
        // the order wav_order() gives: L R C LFE Ls Rs, and L R [LFE] S for 2/1
        for (acmod, lfeon) in [(7, true), (7, false), (6, true), (4, true), (5, false), (3, true), (2, false), (1, true)] {
            let locs = acmod_locations(acmod, lfeon).unwrap();
            let pcm: Vec<Vec<f32>> = (0..locs.len()).map(|i| vec![i as f32]).collect();
            let (_, ours) = Program::new(&locs, pcm.clone()).finish();
            let want = crate::wav_order(pcm, acmod, lfeon);
            assert_eq!(ours, want, "acmod {acmod} lfeon {lfeon}");
        }
    }

    #[test]
    fn dependent_channels_replace_and_extend() {
        let core = acmod_locations(7, true).unwrap();
        let mut p = Program::new(&core, (0..6).map(|i| vec![i as f32; 4]).collect());
        for (loc, v) in [(Loc::Ls, 10.0), (Loc::Rs, 11.0), (Loc::Lrs, 12.0), (Loc::Rrs, 13.0)] {
            p.put(loc, vec![v; 4]);
        }
        // a short channel is padded to the program's length
        p.put(Loc::Cs, vec![9.0; 2]);
        assert_eq!(p.len(), 4);
        let (locs, pcm) = p.finish();
        assert_eq!(locs, [Loc::L, Loc::R, Loc::C, Loc::Lfe, Loc::Lrs, Loc::Rrs, Loc::Ls, Loc::Rs, Loc::Cs]);
        let first: Vec<f32> = pcm.iter().map(|c| c[0]).collect();
        assert_eq!(first, [0.0, 2.0, 1.0, 5.0, 12.0, 13.0, 10.0, 11.0, 9.0]);
        assert_eq!(pcm[8], [9.0, 9.0, 0.0, 0.0]);
    }
}
