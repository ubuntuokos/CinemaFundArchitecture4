//! The MPEG-TS export profile (CFA4-SCE-001 12.6/2, P2a.1).
//!
//! [`MpegTsProfile`] is the saved, versioned form of the transport stream options: packet
//! format, VBR / CBR, DVB or ATSC, the service metadata and ids, the PIDs, the PCR / PSI / SDT
//! periods, the timestamp mode and the table version. Fields missing from a saved profile take
//! the deterministic CFA4 defaults, and fields a newer CFA4 added are ignored, so old and new
//! profiles keep loading. [`MpegTsProfile::to_params`] turns it into the muxer's
//! [`cfa4_mpegts::TsParams`].
//!
//! The muxer takes the profile in the second slice of P2a (after the T-STD scheduler, #1857).
//! Until then CBR and the `Preserve` timestamp mode validate but are refused with
//! [`ProfileError::NotYetSupported`] rather than silently ignored.

use cfa4_mpegts::{ServiceType, SiError, TsParams};
use serde::{Deserialize, Serialize};

/// The profile format this CFA4 writes.
pub const PROFILE_VERSION: u32 = 1;

/// Packet format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TsOutput {
    /// 188-byte transport packets (`.ts`).
    #[default]
    Ts188,
    /// 192-byte BDAV packets (`.m2ts`).
    M2ts192,
}

/// Multiplex rate mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum TsMuxMode {
    /// Variable rate: no null packets.
    #[default]
    Vbr,
    /// Constant rate, filled with null packets to `mux_rate` bits per second.
    Cbr {
        #[serde(rename = "muxRate")]
        mux_rate: u64,
    },
}

/// Broadcast system the service information follows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TsSystem {
    /// DVB: PAT, PMT and SDT.
    #[default]
    SystemA,
    /// ATSC: PAT and PMT only (PSIP is not written).
    SystemB,
}

/// How the source timestamps reach the stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TsTimestamps {
    /// Start at the muxer's fixed delay (today's behaviour).
    #[default]
    Shift,
    /// Keep the source PTS / DTS.
    Preserve,
}

/// The MPEG-TS export options, as saved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MpegTsProfile {
    pub version: u32,
    pub output: TsOutput,
    pub mux: TsMuxMode,
    pub system: TsSystem,
    pub service_name: String,
    pub service_provider: String,
    pub service_id: u16,
    pub transport_stream_id: u16,
    pub original_network_id: u16,
    pub pmt_pid: u16,
    pub first_es_pid: u16,
    pub pcr_pid: Option<u16>,
    pub pcr_period_ms: u32,
    pub psi_period_ms: u32,
    pub sdt_period_ms: u32,
    pub timestamps: TsTimestamps,
    pub tables_version: u8,
}

impl Default for MpegTsProfile {
    fn default() -> Self {
        let p = TsParams::default();
        MpegTsProfile {
            version: PROFILE_VERSION,
            output: TsOutput::default(),
            mux: TsMuxMode::default(),
            system: TsSystem::default(),
            service_name: p.service_name,
            service_provider: p.service_provider,
            service_id: p.program_number,
            transport_stream_id: p.transport_stream_id,
            original_network_id: p.original_network_id,
            pmt_pid: p.pmt_pid,
            first_es_pid: p.first_es_pid,
            pcr_pid: p.pcr_pid,
            // DVB (TR 101 154): PCR at most 40 ms apart, PAT / PMT at most 0.5 s, SDT at most 2 s
            pcr_period_ms: 40,
            psi_period_ms: 100,
            sdt_period_ms: 1000,
            timestamps: TsTimestamps::default(),
            tables_version: p.tables_version,
        }
    }
}

/// Allowed range of the PCR period, ms (H.222.0: at most 100 ms).
pub const PCR_PERIOD_MS: (u32, u32) = (20, 100);
/// Allowed range of the PAT / PMT period, ms.
pub const PSI_PERIOD_MS: (u32, u32) = (25, 500);
/// Allowed range of the SDT period, ms (EN 300 468: SDT actual at least every 2 s).
pub const SDT_PERIOD_MS: (u32, u32) = (25, 2000);
/// Allowed range of the CBR multiplex rate, bits per second.
pub const CBR_MUX_RATE: (u64, u64) = (1_000_000, 200_000_000);

/// Why a profile cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    #[error("MPEG-TS profile version {0} is newer than this CFA4 understands ({PROFILE_VERSION})")]
    UnknownVersion(u32),
    #[error("{field} is {value}; allowed: {min}-{max}")]
    OutOfRange { field: &'static str, value: u64, min: u64, max: u64 },
    #[error("{0}")]
    Params(#[from] SiError),
    #[error("{0} is not supported by the transport stream muxer yet")]
    NotYetSupported(&'static str),
}

fn in_range(field: &'static str, value: u64, (min, max): (u64, u64)) -> Result<(), ProfileError> {
    if (min..=max).contains(&value) { Ok(()) } else { Err(ProfileError::OutOfRange { field, value, min, max }) }
}

fn ms(r: (u32, u32)) -> (u64, u64) {
    (u64::from(r.0), u64::from(r.1))
}

impl MpegTsProfile {
    /// Read a saved profile; missing fields take the defaults, unknown ones are ignored.
    pub fn from_json(json: &str) -> Result<MpegTsProfile, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The profile as saved, always with the current [`PROFILE_VERSION`].
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.effective())
    }

    /// The values the export uses, for the export log: the profile with its version brought to
    /// [`PROFILE_VERSION`] (a version 1 profile has nothing else to fill in).
    pub fn effective(&self) -> MpegTsProfile {
        MpegTsProfile { version: PROFILE_VERSION, ..self.clone() }
    }

    /// Check every field for a stream of `streams` elementary streams.
    pub fn validate(&self, streams: usize) -> Result<(), ProfileError> {
        if self.version > PROFILE_VERSION {
            return Err(ProfileError::UnknownVersion(self.version));
        }
        in_range("pcrPeriodMs", u64::from(self.pcr_period_ms), ms(PCR_PERIOD_MS))?;
        in_range("psiPeriodMs", u64::from(self.psi_period_ms), ms(PSI_PERIOD_MS))?;
        in_range("sdtPeriodMs", u64::from(self.sdt_period_ms), ms(SDT_PERIOD_MS))?;
        if let TsMuxMode::Cbr { mux_rate } = self.mux {
            in_range("muxRate", mux_rate, CBR_MUX_RATE)?;
        }
        self.params(ServiceType::DigitalTelevision).validate(streams)?;
        Ok(())
    }

    /// The muxer parameters, once the profile is valid and everything it asks for is supported.
    /// `has_video` picks the SDT service type.
    pub fn to_params(&self, streams: usize, has_video: bool) -> Result<TsParams, ProfileError> {
        self.validate(streams)?;
        if matches!(self.mux, TsMuxMode::Cbr { .. }) {
            return Err(ProfileError::NotYetSupported("CBR (null packet filling)"));
        }
        if self.timestamps == TsTimestamps::Preserve {
            return Err(ProfileError::NotYetSupported("keeping the source timestamps"));
        }
        Ok(self.params(if has_video { ServiceType::DigitalTelevision } else { ServiceType::DigitalRadio }))
    }

    fn params(&self, service_type: ServiceType) -> TsParams {
        TsParams {
            transport_stream_id: self.transport_stream_id,
            program_number: self.service_id,
            original_network_id: self.original_network_id,
            pmt_pid: self.pmt_pid,
            first_es_pid: self.first_es_pid,
            pcr_pid: self.pcr_pid,
            service_name: self.service_name.clone(),
            service_provider: self.service_provider.clone(),
            service_type,
            sdt: self.system == TsSystem::SystemA,
            tables_version: self.tables_version,
        }
    }
}
