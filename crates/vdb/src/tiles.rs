//! The tile file: how voxel data crosses between the OpenVDB worker and the CFX interpreter (plan
//! CFA4-AX-INTERPRETER-001, X2). Layout (little-endian), written by `workers/vdb/main.cc`:
//! a 48 byte header (`CFXT`, version 1, channels 1 or 3, leaf count, voxel size, background of 3 channels),
//! then per leaf its origin (3 x i32), the active mask (8 x u64) and 512 x channels f32 values.
//! Every length is checked against the file size before anything is allocated, so a hostile or truncated file is an
//! [`Error::BadAnswer`], never a panic or a huge allocation.

use std::io::Write;
use std::path::Path;

use crate::Error;

/// Voxels in one leaf (8 x 8 x 8).
pub const LEAF_VOXELS: usize = 512;
const HEADER: usize = 48;
const MAGIC: &[u8; 4] = b"CFXT";

/// One 8x8x8 block of voxels.
#[derive(Debug, Clone, PartialEq)]
pub struct Leaf {
    /// Index-space origin (a multiple of 8 on every axis).
    pub origin: [i32; 3],
    /// Bit `i` is set when voxel `i` (x slowest, then y, then z) is active.
    pub active: [u64; 8],
    /// `LEAF_VOXELS * channels` values.
    pub values: Vec<f32>,
}

impl Leaf {
    /// Whether voxel `i` is active.
    #[must_use]
    pub fn is_active(&self, i: usize) -> bool {
        self.active[(i >> 6) & 7] >> (i & 63) & 1 == 1
    }

    /// The index-space coordinate of voxel `i`.
    #[must_use]
    pub fn coord(&self, i: usize) -> [i32; 3] {
        // `i` is below 512 here; the casts cannot truncate.
        [self.origin[0] + ((i >> 6) & 7) as i32, self.origin[1] + ((i >> 3) & 7) as i32, self.origin[2] + (i & 7) as i32]
    }
}

/// The leaves of one grid.
#[derive(Debug, Clone, PartialEq)]
pub struct TileSet {
    /// 1 for a float grid, 3 for a vec3s grid.
    pub channels: usize,
    /// Voxel size in world units.
    pub voxel_size: f64,
    /// The value of an inactive voxel outside every leaf.
    pub background: [f32; 3],
    /// The leaves, in the order the worker wrote them (OpenVDB's deterministic tree order).
    pub leaves: Vec<Leaf>,
}

fn bad(m: &str) -> Error {
    Error::BadAnswer(m.to_string())
}

fn take<const N: usize>(d: &[u8], at: &mut usize) -> Result<[u8; N], Error> {
    let end = at.checked_add(N).filter(|e| *e <= d.len()).ok_or_else(|| bad("truncated tile file"))?;
    let mut a = [0u8; N];
    a.copy_from_slice(&d[*at..end]);
    *at = end;
    Ok(a)
}

impl TileSet {
    /// Parses a tile file's bytes.
    ///
    /// # Errors
    /// [`Error::BadAnswer`] for a wrong magic, version, channel count or size.
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        if d.len() < HEADER || &d[..4] != MAGIC {
            return Err(bad("not a tile file"));
        }
        let mut at = 4;
        if u32::from_le_bytes(take(d, &mut at)?) != 1 {
            return Err(bad("unsupported tile file version"));
        }
        let channels = u32::from_le_bytes(take(d, &mut at)?) as usize;
        if channels != 1 && channels != 3 {
            return Err(bad("bad channel count"));
        }
        let _pad = take::<4>(d, &mut at)?;
        let n = u64::from_le_bytes(take(d, &mut at)?);
        let voxel_size = f64::from_le_bytes(take(d, &mut at)?);
        let mut background = [0f32; 3];
        for b in &mut background {
            *b = f32::from_le_bytes(take(d, &mut at)?);
        }
        let _pad = take::<4>(d, &mut at)?;
        let per = (12 + 64 + LEAF_VOXELS * channels * 4) as u64;
        let body = (d.len() - HEADER) as u64;
        if n.checked_mul(per) != Some(body) {
            return Err(bad("tile file size does not match its header"));
        }
        let mut leaves = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let mut origin = [0i32; 3];
            for o in &mut origin {
                *o = i32::from_le_bytes(take(d, &mut at)?);
            }
            if origin.iter().any(|o| o & 7 != 0) {
                return Err(bad("tile origin is not a leaf origin"));
            }
            let mut active = [0u64; 8];
            for a in &mut active {
                *a = u64::from_le_bytes(take(d, &mut at)?);
            }
            let mut values = Vec::with_capacity(LEAF_VOXELS * channels);
            for _ in 0..LEAF_VOXELS * channels {
                values.push(f32::from_le_bytes(take(d, &mut at)?));
            }
            leaves.push(Leaf { origin, active, values });
        }
        Ok(Self { channels, voxel_size, background, leaves })
    }

    /// Reads a tile file.
    ///
    /// # Errors
    /// [`Error::BadAnswer`] for a bad file, [`Error::Spawn`] when it cannot be read.
    pub fn read(path: &Path) -> Result<Self, Error> {
        let d = std::fs::read(path).map_err(|e| Error::Spawn(format!("{}: {e}", path.display())))?;
        Self::parse(&d)
    }

    /// The file's bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(HEADER + self.leaves.len() * (76 + LEAF_VOXELS * self.channels * 4));
        o.extend_from_slice(MAGIC);
        o.extend_from_slice(&1u32.to_le_bytes());
        o.extend_from_slice(&(self.channels as u32).to_le_bytes());
        o.extend_from_slice(&0u32.to_le_bytes());
        o.extend_from_slice(&(self.leaves.len() as u64).to_le_bytes());
        o.extend_from_slice(&self.voxel_size.to_le_bytes());
        for b in self.background {
            o.extend_from_slice(&b.to_le_bytes());
        }
        o.extend_from_slice(&0u32.to_le_bytes());
        for l in &self.leaves {
            for c in l.origin {
                o.extend_from_slice(&c.to_le_bytes());
            }
            for w in l.active {
                o.extend_from_slice(&w.to_le_bytes());
            }
            for v in &l.values {
                o.extend_from_slice(&v.to_le_bytes());
            }
        }
        o
    }

    /// Writes the tile file.
    ///
    /// # Errors
    /// [`Error::Spawn`] when the file cannot be written.
    pub fn write(&self, path: &Path) -> Result<(), Error> {
        let mut f = std::fs::File::create(path).map_err(|e| Error::Spawn(format!("{}: {e}", path.display())))?;
        f.write_all(&self.to_bytes()).map_err(|e| Error::Spawn(format!("{}: {e}", path.display())))
    }

    /// Number of active voxels.
    #[must_use]
    pub fn active_voxels(&self) -> u64 {
        self.leaves.iter().map(|l| l.active.iter().map(|w| u64::from(w.count_ones())).sum::<u64>()).sum()
    }
}
