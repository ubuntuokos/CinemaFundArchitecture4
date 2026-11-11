//! OP-Atom clips spread over several files (SMPTE ST 390: one essence track per file): Avid
//! MediaFiles folders and P2 cards (`CONTENTS/VIDEO` + `CONTENTS/AUDIO`). The files of one clip
//! share the material package UMID; they are found beside the file asked for (and, on a P2 card,
//! in the sibling `VIDEO` / `AUDIO` folder) and opened as one source.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use crate::Services;

/// At most this many MXF files of one folder are looked at.
const MAX_FILES_PER_DIR: usize = 4096;

/// The host's random-access reader as an MXF byte source.
struct Reader(cfa4_media::SharedReader);

impl cfa4_mxf::ByteSource for Reader {
    fn len(&self) -> u64 {
        self.0.len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        self.0.read_at(offset, buf)
    }
}

/// Material package of each OP-Atom file of a folder, remembered while the folder's listing is
/// unchanged (a project opens many clips of one Avid folder).
#[derive(Default)]
pub(crate) struct FolderCache(Mutex<HashMap<String, Folder>>);

struct Folder {
    names: Vec<String>,
    /// (path, material package UMID) of the OP-Atom files.
    atoms: Vec<(String, [u8; 32])>,
}

fn is_mxf(name: &str) -> bool {
    Path::new(name).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("mxf"))
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_string() } else { Path::new(dir).join(name).to_string_lossy().to_string() }
}

fn dir_of(path: &str) -> String {
    Path::new(path).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
}

fn name_of(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// The material package of an OP-Atom file.
fn atom_material(path: &str, services: &dyn Services) -> Option<[u8; 32]> {
    let r = services.reader(path)?.ok()?;
    let p = cfa4_mxf::peek(&Reader(r)).ok()?;
    (p.operational_pattern == cfa4_mxf::OperationalPattern::Atom).then_some(p.material_package?)
}

impl FolderCache {
    /// The OP-Atom files of folder `dir` with their material packages.
    fn atoms(&self, dir: &str, services: &dyn Services) -> Vec<(String, [u8; 32])> {
        let Some(Ok(mut names)) = services.list_dir(dir) else { return Vec::new() };
        names.retain(|n| is_mxf(n));
        names.sort();
        names.truncate(MAX_FILES_PER_DIR);
        let mut cache = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(f) = cache.get(dir)
            && f.names == names
        {
            return f.atoms.clone();
        }
        let atoms: Vec<(String, [u8; 32])> = names
            .iter()
            .filter_map(|n| {
                let p = join(dir, n);
                atom_material(&p, services).map(|m| (p, m))
            })
            .collect();
        cache.insert(dir.to_string(), Folder { names, atoms: atoms.clone() });
        atoms
    }

    /// The files of the clip `path` belongs to, in name order (picture folder first on a P2
    /// card), and the index of `path` among them. `None` when `path` is not an OP-Atom file or no
    /// other file shares its material package.
    pub(crate) fn group(&self, path: &str, services: &dyn Services) -> Option<(Vec<String>, usize)> {
        if !is_mxf(path) {
            return None;
        }
        let material = atom_material(path, services)?;
        let dir = dir_of(path);
        let mut dirs = vec![dir.clone()];
        // P2: CONTENTS/VIDEO/<clip>.MXF and CONTENTS/AUDIO/<clip>NN.MXF
        let parent = name_of(&dir);
        let contents = dir_of(&dir);
        if (parent.eq_ignore_ascii_case("video") || parent.eq_ignore_ascii_case("audio"))
            && name_of(&contents).eq_ignore_ascii_case("contents")
            && let Some(Ok(subs)) = services.list_dir(&contents)
        {
            let mut subs: Vec<String> = subs.into_iter().filter(|s| s.eq_ignore_ascii_case("video") || s.eq_ignore_ascii_case("audio")).collect();
            // VIDEO before AUDIO
            subs.sort_by_key(|s| !s.eq_ignore_ascii_case("video"));
            dirs = subs.iter().map(|s| join(&contents, s)).collect();
            if !dirs.contains(&dir) {
                dirs.push(dir.clone());
            }
        }
        let mut members: Vec<String> = Vec::new();
        for d in &dirs {
            members.extend(self.atoms(d, services).into_iter().filter(|(_, m)| *m == material).map(|(p, _)| p));
        }
        if !members.iter().any(|p| p == path) {
            members.push(path.to_string());
        }
        if members.len() < 2 {
            return None;
        }
        let asked = members.iter().position(|p| p == path)?;
        Some((members, asked))
    }
}
