//! Hardware vendors, from PCI vendor ids.

use serde::{Deserialize, Serialize};

/// The vendor of a GPU or NPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    /// A software rasteriser (Mesa llvmpipe / lavapipe report Mesa's Khronos vendor id).
    Software,
    Other,
}

impl Vendor {
    /// The vendor of a PCI (or Khronos) vendor id. Unknown ids are [`Vendor::Other`].
    pub const fn from_pci(id: u32) -> Self {
        match id {
            0x10de => Vendor::Nvidia,
            // ATI/AMD graphics, and AMD's own id (used by some APU functions and NPUs)
            0x1002 | 0x1022 => Vendor::Amd,
            0x8086 => Vendor::Intel,
            // VK_VENDOR_ID_MESA
            0x10005 => Vendor::Software,
            _ => Vendor::Other,
        }
    }

    /// A short display name.
    pub const fn name(self) -> &'static str {
        match self {
            Vendor::Nvidia => "NVIDIA",
            Vendor::Amd => "AMD",
            Vendor::Intel => "Intel",
            Vendor::Software => "software",
            Vendor::Other => "other",
        }
    }
}

/// Parses a sysfs id such as `0x10de\n`. `None` for anything that is not a hexadecimal `u32`.
pub fn parse_hex_id(text: &str) -> Option<u32> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")).unwrap_or(trimmed);
    // `from_str_radix` would also take a leading `+`; sysfs never writes one
    if digits.is_empty() || digits.len() > 8 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pci_ids_name_the_three_gpu_vendors() {
        assert_eq!(Vendor::from_pci(0x10de), Vendor::Nvidia);
        assert_eq!(Vendor::from_pci(0x1002), Vendor::Amd);
        assert_eq!(Vendor::from_pci(0x1022), Vendor::Amd);
        assert_eq!(Vendor::from_pci(0x8086), Vendor::Intel);
        assert_eq!(Vendor::from_pci(0x10005), Vendor::Software);
        assert_eq!(Vendor::from_pci(0), Vendor::Other);
        assert_eq!(Vendor::from_pci(u32::MAX), Vendor::Other);
    }

    #[test]
    fn sysfs_ids_parse_and_hostile_text_does_not() {
        assert_eq!(parse_hex_id("0x10de\n"), Some(0x10de));
        assert_eq!(parse_hex_id("  0X8086 "), Some(0x8086));
        assert_eq!(parse_hex_id("1002"), Some(0x1002));
        for bad in ["", "0x", "\n", "zz", "0x-1", "0x123456789", "0x1 2", "0x+1", "+10de", "１２", "0x10de0x10de"] {
            assert_eq!(parse_hex_id(bad), None, "{bad:?}");
        }
    }
}
