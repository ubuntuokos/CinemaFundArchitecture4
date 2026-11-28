//! Transport stream identifiers and DVB service information (CFA4-SCE-001 P2a.1).
//!
//! [`TsParams`] holds what a transport stream muxer needs to name its program: the
//! transport stream / program / network ids, the PMT and elementary stream PIDs, and the service
//! name and provider for the DVB Service Description Table (SDT actual, ETSI EN 300 468 §5.2.3).
//! Its default is exactly what [`crate::Muxer`] writes today (transport stream 1, program 1,
//! PMT on 0x1000, elementary streams from 0x100, no SDT).
//!
//! [`sdt_section`] builds the SDT section and [`sdt_packets`] splits it into 188-byte packets
//! on PID 0x0011. Nothing here touches the muxer yet; it is wired in by the second slice of P2a
//! ([`docs/plans/CFA4-SCE-001-P2A-PROFILE-SDT-2026-11-27.md`]).

use crate::ts::crc32_mpeg;

/// The PAT's PID.
pub const PAT_PID: u16 = 0x0000;
/// The DVB SDT / BAT PID.
pub const SDT_PID: u16 = 0x0011;
/// The null packet PID.
pub const NULL_PID: u16 = 0x1FFF;
/// Lowest PID a program may use: 0x0000-0x000F are MPEG tables, 0x0010-0x001F DVB SI.
pub const FIRST_FREE_PID: u16 = 0x0020;
/// Highest PID a program may use: 0x1FFB is ATSC's base PID, 0x1FFC-0x1FFE are reserved by
/// some systems, 0x1FFF is the null packet.
pub const LAST_FREE_PID: u16 = 0x1FFA;
/// Longest service name plus provider name, in bytes as written: the service descriptor's
/// 8-bit length also counts the service type and the two name lengths.
pub const MAX_NAME_BYTES: usize = 252;

/// The service_type of the service descriptor (EN 300 468 table 87).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceType {
    /// 0x01: digital television (any video stream).
    DigitalTelevision,
    /// 0x02: digital radio sound (audio only).
    DigitalRadio,
}

impl ServiceType {
    fn code(self) -> u8 {
        match self {
            ServiceType::DigitalTelevision => 0x01,
            ServiceType::DigitalRadio => 0x02,
        }
    }
}

/// Identifiers of a single-program transport stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TsParams {
    pub transport_stream_id: u16,
    /// The program_number in the PAT and PMT, and the service_id in the SDT.
    pub program_number: u16,
    pub original_network_id: u16,
    pub pmt_pid: u16,
    /// The first elementary stream's PID; the others follow one by one.
    pub first_es_pid: u16,
    /// A PID of its own for the PCR, or `None` for the first video (else first) stream's.
    pub pcr_pid: Option<u16>,
    pub service_name: String,
    pub service_provider: String,
    pub service_type: ServiceType,
    /// Write an SDT (DVB, "System A"); ATSC ("System B") streams have none.
    pub sdt: bool,
    /// version_number of the PAT, PMT and SDT (0-31).
    pub tables_version: u8,
}

impl Default for TsParams {
    fn default() -> Self {
        TsParams {
            transport_stream_id: 1,
            program_number: 1,
            // ETSI TS 101 162: 0xFF01-0xFFFF are for temporary private use
            original_network_id: 0xFF01,
            pmt_pid: 0x1000,
            first_es_pid: 0x0100,
            pcr_pid: None,
            service_name: "CFA4".into(),
            service_provider: "CFA4".into(),
            service_type: ServiceType::DigitalTelevision,
            sdt: false,
            tables_version: 0,
        }
    }
}

/// Why [`TsParams`] cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SiError {
    #[error("{what} PID 0x{pid:04X} is outside 0x{FIRST_FREE_PID:04X}-0x{LAST_FREE_PID:04X}")]
    PidOutOfRange { what: &'static str, pid: u16 },
    #[error("{what} PID 0x{pid:04X} is already used by {other}")]
    PidCollision { what: &'static str, other: &'static str, pid: u16 },
    #[error("a transport stream needs 1 to 16 elementary streams, not {0}")]
    StreamCount(usize),
    #[error("program_number 0 is reserved for the network PID")]
    ProgramNumberZero,
    #[error("tables_version {0} does not fit in 5 bits (0-31)")]
    Version(u8),
    #[error("the service name and provider are {0} bytes together as written; at most {MAX_NAME_BYTES} are allowed")]
    NamesTooLong(usize),
    #[error("the {what} contains a control character")]
    NameControl { what: &'static str },
}

impl TsParams {
    /// Check the parameters for a transport stream of `streams` elementary streams.
    pub fn validate(&self, streams: usize) -> Result<(), SiError> {
        if streams == 0 || streams > 16 {
            return Err(SiError::StreamCount(streams));
        }
        if self.program_number == 0 {
            return Err(SiError::ProgramNumberZero);
        }
        if self.tables_version > 31 {
            return Err(SiError::Version(self.tables_version));
        }
        let free = FIRST_FREE_PID..=LAST_FREE_PID;
        if !free.contains(&self.pmt_pid) {
            return Err(SiError::PidOutOfRange { what: "PMT", pid: self.pmt_pid });
        }
        // `streams` ≤ 16, so the sum cannot overflow u32
        let last_es = u32::from(self.first_es_pid) + streams as u32 - 1;
        if !free.contains(&self.first_es_pid) || last_es > u32::from(LAST_FREE_PID) {
            return Err(SiError::PidOutOfRange { what: "elementary stream", pid: self.first_es_pid });
        }
        let es = u32::from(self.first_es_pid)..=last_es;
        if es.contains(&u32::from(self.pmt_pid)) {
            return Err(SiError::PidCollision { what: "PMT", other: "an elementary stream", pid: self.pmt_pid });
        }
        if let Some(pcr) = self.pcr_pid {
            if !free.contains(&pcr) {
                return Err(SiError::PidOutOfRange { what: "PCR", pid: pcr });
            }
            if pcr == self.pmt_pid {
                return Err(SiError::PidCollision { what: "PCR", other: "the PMT", pid: pcr });
            }
            // a PCR on an elementary stream's PID is allowed: that stream then carries it
        }
        for (what, text) in [("service name", &self.service_name), ("service provider", &self.service_provider)] {
            if text.chars().any(char::is_control) {
                return Err(SiError::NameControl { what });
            }
        }
        let len = text_len(&self.service_name).saturating_add(text_len(&self.service_provider));
        if len > MAX_NAME_BYTES {
            return Err(SiError::NamesTooLong(len));
        }
        Ok(())
    }
}

/// Bytes of a DVB string as written, without building it: the text itself, plus the 0x15
/// selector when it is not printable ASCII.
fn text_len(s: &str) -> usize {
    if s.bytes().all(|b| (0x20..0x7F).contains(&b)) { s.len() } else { s.len() + 1 }
}

/// A DVB string (EN 300 468 annex A): printable ASCII as it is (the default table's ASCII
/// subset), anything else as UTF-8 after the 0x15 selector. The caller has checked the length
/// ([`TsParams::validate`]), so what is allocated is bounded by [`MAX_NAME_BYTES`].
fn encode_text(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(text_len(s));
    if text_len(s) != s.len() {
        v.push(0x15);
    }
    v.extend_from_slice(s.as_bytes());
    v
}

/// The SDT actual section (table_id 0x42) of the one service, CRC-32 included. `streams` is the
/// number of elementary streams, for the validation.
pub fn sdt_section(params: &TsParams, streams: usize) -> Result<Vec<u8>, SiError> {
    params.validate(streams)?;
    let provider = encode_text(&params.service_provider);
    let name = encode_text(&params.service_name);
    // service_descriptor: tag, length, service_type, provider, name
    let mut desc = vec![0x48, 0, params.service_type.code(), provider.len() as u8];
    desc.extend_from_slice(&provider);
    desc.push(name.len() as u8);
    desc.extend_from_slice(&name);
    // validate() keeps the names to MAX_NAME_BYTES together, so the body fits its 8-bit length
    let body = desc.len() - 2;
    let Ok(body) = u8::try_from(body) else {
        return Err(SiError::NamesTooLong(body));
    };
    if let Some(b) = desc.get_mut(1) {
        *b = body;
    }
    let tsid = params.transport_stream_id.to_be_bytes();
    let onid = params.original_network_id.to_be_bytes();
    let sid = params.program_number.to_be_bytes();
    let mut sec = vec![
        0x42,
        0xF0,
        0,
        tsid[0],
        tsid[1],
        0xC1 | (params.tables_version << 1),
        0, // section_number
        0, // last_section_number
        onid[0],
        onid[1],
        0xFF, // reserved_future_use
        sid[0],
        sid[1],
        0xFC, // reserved_future_use, no EIT schedule, no EIT present/following
    ];
    // running_status 4 (running), free_CA_mode 0, descriptors_loop_length
    let dl = desc.len();
    sec.push(0x80 | ((dl >> 8) as u8 & 0x0F));
    sec.push(dl as u8);
    sec.extend_from_slice(&desc);
    // section_length counts from after the field to the end of the CRC (≤ 1021)
    let len = sec.len() - 3 + 4;
    if let Some(b) = sec.get_mut(1) {
        *b = 0xF0 | ((len >> 8) as u8 & 0x0F);
    }
    if let Some(b) = sec.get_mut(2) {
        *b = len as u8;
    }
    let crc = crc32_mpeg(&sec);
    sec.extend_from_slice(&crc.to_be_bytes());
    Ok(sec)
}

/// The SDT as 188-byte transport packets on [`SDT_PID`], starting at continuity counter `*cc`
/// (advanced past the last packet). The first packet carries the payload unit start indicator
/// and a zero pointer field; the last is stuffed with 0xFF.
pub fn sdt_packets(params: &TsParams, streams: usize, cc: &mut u8) -> Result<Vec<[u8; 188]>, SiError> {
    let sec = sdt_section(params, streams)?;
    let mut payload = Vec::with_capacity(sec.len() + 1);
    payload.push(0); // pointer_field
    payload.extend_from_slice(&sec);
    let mut out = Vec::new();
    for (k, chunk) in payload.chunks(184).enumerate() {
        let mut p = [0xFFu8; 188];
        p[0] = 0x47;
        p[1] = (if k == 0 { 0x40 } else { 0 }) | ((SDT_PID >> 8) as u8 & 0x1F);
        p[2] = SDT_PID as u8;
        p[3] = 0x10 | (*cc & 0x0F);
        *cc = ((*cc & 0x0F) + 1) & 0x0F;
        if let Some(dst) = p.get_mut(4..4 + chunk.len()) {
            dst.copy_from_slice(chunk);
        }
        out.push(p);
    }
    Ok(out)
}
