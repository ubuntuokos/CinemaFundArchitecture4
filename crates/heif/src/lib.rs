//! Clean-room HEIF / HEIC still-image container reader.
//!
//! Reads the item structure of a HEIF file (ISO/IEC 23008-12 on the ISO base media file format,
//! ISO/IEC 14496-12): the `meta` box with its items (`iinf`), their data (`iloc`, `idat`), their
//! properties (`iprp`: `hvcC`, `ispe`, `colr`, `irot`, `imir`, `clap`, `auxC`) and references
//! (`iref`: `dimg`, `auxl`). [`parse`] resolves the primary image into what a decoder needs:
//!
//! - an HEVC-coded image (`hvc1`: the `hvcC` record and the length-prefixed NAL units),
//! - an image grid (`grid`: tiles in raster order, cropped to the output size),
//! - an identity-derived image (`iden`), which adds its transforms to its source,
//! - or an item type it does not decode (reported, so the caller can say what it is),
//!
//! with its transforms in the order the file lists them and its alpha plane (an auxiliary image
//! with the alpha URN). Nothing is decoded here; the HEVC bitstreams go to `cfa4-hevc`.
//!
//! The file is hostile: every count and size is checked against the bytes present, item and
//! tile counts are capped, derivation depth is bounded, and nothing panics.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable, clippy::indexing_slicing)
)]

use std::collections::HashMap;

/// Most items read from one file.
pub const MAX_ITEMS: usize = 65_536;
/// Most tiles in one grid (rows and columns are at most 256 each by syntax).
pub const MAX_TILES: usize = 65_536;
/// Deepest chain of derived images (`iden` of `grid` of …).
pub const MAX_DEPTH: usize = 8;
/// Most images one resolution builds (tiles, their alpha and derivations, counted each time
/// they are referenced): nested grids referring to the same items would otherwise multiply.
pub const MAX_IMAGES: usize = MAX_TILES + 1024;
/// Item data one resolution may copy beyond the source's own size: repeated or overlapping
/// extents and references to one item would otherwise copy the same bytes without limit.
pub const ITEM_DATA_SLACK: usize = 64 << 20;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("not a HEIF file: {0}")]
    NotHeif(&'static str),
    #[error("damaged HEIF file: {0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn invalid(what: impl Into<String>) -> Error {
    Error::Invalid(what.into())
}

/// Colour information from a `colr` property.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Colour {
    /// ITU-T H.273 code points.
    Nclx { primaries: u16, transfer: u16, matrix: u16, full_range: bool },
    /// An ICC profile (`prof` or `rICC`).
    Icc(Vec<u8>),
}

/// The clean aperture (`clap`) as fractions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanAperture {
    pub width: (u32, u32),
    pub height: (u32, u32),
    pub horizontal_offset: (i32, u32),
    pub vertical_offset: (i32, u32),
}

impl CleanAperture {
    /// The crop rectangle (left, top, width, height) in a `width`×`height` image, clamped to it.
    /// The centre is `offset + (size - 1) / 2` (ISO/IEC 14496-12 §12.1.4); the left / top edge and
    /// the size are rounded to the nearest whole pixel, halves up (libheif gives the same). `None` for a zero denominator or an
    /// empty rectangle.
    pub fn rect(&self, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
        let edge = |size: u32, clean: (u32, u32), offset: (i32, u32)| -> Option<(u32, u32)> {
            let (cn, cd) = (i128::from(clean.0), i128::from(clean.1));
            let (on, od) = (i128::from(offset.0), i128::from(offset.1));
            if cd == 0 || od == 0 {
                return None;
            }
            // left = offset + (size - 1) / 2 - (clean - 1) / 2, over the denominator 2·cd·od,
            // rounded half up
            let den = 2 * cd * od;
            let num = 2 * on * cd + (i128::from(size) - 1) * cd * od - (cn - cd) * od;
            let left = (2 * num + den).div_euclid(2 * den);
            let clean_px = (2 * cn + cd).div_euclid(2 * cd);
            let right = (left + clean_px).clamp(0, i128::from(size));
            let left = left.clamp(0, i128::from(size));
            let w = right - left;
            if w <= 0 {
                return None;
            }
            Some((u32::try_from(left).ok()?, u32::try_from(w).ok()?))
        };
        let (x, w) = edge(width, self.width, self.horizontal_offset)?;
        let (y, h) = edge(height, self.height, self.vertical_offset)?;
        Some((x, y, w, h))
    }
}

/// A transform applied to a decoded image, in the order the file lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    Crop(CleanAperture),
    /// Anticlockwise quarter turns (0–3).
    Rotate(u8),
    /// Swap left and right when `left_right`, else top and bottom. `imir` mode 0 swaps top and
    /// bottom and mode 1 left and right, as libheif and ISO/IEC 23008-12:2022 read it.
    Mirror {
        left_right: bool,
    },
}

/// How an image's pixels are coded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Coding {
    /// HEVC: the `hvcC` decoder configuration record and the item data (length-prefixed NAL
    /// units).
    Hevc { hvcc: Vec<u8>, data: Vec<u8> },
    /// A grid of tiles in raster order, `columns` per row, cropped to `width`×`height`.
    Grid { rows: u32, columns: u32, width: u32, height: u32, tiles: Vec<Image> },
    /// An item type this reader does not resolve (e.g. `av01`, `jpeg`, `unci`).
    Other([u8; 4]),
}

/// A resolved image: its coding, size, colour, transforms and alpha.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub item: u32,
    pub coding: Coding,
    /// The `ispe` size (the size before transforms), when present.
    pub size: Option<(u32, u32)>,
    pub colour: Vec<Colour>,
    pub transforms: Vec<Transform>,
    /// The alpha plane (a monochrome auxiliary image).
    pub alpha: Option<Box<Image>>,
}

impl Image {
    /// The nclx colour, if any.
    pub fn nclx(&self) -> Option<&Colour> {
        self.colour.iter().find(|c| matches!(c, Colour::Nclx { .. }))
    }
}

/// The parsed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heif {
    /// The `ftyp` major brand.
    pub brand: [u8; 4],
    pub primary: Image,
}

/// A box: its type and payload.
struct BoxRef<'a> {
    kind: [u8; 4],
    body: &'a [u8],
}

/// The boxes in `b`. Stops at the first box that does not fit.
fn boxes(mut b: &[u8]) -> impl Iterator<Item = BoxRef<'_>> {
    std::iter::from_fn(move || {
        let (size, rest) = b.split_first_chunk::<4>()?;
        let (kind, rest) = rest.split_first_chunk::<4>()?;
        let size = u64::from(u32::from_be_bytes(*size));
        let (body_len, rest) = match size {
            0 => (rest.len() as u64, rest),
            1 => {
                let (large, rest) = rest.split_first_chunk::<8>()?;
                (u64::from_be_bytes(*large).checked_sub(16)?, rest)
            }
            n => (n.checked_sub(8)?, rest),
        };
        let body_len = usize::try_from(body_len).ok()?;
        let body = rest.get(..body_len)?;
        b = rest.get(body_len..).unwrap_or(&[]);
        Some(BoxRef { kind: *kind, body })
    })
}

/// A big-endian reader over a box payload.
struct Reader<'a> {
    b: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b }
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let (head, rest) = (self.b.get(..n), self.b.get(n..));
        match (head, rest) {
            (Some(h), Some(r)) => {
                self.b = r;
                Ok(h)
            }
            _ => Err(invalid("box ends early")),
        }
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?.first().copied().unwrap_or(0))
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(self.uint(4)? as u32)
    }
    /// An unsigned big-endian integer of 0–8 bytes.
    fn uint(&mut self, n: usize) -> Result<u64> {
        if n > 8 {
            return Err(invalid("integer wider than 64 bits"));
        }
        Ok(self.bytes(n)?.iter().fold(0u64, |v, b| v << 8 | u64::from(*b)))
    }
    fn fourcc(&mut self) -> Result<[u8; 4]> {
        let b = self.bytes(4)?;
        b.first_chunk::<4>().copied().ok_or_else(|| invalid("box ends early"))
    }
    /// A full box's version and flags.
    fn full(&mut self) -> Result<(u8, u32)> {
        let v = self.u8()?;
        let f = self.uint(3)? as u32;
        Ok((v, f))
    }
    /// Skip a NUL-terminated string.
    fn cstring(&mut self) -> Result<&'a [u8]> {
        let end = self.b.iter().position(|&c| c == 0).ok_or_else(|| invalid("unterminated string"))?;
        let s = self.bytes(end)?;
        self.bytes(1)?;
        Ok(s)
    }
    fn rest(&self) -> &'a [u8] {
        self.b
    }
}

#[derive(Clone, Copy)]
struct Extent {
    offset: u64,
    length: u64,
}

struct Location {
    /// 0: file offsets, 1: `idat` offsets.
    method: u8,
    base: u64,
    extents: Vec<Extent>,
}

#[derive(Clone)]
enum Property<'a> {
    Hvcc(&'a [u8]),
    Ispe(u32, u32),
    Colour(Colour),
    Rotate(u8),
    Mirror(bool),
    Crop(CleanAperture),
    Aux(&'a [u8]),
    Other,
}

struct Meta<'a> {
    file: &'a [u8],
    primary: u32,
    types: HashMap<u32, [u8; 4]>,
    locations: HashMap<u32, Location>,
    idat: &'a [u8],
    properties: Vec<Property<'a>>,
    /// Item → indices into `properties`, in file order.
    associations: HashMap<u32, Vec<usize>>,
    /// (reference type, from) → to items, in file order.
    references: HashMap<([u8; 4], u32), Vec<u32>>,
}

const ALPHA_URNS: [&[u8]; 2] = [b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha", b"urn:mpeg:hevc:2015:auxid:1"];

/// Whether `bytes` starts like a HEIF image file (an `ftyp` box with an image brand).
pub fn sniff(bytes: &[u8]) -> bool {
    brands(bytes).is_some_and(|(major, compatible)| std::iter::once(major).chain(compatible).any(|b| IMAGE_BRANDS.contains(&&b)))
}

const IMAGE_BRANDS: [&[u8; 4]; 8] = [b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"mif1", b"msf1"];

fn brands(bytes: &[u8]) -> Option<([u8; 4], Vec<[u8; 4]>)> {
    let ftyp = boxes(bytes).next().filter(|b| &b.kind == b"ftyp")?;
    let major = *ftyp.body.first_chunk::<4>()?;
    let compatible = ftyp.body.get(8..).unwrap_or(&[]).as_chunks::<4>().0.to_vec();
    Some((major, compatible))
}

/// Parse a HEIF file and resolve its primary image.
pub fn parse(bytes: &[u8]) -> Result<Heif> {
    let (brand, _) = brands(bytes).ok_or(Error::NotHeif("no ftyp box"))?;
    if !sniff(bytes) {
        return Err(Error::NotHeif("no HEIF image brand"));
    }
    let meta = boxes(bytes).find(|b| &b.kind == b"meta").ok_or(Error::NotHeif("no meta box"))?;
    let meta = read_meta(bytes, meta.body)?;
    let mut budget = Budget::for_source(bytes.len().saturating_add(meta.idat.len()));
    let primary = meta.image(meta.primary, 0, &mut budget)?;
    Ok(Heif { brand, primary })
}

fn read_meta<'a>(file: &'a [u8], body: &'a [u8]) -> Result<Meta<'a>> {
    let mut r = Reader::new(body);
    r.full()?;
    let mut meta = Meta {
        file,
        primary: 0,
        types: HashMap::new(),
        locations: HashMap::new(),
        idat: &[],
        properties: Vec::new(),
        associations: HashMap::new(),
        references: HashMap::new(),
    };
    let mut have_primary = false;
    for b in boxes(r.rest()) {
        match &b.kind {
            b"hdlr" => {
                let mut r = Reader::new(b.body);
                r.full()?;
                r.u32()?;
                if &r.fourcc()? != b"pict" {
                    return Err(Error::NotHeif("meta handler is not 'pict'"));
                }
            }
            b"pitm" => {
                let mut r = Reader::new(b.body);
                let (v, _) = r.full()?;
                meta.primary = if v == 0 { u32::from(r.u16()?) } else { r.u32()? };
                have_primary = true;
            }
            b"iinf" => read_iinf(b.body, &mut meta.types)?,
            b"iloc" => read_iloc(b.body, &mut meta.locations)?,
            b"idat" => meta.idat = b.body,
            b"iref" => read_iref(b.body, &mut meta.references)?,
            b"iprp" => read_iprp(b.body, &mut meta.properties, &mut meta.associations)?,
            _ => {}
        }
    }
    if !have_primary {
        return Err(invalid("no primary item"));
    }
    Ok(meta)
}

fn read_iinf(body: &[u8], types: &mut HashMap<u32, [u8; 4]>) -> Result<()> {
    let mut r = Reader::new(body);
    let (v, _) = r.full()?;
    if v == 0 {
        r.u16()? as u32
    } else {
        r.u32()?
    };
    for b in boxes(r.rest()).filter(|b| &b.kind == b"infe") {
        let mut r = Reader::new(b.body);
        let (v, _) = r.full()?;
        // versions 0 and 1 have no item type (they describe files, not images)
        if v < 2 {
            continue;
        }
        let id = if v == 2 { u32::from(r.u16()?) } else { r.u32()? };
        r.u16()?;
        let kind = r.fourcc()?;
        if types.len() >= MAX_ITEMS {
            return Err(invalid("too many items"));
        }
        types.insert(id, kind);
    }
    Ok(())
}

fn read_iloc(body: &[u8], locations: &mut HashMap<u32, Location>) -> Result<()> {
    let mut r = Reader::new(body);
    let (v, _) = r.full()?;
    if v > 2 {
        return Err(invalid(format!("iloc version {v}")));
    }
    let sizes = r.u8()?;
    let (offset_size, length_size) = (usize::from(sizes >> 4), usize::from(sizes & 15));
    let sizes = r.u8()?;
    let base_size = usize::from(sizes >> 4);
    let index_size = if v >= 1 { usize::from(sizes & 15) } else { 0 };
    for s in [offset_size, length_size, base_size, index_size] {
        if !matches!(s, 0 | 4 | 8) {
            return Err(invalid(format!("iloc field size {s}")));
        }
    }
    let count = if v < 2 { u32::from(r.u16()?) } else { r.u32()? };
    for _ in 0..count {
        let id = if v < 2 { u32::from(r.u16()?) } else { r.u32()? };
        let method = if v >= 1 { (r.u16()? & 15) as u8 } else { 0 };
        let data_reference = r.u16()?;
        let base = r.uint(base_size)?;
        let extents = r.u16()?;
        let mut list = Vec::new();
        for _ in 0..extents {
            r.uint(index_size)?;
            let offset = r.uint(offset_size)?;
            let length = r.uint(length_size)?;
            list.push(Extent { offset, length });
        }
        // data in other files (data_reference_index ≠ 0) is not read
        if data_reference == 0 {
            if locations.len() >= MAX_ITEMS {
                return Err(invalid("too many item locations"));
            }
            locations.insert(id, Location { method, base, extents: list });
        }
    }
    Ok(())
}

fn read_iref(body: &[u8], references: &mut HashMap<([u8; 4], u32), Vec<u32>>) -> Result<()> {
    let mut r = Reader::new(body);
    let (v, _) = r.full()?;
    for b in boxes(r.rest()) {
        let mut r = Reader::new(b.body);
        let id = |r: &mut Reader| -> Result<u32> { if v == 0 { Ok(u32::from(r.u16()?)) } else { r.u32() } };
        let from = id(&mut r)?;
        let n = r.u16()?;
        let mut to = Vec::new();
        for _ in 0..n {
            to.push(id(&mut r)?);
        }
        if references.len() >= MAX_ITEMS {
            return Err(invalid("too many item references"));
        }
        references.entry((b.kind, from)).or_default().extend(to);
    }
    Ok(())
}

fn read_iprp<'a>(body: &'a [u8], properties: &mut Vec<Property<'a>>, associations: &mut HashMap<u32, Vec<usize>>) -> Result<()> {
    for b in boxes(body) {
        match &b.kind {
            b"ipco" => {
                for p in boxes(b.body) {
                    if properties.len() >= MAX_ITEMS {
                        return Err(invalid("too many properties"));
                    }
                    properties.push(read_property(&p).unwrap_or(Property::Other));
                }
            }
            b"ipma" => {
                let mut r = Reader::new(b.body);
                let (v, flags) = r.full()?;
                let entries = r.u32()?;
                for _ in 0..entries {
                    let id = if v < 1 { u32::from(r.u16()?) } else { r.u32()? };
                    let n = r.u8()?;
                    let list = associations.entry(id).or_default();
                    for _ in 0..n {
                        let index = if flags & 1 == 1 { r.u16()? & 0x7FFF } else { u16::from(r.u8()? & 0x7F) };
                        // index 0 means "no property"; indices count from 1
                        if let Some(i) = usize::from(index).checked_sub(1) {
                            list.push(i);
                        }
                    }
                    if associations.len() > MAX_ITEMS {
                        return Err(invalid("too many property associations"));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn read_property<'a>(b: &BoxRef<'a>) -> Result<Property<'a>> {
    let mut r = Reader::new(b.body);
    Ok(match &b.kind {
        b"hvcC" => Property::Hvcc(b.body),
        b"ispe" => {
            r.full()?;
            Property::Ispe(r.u32()?, r.u32()?)
        }
        b"colr" => match &r.fourcc()? {
            b"nclx" => {
                let (primaries, transfer, matrix) = (r.u16()?, r.u16()?, r.u16()?);
                Property::Colour(Colour::Nclx { primaries, transfer, matrix, full_range: r.u8()? & 0x80 != 0 })
            }
            b"prof" | b"rICC" => Property::Colour(Colour::Icc(r.rest().to_vec())),
            _ => Property::Other,
        },
        b"irot" => Property::Rotate(r.u8()? & 3),
        b"imir" => Property::Mirror(r.u8()? & 1 == 1),
        b"clap" => {
            let mut f = || -> Result<(u32, u32)> { Ok((r.u32()?, r.u32()?)) };
            let (width, height, h, v) = (f()?, f()?, f()?, f()?);
            Property::Crop(CleanAperture { width, height, horizontal_offset: (h.0 as i32, h.1), vertical_offset: (v.0 as i32, v.1) })
        }
        b"auxC" => {
            r.full()?;
            Property::Aux(r.cstring()?)
        }
        _ => Property::Other,
    })
}

/// What resolving one primary image may still build and copy.
struct Budget {
    images: usize,
    bytes: usize,
}

impl Budget {
    /// Every byte of the source twice over (a master and its alpha may share data), plus
    /// [`ITEM_DATA_SLACK`].
    fn for_source(len: usize) -> Budget {
        Budget { images: MAX_IMAGES, bytes: len.saturating_mul(2).saturating_add(ITEM_DATA_SLACK) }
    }

    fn take_image(&mut self) -> Result<()> {
        self.images = self.images.checked_sub(1).ok_or_else(|| invalid("too many derived images"))?;
        Ok(())
    }

    fn take_bytes(&mut self, n: usize) -> Result<()> {
        self.bytes = self.bytes.checked_sub(n).ok_or_else(|| invalid("item data larger than the file allows"))?;
        Ok(())
    }
}

impl Meta<'_> {
    fn properties(&self, item: u32) -> impl Iterator<Item = &Property<'_>> {
        self.associations.get(&item).into_iter().flatten().filter_map(|&i| self.properties.get(i))
    }

    fn data(&self, item: u32, budget: &mut Budget) -> Result<Vec<u8>> {
        let loc = self.locations.get(&item).ok_or_else(|| invalid(format!("item {item} has no location")))?;
        let source = match loc.method {
            0 => self.file,
            1 => self.idat,
            m => return Err(invalid(format!("item {item}: construction method {m}"))),
        };
        let mut out = Vec::new();
        for e in &loc.extents {
            let start =
                usize::try_from(loc.base.checked_add(e.offset).ok_or_else(|| invalid("extent offset overflows"))?).map_err(|_| invalid("extent offset"))?;
            let rest = source.get(start..).ok_or_else(|| invalid(format!("item {item}: extent past the end of the file")))?;
            // length 0: to the end of the source
            let len = if e.length == 0 { rest.len() } else { usize::try_from(e.length).map_err(|_| invalid("extent length"))? };
            let bytes = rest.get(..len).ok_or_else(|| invalid(format!("item {item}: extent past the end of the file")))?;
            budget.take_bytes(bytes.len())?;
            out.extend_from_slice(bytes);
        }
        Ok(out)
    }

    fn image(&self, item: u32, depth: usize, budget: &mut Budget) -> Result<Image> {
        if depth > MAX_DEPTH {
            return Err(invalid("derived images nest too deeply"));
        }
        budget.take_image()?;
        let kind = *self.types.get(&item).ok_or_else(|| invalid(format!("item {item} has no item info")))?;
        let mut size = None;
        let mut colour = Vec::new();
        let mut transforms = Vec::new();
        let mut hvcc = None;
        for p in self.properties(item) {
            match p {
                Property::Hvcc(h) => hvcc = Some(*h),
                Property::Ispe(w, h) => size = Some((*w, *h)),
                Property::Colour(c) => colour.push(c.clone()),
                Property::Rotate(q) => transforms.push(Transform::Rotate(*q)),
                Property::Mirror(v) => transforms.push(Transform::Mirror { left_right: *v }),
                Property::Crop(c) => transforms.push(Transform::Crop(*c)),
                Property::Aux(_) | Property::Other => {}
            }
        }
        let dimg = self.references.get(&(*b"dimg", item)).map(Vec::as_slice).unwrap_or(&[]);
        let mut image = match &kind {
            b"hvc1" => {
                let hvcc = hvcc.ok_or_else(|| invalid(format!("item {item}: HEVC image without hvcC")))?.to_vec();
                Image { item, coding: Coding::Hevc { hvcc, data: self.data(item, budget)? }, size, colour, transforms, alpha: None }
            }
            b"grid" => {
                let data = self.data(item, budget)?;
                let mut r = Reader::new(&data);
                r.u8()?;
                let flags = r.u8()?;
                let rows = u32::from(r.u8()?) + 1;
                let columns = u32::from(r.u8()?) + 1;
                let field = if flags & 1 == 1 { 4 } else { 2 };
                let (width, height) = (r.uint(field)? as u32, r.uint(field)? as u32);
                let n = (rows * columns) as usize;
                if n > MAX_TILES || dimg.len() != n {
                    return Err(invalid(format!("grid of {rows}×{columns} has {} tiles", dimg.len())));
                }
                let tiles = dimg.iter().map(|&t| self.image(t, depth + 1, budget)).collect::<Result<Vec<_>>>()?;
                Image { item, coding: Coding::Grid { rows, columns, width, height, tiles }, size, colour, transforms, alpha: None }
            }
            b"iden" => {
                let [source] = dimg else { return Err(invalid(format!("identity item {item} needs one source"))) };
                let mut image = self.image(*source, depth + 1, budget)?;
                image.item = item;
                image.transforms.extend(transforms);
                if !colour.is_empty() {
                    image.colour = colour;
                }
                image
            }
            other => Image { item, coding: Coding::Other(*other), size, colour, transforms, alpha: None },
        };
        // an auxiliary alpha image refers to its master with `auxl`
        if depth == 0 {
            let alpha = self
                .references
                .iter()
                .filter(|((kind, _), to)| kind == b"auxl" && to.contains(&item))
                .map(|((_, from), _)| *from)
                .filter(|&a| self.properties(a).any(|p| matches!(p, Property::Aux(urn) if ALPHA_URNS.contains(urn))))
                .min();
            if let Some(a) = alpha {
                image.alpha = Some(Box::new(self.image(a, depth + 1, budget)?));
            }
        }
        Ok(image)
    }
}

#[cfg(feature = "writer")]
pub mod writer;

#[cfg(test)]
mod tests;
