use crate::diff::{ChangeKind, diff, merge};
use crate::doc::{CreditsDoc, is_doc};
use crate::{Styling, layout, read_rows};

fn t(rows: &[&str]) -> Vec<Vec<String>> {
    rows.iter().map(|r| r.split(',').map(str::to_string).collect()).collect()
}

#[test]
fn diff_finds_added_removed_and_changed_rows() {
    let old = t(&["Head,Body", "Directed by,Ann", ",", "Cast,Bob", ",Cyd", ",Dee"]);
    let new = t(&["Head,Body", "Directed by,Anna", ",", "Cast,Bob", ",Dee", ",Eve", ""]);
    let c = diff(&old, &new);
    let kinds: Vec<ChangeKind> = c.iter().map(|c| c.kind).collect();
    assert_eq!(kinds, vec![ChangeKind::Changed, ChangeKind::Removed, ChangeKind::Added], "{c:#?}");
    assert_eq!(c[0].old_rows, (1, 2));
    assert_eq!(c[0].after, t(&["Directed by,Anna"]));
    assert_eq!(c[1].before, t(&[",Cyd"]));
    // the trailing empty row is not a change; trailing empty cells neither
    assert_eq!(c[2].after, t(&[",Eve"]));
    // accept everything: the new table (without the meaningless empty row)
    let all: Vec<usize> = c.iter().map(|c| c.id).collect();
    assert_eq!(merge(&old, &new, &c, &all)[..6], new[..6]);
    // accept nothing: the old table
    assert_eq!(merge(&old, &new, &c, &[]), old);
    // only the renamed director
    let m = merge(&old, &new, &c, &[0]);
    assert_eq!(m, t(&["Head,Body", "Directed by,Anna", ",", "Cast,Bob", ",Cyd", ",Dee"]));
    // only the removal of Cyd
    assert_eq!(merge(&old, &new, &c, &[1]), t(&["Head,Body", "Directed by,Ann", ",", "Cast,Bob", ",Dee"]));
}

#[test]
fn identical_and_empty_tables() {
    let a = t(&["Body", "A"]);
    assert!(diff(&a, &a).is_empty());
    let c = diff(&[], &a);
    assert_eq!(c.len(), 1);
    assert_eq!(merge(&[], &a, &c, &[0]), a);
    assert_eq!(diff(&a, &[])[0].kind, ChangeKind::Removed);
}

#[test]
fn huge_rewrites_are_one_change_not_a_quadratic_search() {
    let old: Vec<Vec<String>> = (0..3000).map(|i| vec![format!("a{i}")]).collect();
    let new: Vec<Vec<String>> = (0..3000).map(|i| vec![format!("b{i}")]).collect();
    let c = diff(&old, &new);
    assert_eq!(c.len(), 1);
    assert_eq!(merge(&old, &new, &c, &[0]), new);
}

#[test]
fn doc_round_trip_and_refusals() {
    let d = CreditsDoc { table: t(&["Body", "Ann"]), ..Default::default() };
    let json = d.to_json();
    assert!(is_doc(json.as_bytes()));
    assert_eq!(CreditsDoc::from_bytes(json.as_bytes()).unwrap(), d);
    assert_eq!(d.credits().unwrap().pages[0].blocks[0].body, vec!["Ann"]);
    assert!(CreditsDoc::from_bytes(br#"{"format":"other"}"#).is_err());
    assert!(CreditsDoc::from_bytes(br#"{"format":"cfa4.credits","version":99}"#).is_err());
    assert!(CreditsDoc::from_bytes(b"{not json").is_err());
    assert!(!is_doc(b"Head,Body\n"));
    // read_rows does not mistake a document for a CSV with one strange column
    assert!(read_rows(b"Body\nA\n", None).is_ok());
}

#[test]
fn layer_keys_survive_unrelated_changes() {
    struct M;
    impl layout::Measure for M {
        fn bounds(&self, _: &cfa4_project::EffectInstance) -> Option<[f64; 4]> {
            Some([-50.0, -30.0, 50.0, 10.0])
        }
    }
    let frame = layout::Frame { width: 1920, height: 1080, rate: cfa4_time::FrameRate::FPS_25 };
    let keys = |rows: &[&str]| {
        let c = crate::read_sheet(&t(rows), "Scroll", "Gutter").unwrap();
        let l = layout::layout(&c, &Styling::default(), frame, &M).unwrap();
        l.pages[0].keys.iter().map(|(_, k)| k.clone()).collect::<Vec<_>>()
    };
    let a = keys(&["Head,Body", "Music,Hana", ",", "Cast,Bob"]);
    let b = keys(&["Head,Body", "Producer,Zed", ",", "Music,Hana", ",", "Cast,Bob"]);
    // the Music and Cast blocks keep their keys although rows were added before them
    for k in &a {
        assert!(b.contains(k), "{k} lost: {b:?}");
    }
    // a changed block gets new keys
    let c = keys(&["Head,Body", "Music,Hana Kobayashi", ",", "Cast,Bob"]);
    assert!(!c.contains(&a[0]) && c.contains(a.last().unwrap()));
}

#[test]
fn hostile_tables_never_panic_in_diff_or_merge() {
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..300 {
        let mk = |rnd: &mut dyn FnMut() -> u64| -> Vec<Vec<String>> {
            (0..rnd() % 30).map(|_| (0..rnd() % 4).map(|_| ["", "a", "b", " "][(rnd() % 4) as usize].to_string()).collect()).collect()
        };
        let old = mk(&mut rnd);
        let new = mk(&mut rnd);
        let c = diff(&old, &new);
        let all: Vec<usize> = c.iter().map(|c| c.id).collect();
        let merged = merge(&old, &new, &c, &all);
        // accepting everything gives the new table up to empty cells and rows
        assert!(diff(&merged, &new).is_empty(), "{old:?} -> {new:?} gave {merged:?}");
        assert!(diff(&merge(&old, &new, &c, &[]), &old).is_empty());
    }
}
