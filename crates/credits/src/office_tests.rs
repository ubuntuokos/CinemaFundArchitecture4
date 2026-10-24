use crate::office::read_office;
use crate::table::Limits;
use crate::zip::Zip;
use crate::{CreditsError, Styling, read, read_rows, read_with};

/// Build a ZIP archive. `deflated` entries carry pre-compressed raw DEFLATE data.
fn zip(entries: &[(&str, &[u8], Option<usize>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data, unpacked) in entries {
        let local = out.len() as u32;
        let (method, size) = match unpacked {
            Some(n) => (8u16, *n as u32),
            None => (0u16, data.len() as u32),
        };
        let mut h = Vec::new();
        h.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        h.extend_from_slice(&20u16.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(&method.to_le_bytes());
        h.extend_from_slice(&[0; 8]); // time, date, crc
        h.extend_from_slice(&(data.len() as u32).to_le_bytes());
        h.extend_from_slice(&size.to_le_bytes());
        h.extend_from_slice(&(name.len() as u16).to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&[0; 8]);
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]); // extra, comment, disk, attrs
        central.extend_from_slice(&local.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_off = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

const WB: &str = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Notes" sheetId="1" r:id="rId1"/><sheet name="Credits" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
const RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Target="/xl/worksheets/sheet2.xml"/></Relationships>"#;
const SST: &str = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>Head</t></si><si><t>Body</t></si><si><r><t>Directed </t></r><r><t>by</t></r><rPh><t>x</t></rPh></si><si><t xml:space="preserve">Ann
Smith</t></si></sst>"#;
const S1: &str = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>just notes</t></is></c></row></sheetData></worksheet>"#;
const S2: &str = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="2"><c r="A2" t="s"><v>0</v></c><c r="B2" t="s"><v>1</v></c><c r="D2" t="inlineStr"><is><t>Vertical Gap</t></is></c></row>
<row r="3"><c r="A3" t="s"><v>2</v></c><c r="B3" t="s"><v>3</v></c></row>
<row r="5"><c r="B5" t="str"><v>Bob</v></c><c r="D5"><v>2</v></c></row>
</sheetData></worksheet>"#;

fn xlsx() -> Vec<u8> {
    zip(&[
        ("[Content_Types].xml", b"<Types/>", None),
        ("xl/workbook.xml", WB.as_bytes(), None),
        ("xl/_rels/workbook.xml.rels", RELS.as_bytes(), None),
        ("xl/sharedStrings.xml", SST.as_bytes(), None),
        ("xl/worksheets/sheet1.xml", S1.as_bytes(), None),
        ("xl/worksheets/sheet2.xml", S2.as_bytes(), None),
    ])
}

#[test]
fn xlsx_sheets_shared_strings_and_references() {
    let x = xlsx();
    let s = read_rows(&x, Some("credits")).unwrap();
    assert_eq!(s.sheets, vec!["Notes", "Credits"]);
    assert_eq!(s.chosen, "Credits");
    assert_eq!(s.rows[1], vec!["Head", "Body", "", "Vertical Gap"]);
    assert_eq!(s.rows[2], vec!["Directed by", "Ann\nSmith"]);
    assert_eq!(s.rows[4], vec!["", "Bob", "", "2"]);
    assert_eq!(read_rows(&x, Some("2")).unwrap().chosen, "Credits");
    assert_eq!(read_rows(&x, None).unwrap().chosen, "Notes");
    assert!(read_rows(&x, Some("nope")).is_err());

    let (c, _) = read_with(&x, &Styling::default(), Some("Credits")).unwrap();
    let b = &c.pages[0].blocks;
    assert_eq!(b[0].head.as_deref(), Some("Directed by"));
    assert_eq!(b[1].body, vec!["Bob"]);
    assert_eq!(b[1].gap_before, crate::Gap::Units(2.0));
}

const ODS: &str = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet>
<table:table table:name="Stáblista">
<table:table-row><table:table-cell><text:p>Fej</text:p></table:table-cell><table:table-cell><text:p>Törzs</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell><text:p>Zene<text:s text:c="2"/>és<text:s/>hang</text:p></table:table-cell><table:table-cell><text:p>Kiss</text:p><text:p>Anna<text:line-break/>Bea</text:p></table:table-cell></table:table-row>
<table:table-row table:number-rows-repeated="3"><table:table-cell table:number-columns-repeated="1024"/></table:table-row>
<table:table-row><table:table-cell table:number-columns-repeated="1"/><table:table-cell office:value-type="float" office:value="7"><text:p>7</text:p></table:table-cell></table:table-row>
<table:table-row table:number-rows-repeated="1048570"><table:table-cell table:number-columns-repeated="1024"/></table:table-row>
</table:table></office:spreadsheet></office:body></office:document-content>"#;

#[test]
fn ods_repeats_spaces_paragraphs_and_breaks() {
    let o = zip(&[("mimetype", b"application/vnd.oasis.opendocument.spreadsheet", None), ("content.xml", ODS.as_bytes(), None)]);
    let s = read_rows(&o, None).unwrap();
    assert_eq!(s.sheets, vec!["Stáblista"]);
    assert_eq!(s.rows.len(), 6, "trailing empty rows cost nothing");
    assert_eq!(s.rows[1], vec!["Zene  és hang", "Kiss\nAnna\nBea"]);
    assert_eq!(s.rows[5], vec!["", "7"]);
    let c = read(&o, &Styling::default()).unwrap();
    assert_eq!(c.pages[0].blocks[0].body, vec!["Kiss\nAnna\nBea"]);
    assert_eq!(c.pages[0].blocks[1].gap_before, crate::Gap::Units(3.0));
}

#[test]
fn deflated_entries_unpack() {
    // raw DEFLATE of "<x>hello deflate hello deflate</x>" (zlib, level 9, no header)
    let d: [u8; 25] = [179, 169, 176, 203, 72, 205, 201, 201, 87, 72, 73, 77, 203, 73, 44, 73, 85, 64, 225, 217, 232, 87, 216, 1, 0];
    let z = zip(&[("a.xml", &d, Some(34))]);
    let a = Zip::open(&z).unwrap();
    assert_eq!(a.read_named("a.xml").unwrap(), b"<x>hello deflate hello deflate</x>");
    // a wrong declared size is an error
    let z = zip(&[("a.xml", &d, Some(40))]);
    assert!(Zip::open(&z).unwrap().read_named("a.xml").is_err());
}

#[test]
fn not_a_spreadsheet_and_limits() {
    let other = zip(&[("readme.txt", b"hi", None)]);
    assert!(matches!(read_rows(&other, None), Err(CreditsError::Format(_))));
    let tiny = Limits { max_rows: 2, ..Limits::default() };
    assert!(matches!(read_office(&xlsx(), Some("Credits"), tiny), Err(CreditsError::TooLarge(_))));
}

/// Hostile packages: truncations and byte flips never panic.
#[test]
fn hostile_packages_never_panic() {
    let o = zip(&[("mimetype", b"application/vnd.oasis.opendocument.spreadsheet", None), ("content.xml", ODS.as_bytes(), None)]);
    let d: [u8; 25] = [179, 169, 176, 203, 72, 205, 201, 201, 87, 72, 73, 77, 203, 73, 44, 73, 85, 64, 225, 217, 232, 87, 216, 1, 0];
    let deflated = zip(&[("xl/workbook.xml", &d, Some(34))]);
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for base in [xlsx(), o, deflated] {
        for i in 0..1500 {
            let mut b = base.clone();
            if i % 3 == 0 {
                b.truncate((rnd() as usize) % (b.len() + 1));
            } else {
                for _ in 0..1 + rnd() % 6 {
                    let n = b.len();
                    let k = (rnd() as usize) % n;
                    b[k] = rnd() as u8;
                }
            }
            if let Ok(s) = read_rows(&b, None) {
                let _ = crate::read_sheet(&s.rows, "Scroll", "Gutter");
            }
        }
    }
}
