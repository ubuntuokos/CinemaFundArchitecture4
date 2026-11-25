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
    Hello { version: ContractVersion, kind: WorkerKind },
    HelloAck { version: ContractVersion },
    Reject { reason: String },
    Submit { job: String, frame_index: u64 },
    Cancel { job: String },
    FrameReady { job: String, frame_index: u64 },
    Failed { job: String, message: String },
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

    #[test]
    fn messages_round_trip_through_json() {
        let m = WorkerMessage::Hello { version: v(0, 1), kind: WorkerKind::RenderWorker };
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<WorkerMessage>(&s).unwrap(), m);
        assert!(serde_json::from_str::<WorkerMessage>(r#"{"type":"nope"}"#).is_err());
    }
}
