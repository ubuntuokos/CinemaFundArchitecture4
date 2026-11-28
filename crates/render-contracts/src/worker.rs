//! Worker protocol v0 (messages and rules only; the frame-exchange mechanism, shared memory or
//! file, is decided by the F2 proof of concept and is not fixed here).
//!
//! A worker is a separate process under a `cfa4-sandbox` profile (`renderWorker`,
//! `externalRender`). The first message each way is a version handshake; a worker that speaks an
//! incompatible version is refused, not tried.

use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::version::ContractVersion;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkerKind {
    RenderWorker,
    ExternalRender,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum WorkerMessage {
    Hello {
        version: ContractVersion,
        kind: WorkerKind,
    },
    HelloAck {
        version: ContractVersion,
    },
    Reject {
        reason: String,
    },
    Submit {
        job: String,
        frame_index: u64,
    },
    Cancel {
        job: String,
    },
    FrameReady {
        job: String,
        frame_index: u64,
    },
    Failed {
        job: String,
        message: String,
    },
    /// Worker to client, once, after `HelloAck`: how this worker's output relates to its input
    /// (ADR "render worker output index", contract 0.2). A worker that sends none is 1:1.
    Describe(Describe),
    /// Worker to client: the output is complete and has exactly `frames` frames. Required for a
    /// worker whose `Describe` has no ratio (unknown length); optional otherwise.
    Finished {
        job: String,
        frames: u64,
    },
}

/// Input frames per output frame as an exact fraction (IVTC 5→4 is `5/4`, a 2× deinterlace `1/2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ratio {
    pub num: u32,
    pub den: u32,
}

/// Why a [`Describe`] is refused or cannot answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DescribeError {
    #[error("ratio with a zero part")]
    ZeroRatio,
    #[error("output frame rate with a non-positive part")]
    BadRate,
    #[error("restart alignment must be at least 1")]
    BadAlignment,
    #[error("the worker declared no ratio: the length is known only from `Finished`")]
    UnknownLength,
    #[error("frame index out of range for the declared ratio")]
    Overflow,
}

/// The shape of a worker's output relative to its input. Every output frame still carries its own
/// `Tick`; nothing here rounds time to a constant frame rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Describe {
    /// `None`: the ratio is not constant, the output has an unknown length until `Finished`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_per_output: Option<Ratio>,
    /// The output frame rate (can differ from the input rate).
    pub output_rate: cfa4_time::FrameRate,
    /// A retry restarts at a multiple of this many output frames (a stateful filter such as an
    /// IVTC pattern recounts from the start of its cycle); 1 means any frame.
    pub restart_alignment: u64,
}

impl Describe {
    /// The description of a worker that sends none: one output frame per input frame, at the input rate.
    pub fn one_to_one(rate: cfa4_time::FrameRate) -> Describe {
        Describe { input_per_output: Some(Ratio { num: 1, den: 1 }), output_rate: rate, restart_alignment: 1 }
    }

    pub fn validate(&self) -> Result<(), DescribeError> {
        if let Some(r) = self.input_per_output
            && (r.num == 0 || r.den == 0)
        {
            return Err(DescribeError::ZeroRatio);
        }
        if self.output_rate.num <= 0 || self.output_rate.den <= 0 {
            return Err(DescribeError::BadRate);
        }
        if self.restart_alignment == 0 {
            return Err(DescribeError::BadAlignment);
        }
        Ok(())
    }

    /// The first input frame output frame `out_index` is built from: `floor(out_index * num / den)`.
    pub fn first_input(&self, out_index: u64) -> Result<u64, DescribeError> {
        self.validate()?;
        let r = self.input_per_output.ok_or(DescribeError::UnknownLength)?;
        let v = u128::from(out_index) * u128::from(r.num) / u128::from(r.den);
        u64::try_from(v).map_err(|_| DescribeError::Overflow)
    }

    /// The number of output frames `inputs` input frames yield: `floor(inputs * den / num)`. This is
    /// the estimate for `RenderProgress::total`; it may move while the job runs.
    pub fn output_count(&self, inputs: u64) -> Result<u64, DescribeError> {
        self.validate()?;
        let r = self.input_per_output.ok_or(DescribeError::UnknownLength)?;
        let v = u128::from(inputs) * u128::from(r.den) / u128::from(r.num);
        u64::try_from(v).map_err(|_| DescribeError::Overflow)
    }

    /// The output frame a retry restarts from, given the first one that is missing.
    pub fn restart_from(&self, missing: u64) -> u64 {
        missing - missing % self.restart_alignment.max(1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HandshakeError {
    #[error("worker speaks contract {theirs:?}, this side {ours:?}")]
    Incompatible { ours: ContractVersion, theirs: ContractVersion },
}

/// The version both sides continue with, or the refusal. Same major; before 1.0 the minor must
/// match as well (see [`ContractVersion::compatible_with`]); the agreed minor is the lower one.
pub fn negotiate(ours: ContractVersion, theirs: ContractVersion) -> Result<ContractVersion, HandshakeError> {
    if ours.compatible_with(theirs) {
        Ok(ContractVersion { major: ours.major, minor: ours.minor.min(theirs.minor) })
    } else {
        Err(HandshakeError::Incompatible { ours, theirs })
    }
}

/// Limits a worker runs under; zero means "unset" and is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceLimits {
    pub timeout_ms: u64,
    pub max_memory_bytes: u64,
    pub max_threads: u32,
}

impl ResourceLimits {
    pub fn is_valid(&self) -> bool {
        self.timeout_ms > 0 && self.max_memory_bytes > 0 && self.max_threads > 0
    }
}

/// Where a worker may read and write: absolute paths under listed roots, no `..`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathPolicy {
    pub allowed_roots: Vec<String>,
}

impl PathPolicy {
    pub fn allows(&self, path: &str) -> bool {
        let p = Path::new(path);
        if !p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
            return false;
        }
        self.allowed_roots.iter().any(|r| {
            let root = Path::new(r);
            root.is_absolute() && !root.components().any(|c| matches!(c, Component::ParentDir)) && p.starts_with(root)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(major: u16, minor: u16) -> ContractVersion {
        ContractVersion { major, minor }
    }

    #[test]
    fn handshake_refuses_a_foreign_version() {
        assert_eq!(negotiate(v(0, 1), v(0, 1)), Ok(v(0, 1)));
        assert!(negotiate(v(0, 1), v(0, 2)).is_err());
        assert_eq!(negotiate(v(1, 3), v(1, 1)), Ok(v(1, 1)));
        assert!(negotiate(v(1, 0), v(2, 0)).is_err());
    }

    #[test]
    fn limits_must_all_be_set() {
        assert!(ResourceLimits { timeout_ms: 1, max_memory_bytes: 1, max_threads: 1 }.is_valid());
        assert!(!ResourceLimits { timeout_ms: 0, max_memory_bytes: 1, max_threads: 1 }.is_valid());
        assert!(!ResourceLimits { timeout_ms: 1, max_memory_bytes: 0, max_threads: 1 }.is_valid());
        assert!(!ResourceLimits { timeout_ms: 1, max_memory_bytes: 1, max_threads: 0 }.is_valid());
    }

    #[test]
    fn paths_stay_inside_the_roots() {
        let p = PathPolicy { allowed_roots: vec!["/tmp/cfa4-work".into()] };
        assert!(p.allows("/tmp/cfa4-work/frames/0001.exr"));
        assert!(!p.allows("/tmp/cfa4-work/../etc/passwd"));
        assert!(!p.allows("/tmp/cfa4-work-evil/x"));
        assert!(!p.allows("relative/path"));
        assert!(!p.allows(""));
        assert!(!PathPolicy::default().allows("/tmp/cfa4-work/x"));
        assert!(!PathPolicy { allowed_roots: vec!["/tmp/../".into()] }.allows("/tmp/x"));
    }

    fn rate(n: i64, d: i64) -> cfa4_time::FrameRate {
        cfa4_time::FrameRate::new(n, d)
    }

    fn d(num: u32, den: u32, align: u64) -> Describe {
        Describe { input_per_output: Some(Ratio { num, den }), output_rate: rate(25, 1), restart_alignment: align }
    }

    #[test]
    fn the_output_index_follows_the_ratio() {
        // 1:1 (the default of a worker that sends no Describe)
        let one = Describe::one_to_one(rate(25, 1));
        assert_eq!((one.first_input(7), one.output_count(48)), (Ok(7), Ok(48)));
        // IVTC 5 -> 4: ten inputs give eight outputs; output n starts at floor(n * 5 / 4)
        let ivtc = d(5, 4, 4);
        assert_eq!(ivtc.output_count(10), Ok(8));
        assert_eq!((0..8).map(|n| ivtc.first_input(n).unwrap()).collect::<Vec<_>>(), [0, 1, 2, 3, 5, 6, 7, 8]);
        // 2x deinterlace: one input gives two outputs
        let dbl = d(1, 2, 2);
        assert_eq!(dbl.output_count(10), Ok(20));
        assert_eq!((0..6).map(|n| dbl.first_input(n).unwrap()).collect::<Vec<_>>(), [0, 0, 1, 1, 2, 2]);
    }

    #[test]
    fn a_retry_restarts_on_the_cycle_boundary() {
        let ivtc = d(5, 4, 4);
        assert_eq!([ivtc.restart_from(0), ivtc.restart_from(3), ivtc.restart_from(4), ivtc.restart_from(7)], [0, 0, 4, 4]);
        assert_eq!(d(1, 1, 1).restart_from(9), 9);
    }

    #[test]
    fn an_unknown_length_is_known_only_from_finished() {
        let unknown = Describe { input_per_output: None, output_rate: rate(25, 1), restart_alignment: 1 };
        assert_eq!(unknown.validate(), Ok(()));
        assert_eq!(unknown.output_count(10), Err(DescribeError::UnknownLength));
        assert_eq!(unknown.first_input(1), Err(DescribeError::UnknownLength));
        let m = WorkerMessage::Finished { job: "j".into(), frames: 17 };
        assert_eq!(serde_json::from_str::<WorkerMessage>(&serde_json::to_string(&m).unwrap()).unwrap(), m);
    }

    /// Hostile input: an error, never a panic or a wrapped number.
    #[test]
    fn a_hostile_describe_is_an_error() {
        assert_eq!(d(0, 1, 1).validate(), Err(DescribeError::ZeroRatio));
        assert_eq!(d(1, 0, 1).first_input(1), Err(DescribeError::ZeroRatio));
        assert_eq!(d(1, 1, 0).validate(), Err(DescribeError::BadAlignment));
        for (n, den) in [(0, 1), (-25, 1), (25, 0), (25, -1)] {
            let bad = Describe { input_per_output: None, output_rate: cfa4_time::FrameRate { num: n, den }, restart_alignment: 1 };
            assert_eq!(bad.validate(), Err(DescribeError::BadRate), "{n}/{den}");
        }
        // u64::MAX * 2 does not fit: Overflow, not a wrap
        assert_eq!(d(2, 1, 1).first_input(u64::MAX), Err(DescribeError::Overflow));
        assert_eq!(d(1, 2, 1).output_count(u64::MAX), Err(DescribeError::Overflow));
    }

    #[test]
    fn describe_round_trips_and_a_worker_without_one_is_one_to_one() {
        let m = WorkerMessage::Describe(d(5, 4, 4));
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<WorkerMessage>(&s).unwrap(), m);
        let none = WorkerMessage::Describe(Describe { input_per_output: None, output_rate: rate(50, 1), restart_alignment: 1 });
        assert!(!serde_json::to_string(&none).unwrap().contains("inputPerOutput"));
        assert_eq!(Describe::one_to_one(rate(24, 1)).validate(), Ok(()));
    }

    #[test]
    fn messages_round_trip_through_json() {
        let m = WorkerMessage::Hello { version: v(0, 1), kind: WorkerKind::RenderWorker };
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<WorkerMessage>(&s).unwrap(), m);
        assert!(serde_json::from_str::<WorkerMessage>(r#"{"type":"nope"}"#).is_err());
    }
}
