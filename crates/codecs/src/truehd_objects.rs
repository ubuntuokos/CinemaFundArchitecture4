//! The object audio of a Dolby TrueHD Atmos track, unrendered, for the headphone monitor
//! (`docs/plans/CFA4-BINAURAL-MONITOR-001.md` B-03; [`cfa4_media::MediaSource::object_audio`]).
//!
//! The container hands over its packets (one or more access units each) and their first samples;
//! [`TrueHdObjects::read`] decodes the object presentation with `cfa4_truehd::cfa4::ObjectDecoder`
//! from an access unit with a major sync at or before the block, less the decoder's pre-roll, and
//! keeps decoding forward while the blocks follow each other. A jump back, or more than
//! [`MAX_SKIP_SECONDS`] ahead, starts over. Metadata offsets come out in media samples.

use std::sync::Mutex;

use cfa4_media::{MediaError, ObjectAudio};
use cfa4_truehd::cfa4::ObjectDecoder;
use cfa4_truehd::render::Event;

/// How far ahead a block may start and still be reached by decoding on, in seconds.
pub const MAX_SKIP_SECONDS: i64 = 4;
/// Packets searched back for a major sync (an interval is at most 129 access units).
const SEARCH_BACK: usize = 512;
/// Most objects kept (as `cfa4_truehd::render::MAX_OBJECTS`).
const MAX_OBJECTS: usize = cfa4_truehd::render::MAX_OBJECTS;

/// Whether `packet` starts with an access unit carrying a major sync (TrueHD or MLP).
pub fn starts_with_major_sync(packet: &[u8]) -> bool {
    matches!(packet.get(4..8), Some([0xF8, 0x72, 0x6F, 0xBA | 0xBB]))
}

struct State {
    decoder: ObjectDecoder,
    /// The media sample of the decoder's first sample.
    base: i64,
    /// The next packet to decode.
    next: usize,
    rate: u32,
    /// Samples decoded and not yet passed, from `from`.
    from: i64,
    objects: Vec<Vec<f32>>,
    /// Each object's last update before `from`.
    current: Vec<Option<Event>>,
    /// Updates at or after `from`, in order (media samples).
    events: Vec<Event>,
    /// The stream ended: no more packets.
    ended: bool,
}

impl State {
    fn end(&self) -> i64 {
        self.from + self.objects.first().map_or(0, Vec::len) as i64
    }

    /// Forgets what lies before `at`, keeping the objects' last updates.
    fn pass(&mut self, at: i64) {
        let n = usize::try_from(at - self.from).unwrap_or(0);
        if n == 0 {
            return;
        }
        let at_u = u64::try_from(at).unwrap_or(0);
        let gone = self.events.partition_point(|e| e.offset < at_u);
        for e in self.events.drain(..gone) {
            if self.current.len() <= e.object {
                self.current.resize(e.object + 1, None);
            }
            self.current[e.object] = Some(e);
        }
        for o in &mut self.objects {
            o.drain(..n.min(o.len()));
        }
        self.from = at;
    }
}

/// A TrueHD track's object decoding state (one per track; see the module notes).
#[derive(Default)]
pub struct TrueHdObjects {
    state: Mutex<Option<State>>,
}

impl TrueHdObjects {
    pub fn new() -> Self {
        Self::default()
    }

    /// The objects of `frames` samples from `start`. `starts[i]` is packet `i`'s first sample
    /// (media samples at the stream's rate `rate`), `read(i)` its bytes, `preroll` the samples the
    /// decoder needs before `start`. `None` when the track has no object presentation.
    pub fn read(
        &self,
        starts: &[i64],
        rate: u32,
        preroll: i64,
        start: i64,
        frames: usize,
        read: impl Fn(usize) -> Result<Vec<u8>, MediaError>,
    ) -> Option<Result<ObjectAudio, MediaError>> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let skip = MAX_SKIP_SECONDS.saturating_mul(i64::from(rate.max(1)));
        let start = start.max(0);
        let reuse = guard.as_ref().is_some_and(|s| start >= s.from && start <= s.end().saturating_add(skip));
        if !reuse {
            *guard = None;
            let i0 = starts.partition_point(|&x| x <= start.saturating_sub(preroll)).saturating_sub(1);
            let mut found = None;
            for i in (i0.saturating_sub(SEARCH_BACK)..=i0).rev() {
                match read(i) {
                    Ok(p) if starts_with_major_sync(&p) => {
                        found = Some(i);
                        break;
                    }
                    Ok(_) => {}
                    Err(e) => return Some(Err(e)),
                }
            }
            let i = found?;
            let base = *starts.get(i)?;
            *guard = Some(State {
                decoder: ObjectDecoder::new(),
                base,
                next: i,
                rate,
                from: base,
                objects: Vec::new(),
                current: Vec::new(),
                events: Vec::new(),
                ended: false,
            });
        }
        let st = guard.as_mut()?;
        let want = start.saturating_add(frames as i64);
        while st.end() < want && !st.ended {
            if st.next >= starts.len() {
                st.ended = true;
                break;
            }
            let data = match read(st.next) {
                Ok(d) => d,
                Err(e) => {
                    *guard = None;
                    return Some(Err(e));
                }
            };
            st.next += 1;
            let chunk = match st.decoder.decode(&data) {
                Ok(c) => c,
                Err(e) => {
                    let not_atmos = e.to_string().contains("no object presentation");
                    *guard = None;
                    // a stream without objects: the channels are all there is
                    return if not_atmos { None } else { Some(Err(MediaError::Decode(e.to_string()))) };
                }
            };
            if chunk.objects.is_empty() {
                continue;
            }
            if chunk.sample_rate != st.rate {
                *guard = None;
                return Some(Err(MediaError::Decode("the TrueHD objects' rate differs from the track's".into())));
            }
            let have = st.objects.first().map_or(0, Vec::len);
            let count = chunk.objects.len().min(MAX_OBJECTS);
            if st.objects.len() < count {
                st.objects.resize(count, vec![0.0; have]);
            }
            let len = chunk.objects.first().map_or(0, Vec::len);
            for (c, o) in st.objects.iter_mut().enumerate() {
                match chunk.objects.get(c) {
                    Some(x) => o.extend_from_slice(x),
                    None => o.resize(o.len() + len, 0.0),
                }
            }
            let base = u64::try_from(st.base).unwrap_or(0);
            st.events.extend(chunk.events.into_iter().filter(|e| e.object < MAX_OBJECTS).map(|mut e| {
                e.offset = e.offset.saturating_add(base);
                e
            }));
            // keep only what the block needs
            let keep_from = start.min(st.end());
            st.pass(keep_from.max(st.from));
        }
        st.pass(start.min(st.end()).max(st.from));
        let block_end = u64::try_from(want).unwrap_or(0);
        let begin = st.events.partition_point(|e| e.offset < u64::try_from(start).unwrap_or(0));
        let end = st.events.partition_point(|e| e.offset < block_end);
        let lead = usize::try_from(st.from - start).unwrap_or(0);
        let objects = st
            .objects
            .iter()
            .map(|o| {
                let mut v = vec![0.0; frames];
                for (k, x) in o.iter().take(frames.saturating_sub(lead)).enumerate() {
                    v[lead + k] = *x;
                }
                v
            })
            .collect();
        Some(Ok(ObjectAudio {
            sample_rate: st.rate,
            start,
            objects,
            current: st.current.iter().flatten().cloned().collect(),
            events: st.events.get(begin..end).unwrap_or_default().to_vec(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_syncs_are_recognised() {
        assert!(starts_with_major_sync(&[0, 0, 0, 0, 0xF8, 0x72, 0x6F, 0xBA, 1]));
        assert!(starts_with_major_sync(&[0, 0, 0, 0, 0xF8, 0x72, 0x6F, 0xBB]));
        assert!(!starts_with_major_sync(&[0, 0, 0, 0, 0xF8, 0x72, 0x6F]));
        assert!(!starts_with_major_sync(&[]));
    }

    #[test]
    fn a_track_without_a_major_sync_has_no_objects() {
        let objects = TrueHdObjects::new();
        let starts = [0, 40, 80];
        assert!(objects.read(&starts, 48_000, 0, 50, 100, |_| Ok(vec![0; 16])).is_none());
        let failing = objects.read(&starts, 48_000, 0, 50, 100, |_| Err(MediaError::Decode("gone".into())));
        assert!(matches!(failing, Some(Err(_))));
    }
}
