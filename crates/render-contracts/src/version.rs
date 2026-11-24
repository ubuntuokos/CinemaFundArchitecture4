//! Contract version (semantic: a breaking change raises `major`, an addition raises `minor`).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractVersion {
    pub major: u16,
    pub minor: u16,
}

/// The version this crate implements.
pub const CONTRACT_VERSION: ContractVersion = ContractVersion { major: 0, minor: 1 };

impl ContractVersion {
    /// Whether a peer that speaks `other` can work with this version: the same major version
    /// (before 1.0 a minor change may still break, so a 0.x peer must match the minor too).
    pub fn compatible_with(self, other: ContractVersion) -> bool {
        self.major == other.major && (self.major > 0 || self.minor == other.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn before_one_point_zero_the_minor_must_match() {
        let v = |major, minor| ContractVersion { major, minor };
        assert!(v(0, 1).compatible_with(v(0, 1)));
        assert!(!v(0, 1).compatible_with(v(0, 2)));
        assert!(v(1, 0).compatible_with(v(1, 5)));
        assert!(!v(1, 0).compatible_with(v(2, 0)));
    }
}
