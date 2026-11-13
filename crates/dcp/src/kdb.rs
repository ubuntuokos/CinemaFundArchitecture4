//! Key Delivery Bundles (SMPTE ST 430-9:2008 with Amendment 1:2011): many KDMs in one POSIX ustar
//! archive, with a `CATALOG` XML file (namespace `http://www.smpte-ra.org/schemas/429-10/2008/KDMB`)
//! that lists each KDM file of the `CONTENT` directory with its composition, recipient and key
//! validity. Other top-level entries are ignored (ST 430-9 6).
//!
//! The archive is read from memory with limits: regular files only (no links or devices),
//! relative paths without `..`, at most [`MAX_FILES`] files of at most [`MAX_KDM_FILE`] bytes.

use roxmltree::Node;

use crate::Limits;
use crate::kdm::KdmError;

/// Largest bundle read (bytes).
pub const MAX_BUNDLE: u64 = 64 << 20;
/// Most KDM files taken from one bundle.
pub const MAX_FILES: usize = 4096;
/// Largest KDM file in a bundle (bytes).
pub const MAX_KDM_FILE: usize = 4 << 20;
/// Largest CATALOG file (bytes).
const MAX_CATALOG: usize = 8 << 20;

/// One KDM of a bundle: what the catalog says about it, and its text.
#[derive(Clone, Debug)]
pub struct BundleKdm {
    /// Path relative to `CONTENT` (the catalog's `FilePath`), or the archive path of a KDM the
    /// catalog does not list.
    pub path: String,
    /// The catalog's `CPLId`, `ContentKeysNotValidBefore` / `After` and recipient subject.
    pub cpl_id: Option<String>,
    pub not_before: Option<String>,
    pub not_after: Option<String>,
    pub recipient: Option<String>,
    /// The KDM document.
    pub xml: String,
}

/// A read Key Delivery Bundle.
#[derive(Clone, Debug, Default)]
pub struct Bundle {
    pub id: Option<String>,
    pub annotation: Option<String>,
    pub creator: Option<String>,
    /// The KDMs, in catalog order, then any `CONTENT` file the catalog does not list.
    pub kdms: Vec<BundleKdm>,
    /// What did not add up: catalog entries without a file, unreadable files.
    pub issues: Vec<String>,
}

/// Whether `head` (the first 512 bytes or more) starts a POSIX ustar archive.
pub fn is_ustar(head: &[u8]) -> bool {
    head.get(257..262) == Some(b"ustar")
}

/// One regular file of a ustar archive.
struct Entry<'a> {
    path: String,
    data: &'a [u8],
}

fn field(h: &[u8], at: usize, len: usize) -> &[u8] {
    let f = h.get(at..at + len).unwrap_or_default();
    let end = f.iter().position(|&b| b == 0).unwrap_or(f.len());
    f.get(..end).unwrap_or_default()
}

fn octal(h: &[u8], at: usize, len: usize) -> Option<u64> {
    let f = field(h, at, len);
    let t = std::str::from_utf8(f).ok()?.trim();
    if t.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(t, 8).ok()
}

/// The regular files of a ustar archive (ST 430-9 5), at most `MAX_FILES + 1`; an archive cut
/// short at a header ends there.
fn entries<'a>(tar: &'a [u8], issues: &mut Vec<String>) -> Result<Vec<Entry<'a>>, KdmError> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(h) = tar.get(at..at.saturating_add(512)) {
        if h.iter().all(|&b| b == 0) {
            break;
        }
        if !is_ustar(h) {
            return Err(KdmError::Invalid(format!("the bundle is not a ustar archive (header at byte {at})")));
        }
        // the header checksum: the header's bytes summed with the checksum field as spaces
        let want = octal(h, 148, 8).ok_or_else(|| KdmError::Invalid("bad ustar checksum field".into()))?;
        let sum: u64 = h.iter().enumerate().map(|(i, &b)| if (148..156).contains(&i) { 32 } else { u64::from(b) }).sum();
        if sum != want {
            return Err(KdmError::Invalid(format!("ustar header at byte {at} is damaged (checksum)")));
        }
        let size = octal(h, 124, 12).ok_or_else(|| KdmError::Invalid("bad ustar size field".into()))?;
        let size = usize::try_from(size).map_err(|_| KdmError::Invalid("ustar entry too large".into()))?;
        let data_at = at.checked_add(512).ok_or_else(|| KdmError::Invalid("ustar offset overflows".into()))?;
        let data = tar.get(data_at..data_at.saturating_add(size)).ok_or_else(|| KdmError::Invalid("the bundle is cut short".into()))?;
        let name = String::from_utf8_lossy(field(h, 0, 100)).into_owned();
        let prefix = String::from_utf8_lossy(field(h, 345, 155)).into_owned();
        let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let path = path.trim_start_matches("./").to_string();
        match h.get(156).copied().unwrap_or(0) {
            b'0' | 0 => {
                if path.starts_with('/') || path.split('/').any(|c| c == "..") {
                    issues.push(format!("{path}: not a relative path, ignored"));
                } else if out.len() > MAX_FILES {
                    return Err(KdmError::Invalid(format!("the bundle holds more than {MAX_FILES} files")));
                } else {
                    out.push(Entry { path, data });
                }
            }
            b'5' | b'x' | b'g' => {}
            _ => issues.push(format!("{path}: not a regular file, ignored")),
        }
        let blocks = size.div_ceil(512).checked_mul(512).ok_or_else(|| KdmError::Invalid("ustar size overflows".into()))?;
        at = data_at.checked_add(blocks).ok_or_else(|| KdmError::Invalid("ustar offset overflows".into()))?;
    }
    Ok(out)
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn text(n: Option<Node>) -> Option<String> {
    n.and_then(|n| n.text()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// Reads a Key Delivery Bundle from the bytes of its archive.
pub fn read_bundle(tar: &[u8]) -> Result<Bundle, KdmError> {
    if tar.len() as u64 > MAX_BUNDLE {
        return Err(KdmError::Invalid(format!("the bundle is over {MAX_BUNDLE} bytes")));
    }
    let mut b = Bundle::default();
    let files = entries(tar, &mut b.issues)?;
    let catalog = files.iter().find(|e| e.path == "CATALOG").ok_or_else(|| KdmError::NotKdm("a ustar archive without a CATALOG file".into()))?;
    if catalog.data.len() > MAX_CATALOG {
        return Err(KdmError::Invalid("the CATALOG file is too large".into()));
    }
    let catalog_text = std::str::from_utf8(catalog.data).map_err(|_| KdmError::Invalid("the CATALOG file is not UTF-8".into()))?;
    let doc = crate::xml::parse(catalog_text, Limits { max_bytes: MAX_CATALOG as u64, max_nodes: 2_000_000 })?;
    let root = doc.root_element();
    if root.tag_name().name() != "Catalog" {
        return Err(KdmError::NotKdm(format!("the CATALOG root element is {}", root.tag_name().name())));
    }
    b.id = text(child(root, "Id"));
    b.annotation = text(child(root, "AnnotationText"));
    b.creator = text(child(root, "Creator"));
    let content = |p: &str| files.iter().find(|e| e.path == format!("CONTENT/{p}"));
    let read = |e: &Entry, issues: &mut Vec<String>| -> Option<String> {
        if e.data.len() > MAX_KDM_FILE {
            issues.push(format!("{}: over {MAX_KDM_FILE} bytes, skipped", e.path));
            return None;
        }
        match std::str::from_utf8(e.data) {
            Ok(t) => Some(t.to_string()),
            Err(_) => {
                issues.push(format!("{}: not UTF-8 text, skipped", e.path));
                None
            }
        }
    };
    let mut listed = Vec::new();
    for f in child(root, "KDMFileList").into_iter().flat_map(|l| l.children()).filter(|c| c.is_element() && c.tag_name().name() == "KDMFile") {
        let Some(path) = text(child(f, "FilePath")) else {
            b.issues.push("a KDMFile without a FilePath".into());
            continue;
        };
        let Some(e) = content(&path) else {
            b.issues.push(format!("CONTENT/{path}: listed in the CATALOG but not in the bundle"));
            continue;
        };
        listed.push(e.path.clone());
        let Some(xml) = read(e, &mut b.issues) else { continue };
        let recipient = child(f, "Recipient").and_then(|r| text(child(r, "X509SubjectName")));
        b.kdms.push(BundleKdm {
            path,
            cpl_id: text(child(f, "CPLId")),
            not_before: text(child(f, "ContentKeysNotValidBefore")),
            not_after: text(child(f, "ContentKeysNotValidAfter")),
            recipient,
            xml,
        });
    }
    // files in CONTENT the catalog does not list are still KDMs (ST 430-9 6.2)
    for e in files.iter().filter(|e| e.path.starts_with("CONTENT/") && !listed.contains(&e.path)) {
        let Some(xml) = read(e, &mut b.issues) else { continue };
        let path = e.path.trim_start_matches("CONTENT/").to_string();
        b.kdms.push(BundleKdm { path, cpl_id: None, not_before: None, not_after: None, recipient: None, xml });
    }
    if b.kdms.is_empty() {
        return Err(KdmError::NotKdm("the bundle holds no KDM".into()));
    }
    Ok(b)
}

#[cfg(test)]
mod tests;
