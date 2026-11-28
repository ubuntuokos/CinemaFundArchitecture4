//! The index state of one media asset (`mi.asset.index_state`).

use serde::{Deserialize, Serialize};

use crate::MiError;

/// Where an asset is in the index. An asset starts as `PendingHash` (the background indexer has not
/// computed its content hash yet, so the `.fcproj` `MediaIdentity` stays the relink key), then
/// waits for its stages, is indexed, or has failed (the reason is in the task, never silently dropped).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexState {
    PendingHash,
    Pending,
    Indexing,
    Indexed,
    Failed,
}

impl IndexState {
    /// The text stored in `mi.asset.index_state`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PendingHash => "pending_hash",
            Self::Pending => "pending",
            Self::Indexing => "indexing",
            Self::Indexed => "indexed",
            Self::Failed => "failed",
        }
    }

    /// Parse the stored text; anything else is an error value.
    pub fn parse(text: &str) -> Result<Self, MiError> {
        Ok(match text {
            "pending_hash" => Self::PendingHash,
            "pending" => Self::Pending,
            "indexing" => Self::Indexing,
            "indexed" => Self::Indexed,
            "failed" => Self::Failed,
            _ => return Err(MiError::InvalidState("unknown index state")),
        })
    }

    /// True while a hash is not known: such an asset has no `content_hash`.
    pub fn needs_hash(self) -> bool {
        self == Self::PendingHash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_round_trips_and_garbage_is_an_error() {
        for s in [IndexState::PendingHash, IndexState::Pending, IndexState::Indexing, IndexState::Indexed, IndexState::Failed] {
            assert_eq!(IndexState::parse(s.as_str()).unwrap(), s);
        }
        for bad in ["", "Indexed", "pending ", "done", "nul\0"] {
            assert!(IndexState::parse(bad).is_err(), "{bad:?}");
        }
        assert!(IndexState::PendingHash.needs_hash() && !IndexState::Pending.needs_hash());
    }
}
