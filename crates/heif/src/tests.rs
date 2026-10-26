use super::*;
use crate::writer::*;

fn hevc(id: u32, data: &[u8]) -> Item {
    Item { id, kind: *b"hvc1", data: data.to_vec(), properties: vec![hvcc_box(&[1, 2, 3]), ispe_box(64, 48)], ..Item::default() }
}

#[test]
fn a_single_hevc_image_with_transforms_and_colour() {
    let mut item = hevc(1, b"nal units");
    item.properties.extend([nclx_box(1, 13, 6, true), clap_box((60, 1), (40, 1), (0, 1), (0, 1)), irot_box(1), imir_box(1)]);
    let file = build(1, &[item]);
    assert!(sniff(&file));
    let heif = parse(&file).unwrap();
    assert_eq!(&heif.brand, b"heic");
    let p = heif.primary;
    assert_eq!(p.coding, Coding::Hevc { hvcc: vec![1, 2, 3], data: b"nal units".to_vec() });
    assert_eq!(p.size, Some((64, 48)));
    assert_eq!(p.nclx(), Some(&Colour::Nclx { primaries: 1, transfer: 13, matrix: 6, full_range: true }));
    assert!(matches!(p.transforms.as_slice(), [Transform::Crop(_), Transform::Rotate(1), Transform::Mirror { left_right: true }]), "{:?}", p.transforms);
    assert!(p.alpha.is_none());
}

#[test]
fn a_grid_lists_its_tiles_in_raster_order() {
    let grid = Item { id: 10, kind: *b"grid", data: grid_data(2, 3, 150, 90), references: vec![(*b"dimg", vec![1, 2, 3, 4, 5, 6])], ..Item::default() };
    let mut items = vec![grid];
    items.extend((1..=6).map(|i| Item { hidden: true, ..hevc(i, &[i as u8; 3]) }));
    let p = parse(&build(10, &items)).unwrap().primary;
    let Coding::Grid { rows, columns, width, height, tiles } = p.coding else { panic!("grid") };
    assert_eq!((rows, columns, width, height), (2, 3, 150, 90));
    let data: Vec<_> = tiles
        .iter()
        .map(|t| match &t.coding {
            Coding::Hevc { data, .. } => data[0],
            _ => 0,
        })
        .collect();
    assert_eq!(data, [1, 2, 3, 4, 5, 6]);
}

#[test]
fn identity_items_add_their_transforms_and_alpha_is_found() {
    let iden = Item { id: 5, kind: *b"iden", properties: vec![irot_box(2)], references: vec![(*b"dimg", vec![1])], ..Item::default() };
    let mut source = hevc(1, b"colour");
    source.properties.push(imir_box(0));
    let mut alpha = hevc(2, b"alpha");
    alpha.properties.push(auxc_box("urn:mpeg:mpegB:cicp:systems:auxiliary:alpha"));
    alpha.references.push((*b"auxl", vec![5]));
    let mut depth = hevc(3, b"depth");
    depth.properties.push(auxc_box("urn:mpeg:hevc:2015:auxid:2"));
    depth.references.push((*b"auxl", vec![5]));
    let p = parse(&build(5, &[iden, source, alpha, depth])).unwrap().primary;
    assert_eq!(p.item, 5);
    assert_eq!(p.transforms, [Transform::Mirror { left_right: false }, Transform::Rotate(2)]);
    let a = p.alpha.expect("alpha");
    assert_eq!(a.coding, Coding::Hevc { hvcc: vec![1, 2, 3], data: b"alpha".to_vec() });
}

#[test]
fn other_item_types_are_reported() {
    let av1 = Item { id: 1, kind: *b"av01", data: vec![0; 4], ..Item::default() };
    assert_eq!(parse(&build(1, &[av1])).unwrap().primary.coding, Coding::Other(*b"av01"));
}

#[test]
fn clean_aperture_rectangles() {
    let c = |w: (u32, u32), h: (u32, u32), x: (i32, u32), y: (i32, u32)| CleanAperture { width: w, height: h, horizontal_offset: x, vertical_offset: y };
    // centred
    assert_eq!(c((60, 1), (40, 1), (0, 1), (0, 1)).rect(64, 48), Some((2, 4, 60, 40)));
    // odd difference: the edge falls between pixels and rounds half up
    assert_eq!(c((61, 1), (47, 1), (0, 1), (0, 1)).rect(64, 48), Some((2, 1, 61, 47)));
    // offsets move the centre
    assert_eq!(c((10, 1), (10, 1), (-20, 1), (5, 2)).rect(64, 48), Some((7, 22, 10, 10)));
    // clamped to the image, never empty, never a zero denominator
    assert_eq!(c((100, 1), (100, 1), (0, 1), (0, 1)).rect(64, 48), Some((0, 0, 64, 48)));
    assert_eq!(c((10, 0), (10, 1), (0, 1), (0, 1)).rect(64, 48), None);
    assert_eq!(c((10, 1), (10, 1), (1000, 1), (0, 1)).rect(64, 48), None);
    assert!(c((u32::MAX, 1), (u32::MAX, 1), (i32::MIN, 1), (i32::MAX, 1)).rect(u32::MAX, u32::MAX).is_some());
}

#[test]
fn broken_structures_are_errors() {
    // a grid with the wrong number of tiles
    let grid = Item { id: 10, kind: *b"grid", data: grid_data(2, 2, 10, 10), references: vec![(*b"dimg", vec![1])], ..Item::default() };
    assert!(parse(&build(10, &[grid, hevc(1, b"x")])).is_err());
    // an identity item that refers to itself
    let iden = Item { id: 1, kind: *b"iden", references: vec![(*b"dimg", vec![1])], ..Item::default() };
    assert!(matches!(parse(&build(1, &[iden])), Err(Error::Invalid(_))));
    // no primary item info
    assert!(parse(&build(9, &[hevc(1, b"x")])).is_err());
    // not HEIF
    assert!(matches!(parse(b"\0\0\0\x10ftypisom\0\0\0\0"), Err(Error::NotHeif(_))));
    assert!(!sniff(b"\0\0\0\x10ftypisom\0\0\0\0"));
}

#[test]
fn damaged_files_never_panic() {
    let grid = Item { id: 10, kind: *b"grid", data: grid_data(1, 2, 100, 50), references: vec![(*b"dimg", vec![1, 2])], ..Item::default() };
    let mut alpha = hevc(3, b"alpha");
    alpha.properties.push(auxc_box("urn:mpeg:hevc:2015:auxid:1"));
    alpha.references.push((*b"auxl", vec![10]));
    let whole = build(10, &[grid, hevc(1, b"left"), hevc(2, b"right"), alpha]);
    assert!(parse(&whole).unwrap().primary.alpha.is_some());
    for cut in 0..whole.len() {
        let _ = parse(&whole[..cut]);
    }
    let mut s = 0x2545_F491u32;
    for round in 0..5000 {
        let mut b = whole.clone();
        for _ in 0..1 + round % 4 {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let i = (s >> 8) as usize % b.len();
            b[i] = if round % 3 == 0 { 0xFF } else { (s >> 24) as u8 };
        }
        let _ = parse(&b);
    }
}

/// Eight nested grids of 16 references each to the next: nine items, within the depth and tile
/// limits, that expanded into 16^8 leaf images.
#[test]
fn nested_grids_sharing_items_cannot_multiply() {
    let start = std::time::Instant::now();
    let mut items: Vec<Item> = (0..8u32)
        .map(|k| Item {
            id: 10 + k,
            kind: *b"grid",
            data: grid_data(4, 4, 64, 64),
            references: vec![(*b"dimg", vec![11 + k; 16])],
            hidden: k > 0,
            ..Item::default()
        })
        .collect();
    items.push(Item { hidden: true, ..hevc(18, b"leaf") });
    assert!(matches!(parse(&build(10, &items)), Err(Error::Invalid(_))));
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    // one level of the same (16 references to one tile) is a legal grid
    let one = vec![
        Item { id: 10, kind: *b"grid", data: grid_data(4, 4, 64, 64), references: vec![(*b"dimg", vec![1; 16])], ..Item::default() },
        Item { hidden: true, ..hevc(1, b"tile") },
    ];
    assert!(parse(&build(10, &one)).is_ok());
}

/// 65 535 extents over the same megabyte asked for 64 GiB of item data.
#[test]
fn repeated_extents_are_capped_by_the_budget() {
    let file = vec![0u8; 1 << 20];
    let mut meta = Meta {
        file: &file,
        primary: 1,
        types: HashMap::new(),
        locations: HashMap::new(),
        idat: &[],
        properties: Vec::new(),
        associations: HashMap::new(),
        references: HashMap::new(),
    };
    let extents = (0..65_535).map(|_| Extent { offset: 0, length: 0 }).collect();
    meta.locations.insert(1, Location { method: 0, base: 0, extents });
    let mut budget = Budget::for_source(file.len());
    assert!(matches!(meta.data(1, &mut budget), Err(Error::Invalid(_))));
    // a few copies fit
    meta.locations.insert(2, Location { method: 0, base: 0, extents: vec![Extent { offset: 0, length: 16 }; 3] });
    assert_eq!(meta.data(2, &mut Budget::for_source(file.len())).map(|d| d.len()), Ok(48));
}

/// Extents of zero-width fields take no bytes of the iloc box, but each is an allocation.
#[test]
fn zero_width_extents_are_bounded() {
    let items = MAX_EXTENTS / 65_535 + 1;
    let mut body = vec![0u8, 0, 0, 0, 0x00, 0x00];
    body.extend((items as u16).to_be_bytes());
    for id in 0..items as u16 {
        body.extend(id.to_be_bytes());
        body.extend(0u16.to_be_bytes()); // data_reference_index
        body.extend(u16::MAX.to_be_bytes());
    }
    let mut locations = HashMap::new();
    assert!(matches!(read_iloc(&body, &mut locations), Err(Error::Invalid(_))));
    // one item with that many is fine
    let mut one = vec![0u8, 0, 0, 0, 0x00, 0x00, 0, 1, 0, 1, 0, 0];
    one.extend(u16::MAX.to_be_bytes());
    assert!(read_iloc(&one, &mut HashMap::new()).is_ok());
}

/// A grid repeating one tile copies its hvcC per reference: the copies count against the budget.
#[test]
fn repeated_property_copies_are_charged_to_the_budget() {
    let mut tile = Item { hidden: true, ..hevc(1, b"tile") };
    tile.properties = vec![hvcc_box(&vec![7u8; 1 << 20]), ispe_box(64, 48)];
    let grid = Item { id: 10, kind: *b"grid", data: grid_data(16, 16, 1024, 768), references: vec![(*b"dimg", vec![1; 256])], ..Item::default() };
    assert!(matches!(parse(&build(10, &[grid, tile.clone()])), Err(Error::Invalid(_))));
    // a few references to a large hvcC still parse
    let small = Item { id: 10, kind: *b"grid", data: grid_data(2, 2, 128, 96), references: vec![(*b"dimg", vec![1; 4])], ..Item::default() };
    assert!(parse(&build(10, &[small, tile])).is_ok());
}
