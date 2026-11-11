//! A cheap look at a file's header partition: operational pattern and package identities,
//! without walking the essence (for grouping the OP-Atom files of one clip, ST 390).

use crate::file::OperationalPattern;
use crate::klv::{KeyClass, ber_length, classify, find_header_partition};
use crate::meta::{Metadata, Primer, Set, set_type as st, tag};
use crate::source::{ByteSource, read_upto};
use crate::{Error, Result, Ul};

/// At most this many bytes of header metadata are read.
const MAX_HEADER: u64 = 16 << 20;

/// What the header partition of a file says about its packages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderPeek {
    pub operational_pattern: OperationalPattern,
    /// UMID of the material package (shared by every OP-Atom file of one clip).
    pub material_package: Option<[u8; 32]>,
    /// UMIDs of the file (source) packages with an essence descriptor.
    pub file_packages: Vec<[u8; 32]>,
}

/// The header partition's operational pattern label, primer and local sets (key, value) in file
/// order, read without walking the essence.
pub(crate) fn header_sets(src: &(impl ByteSource + ?Sized)) -> Result<(Option<Ul>, Primer, Vec<(Ul, Vec<u8>)>)> {
    let head = read_upto(src, 0, 65_536 + 32)?;
    let at = find_header_partition(&head).ok_or_else(|| Error::NotMxf("no header partition pack".into()))? as u64;
    let hdr = read_upto(src, at, 25)?;
    let (plen, n) = hdr.get(16..).and_then(ber_length).ok_or_else(|| Error::Invalid("bad partition pack length".into()))?;
    let pack = read_upto(src, at + 16 + n as u64, plen.min(64 << 10) as usize)?;
    let mut c = crate::klv::Cur::new(&pack);
    // major, minor, KAG, this, previous, footer: then HeaderByteCount
    let header_bytes = (|| {
        c.u16()?;
        c.u16()?;
        c.u32()?;
        c.u64()?;
        c.u64()?;
        c.u64()?;
        c.u64()
    })()
    .ok_or_else(|| Error::Invalid("short partition pack".into()))?;
    let op_label = pack.get(64..80).and_then(crate::klv::ul_of);
    let start = at + 16 + n as u64 + plen;
    // header_bytes counts from the end of the partition pack (fill included); a writer that left
    // it zero still has the metadata here, so read a bounded window and stop at the first essence
    let want = if header_bytes > 0 { header_bytes.min(MAX_HEADER) } else { 1 << 20 };
    let region = read_upto(src, start, want as usize)?;
    let mut primer = Primer::default();
    let mut sets = Vec::new();
    let mut pos = 0usize;
    while let Some(key) = region.get(pos..pos + 16) {
        let key = Ul(key.try_into().unwrap_or([0; 16]));
        let Some((vlen, n)) = region.get(pos + 16..).and_then(ber_length) else { break };
        let vstart = pos + 16 + n;
        let Some(vend) = usize::try_from(vlen).ok().and_then(|l| vstart.checked_add(l)) else { break };
        let Some(v) = region.get(vstart..vend) else { break };
        match classify(&key) {
            KeyClass::Primer => primer = Primer::parse(v),
            KeyClass::LocalSet => sets.push((key, v.to_vec())),
            KeyClass::Fill => {}
            _ => break,
        }
        pos = vend;
    }
    Ok((op_label, primer, sets))
}

/// Read the header partition pack and header metadata only.
pub fn peek(src: &(impl ByteSource + ?Sized)) -> Result<HeaderPeek> {
    let (op_label, primer, sets) = header_sets(src)?;
    let mut md = Metadata::default();
    for (key, v) in &sets {
        md.push(Set::parse(*key, v, &primer));
    }
    let preface_op = md.of_kind(st::PREFACE).next().and_then(|p| p.ul(tag::OPERATIONAL_PATTERN));
    let umid = |s: &Set| s.get(tag::PACKAGE_UID).and_then(|v| v.get(..32)).and_then(|b| b.try_into().ok());
    Ok(HeaderPeek {
        operational_pattern: preface_op.or(op_label).map_or(OperationalPattern::Unknown, |l| OperationalPattern::from_ul(&l)),
        material_package: md.of_kind(st::MATERIAL_PACKAGE).next().and_then(umid),
        file_packages: md.of_kind(st::SOURCE_PACKAGE).filter(|p| p.reference(tag::SOURCE_PACKAGE_DESCRIPTOR).is_some()).filter_map(umid).collect(),
    })
}
