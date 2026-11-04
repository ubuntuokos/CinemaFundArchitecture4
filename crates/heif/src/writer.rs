//! A minimal HEIF writer for tests: items with chosen data, properties and references, laid out
//! as `ftyp`, `meta` and `mdat`. It writes what it is given (including invalid structures).

/// One item to write.
#[derive(Clone, Debug, Default)]
pub struct Item {
    pub id: u32,
    pub kind: [u8; 4],
    pub data: Vec<u8>,
    /// Whole property boxes (see the `*_box` functions), associated in this order.
    pub properties: Vec<Vec<u8>>,
    /// References from this item: (type, to items).
    pub references: Vec<([u8; 4], Vec<u32>)>,
    pub hidden: bool,
}

fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = vec![version];
    b.extend_from_slice(&flags.to_be_bytes()[1..]);
    b.extend_from_slice(body);
    boxed(kind, &b)
}

/// An `hvcC` property with the given record.
pub fn hvcc_box(record: &[u8]) -> Vec<u8> {
    boxed(b"hvcC", record)
}

pub fn ispe_box(width: u32, height: u32) -> Vec<u8> {
    let mut b = width.to_be_bytes().to_vec();
    b.extend_from_slice(&height.to_be_bytes());
    full(b"ispe", 0, 0, &b)
}

/// `irot` with anticlockwise quarter turns.
pub fn irot_box(quarter_turns: u8) -> Vec<u8> {
    boxed(b"irot", &[quarter_turns & 3])
}

/// `imir` with its mode bit (0: top and bottom swap; 1: left and right swap).
pub fn imir_box(axis: u8) -> Vec<u8> {
    boxed(b"imir", &[axis & 1])
}

/// `clap`: width, height and the offsets as (numerator, denominator).
pub fn clap_box(width: (u32, u32), height: (u32, u32), horizontal: (i32, u32), vertical: (i32, u32)) -> Vec<u8> {
    let mut b = Vec::new();
    for (n, d) in [width, height, (horizontal.0 as u32, horizontal.1), (vertical.0 as u32, vertical.1)] {
        b.extend_from_slice(&n.to_be_bytes());
        b.extend_from_slice(&d.to_be_bytes());
    }
    boxed(b"clap", &b)
}

pub fn auxc_box(urn: &str) -> Vec<u8> {
    let mut b = urn.as_bytes().to_vec();
    b.push(0);
    full(b"auxC", 0, 0, &b)
}

pub fn nclx_box(primaries: u16, transfer: u16, matrix: u16, full_range: bool) -> Vec<u8> {
    let mut b = b"nclx".to_vec();
    for v in [primaries, transfer, matrix] {
        b.extend_from_slice(&v.to_be_bytes());
    }
    b.push(if full_range { 0x80 } else { 0 });
    boxed(b"colr", &b)
}

/// The data of a `grid` item.
pub fn grid_data(rows: u32, columns: u32, width: u32, height: u32) -> Vec<u8> {
    let wide = width > 0xFFFF || height > 0xFFFF;
    let mut b = vec![0, u8::from(wide), (rows - 1) as u8, (columns - 1) as u8];
    if wide {
        b.extend_from_slice(&width.to_be_bytes());
        b.extend_from_slice(&height.to_be_bytes());
    } else {
        b.extend_from_slice(&(width as u16).to_be_bytes());
        b.extend_from_slice(&(height as u16).to_be_bytes());
    }
    b
}

fn meta(primary: u32, items: &[Item], data_start: u32) -> Vec<u8> {
    let mut body = Vec::new();
    let mut hdlr = vec![0; 4];
    hdlr.extend_from_slice(b"pict");
    hdlr.extend_from_slice(&[0; 13]);
    body.extend(full(b"hdlr", 0, 0, &hdlr));
    body.extend(full(b"pitm", 0, 0, &(primary as u16).to_be_bytes()));

    let mut iinf = (items.len() as u16).to_be_bytes().to_vec();
    for it in items {
        let mut infe = (it.id as u16).to_be_bytes().to_vec();
        infe.extend_from_slice(&[0, 0]);
        infe.extend_from_slice(&it.kind);
        infe.push(0);
        iinf.extend(full(b"infe", 2, u32::from(it.hidden), &infe));
    }
    body.extend(full(b"iinf", 0, 0, &iinf));

    let mut iref = Vec::new();
    for it in items {
        for (kind, to) in &it.references {
            let mut r = (it.id as u16).to_be_bytes().to_vec();
            r.extend_from_slice(&(to.len() as u16).to_be_bytes());
            for t in to {
                r.extend_from_slice(&(*t as u16).to_be_bytes());
            }
            iref.extend(boxed(kind, &r));
        }
    }
    if !iref.is_empty() {
        body.extend(full(b"iref", 0, 0, &iref));
    }

    let mut ipco = Vec::new();
    let mut ipma = (items.len() as u32).to_be_bytes().to_vec();
    let mut index = 0u8;
    for it in items {
        ipma.extend_from_slice(&(it.id as u16).to_be_bytes());
        ipma.push(it.properties.len() as u8);
        for p in &it.properties {
            ipco.extend_from_slice(p);
            index += 1;
            ipma.push(0x80 | index);
        }
    }
    let mut iprp = boxed(b"ipco", &ipco);
    iprp.extend(full(b"ipma", 0, 0, &ipma));
    body.extend(boxed(b"iprp", &iprp));

    // iloc version 0: offset 4, length 4, base 0
    let mut iloc = vec![0x44, 0x00];
    iloc.extend_from_slice(&(items.len() as u16).to_be_bytes());
    let mut at = data_start;
    for it in items {
        iloc.extend_from_slice(&(it.id as u16).to_be_bytes());
        iloc.extend_from_slice(&[0, 0, 0, 1]);
        iloc.extend_from_slice(&at.to_be_bytes());
        iloc.extend_from_slice(&(it.data.len() as u32).to_be_bytes());
        at += it.data.len() as u32;
    }
    body.extend(full(b"iloc", 0, 0, &iloc));
    full(b"meta", 0, 0, &body)
}

/// A HEIF file (`heic` brand) with `items`, `primary` as its primary item.
pub fn build(primary: u32, items: &[Item]) -> Vec<u8> {
    let mut ftyp = b"heic".to_vec();
    ftyp.extend_from_slice(&[0; 4]);
    ftyp.extend_from_slice(b"mif1heic");
    let ftyp = boxed(b"ftyp", &ftyp);
    let size = meta(primary, items, 0).len();
    let data_start = (ftyp.len() + size + 8) as u32;
    let mut out = ftyp;
    out.extend(meta(primary, items, data_start));
    let data: Vec<u8> = items.iter().flat_map(|i| i.data.iter().copied()).collect();
    out.extend(boxed(b"mdat", &data));
    out
}
