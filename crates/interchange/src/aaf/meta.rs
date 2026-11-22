//! The AAF meta-dictionary (AAF Object Specification v1.1, "meta-dictionary"): the class and type
//! definitions of the built-in object model that readers such as the AAF SDK need in a file.
//! The definitions are data in `meta_model.txt` (format described at the top of that file),
//! turned into the stored object tree here.

use super::ids::META_DICTIONARY;
use super::store::{Auid, Obj, Value, utf16z};

const MODEL: &str = include_str!("meta_model.txt");

/// `ClassDefinition`, `PropertyDefinition` (classes of the meta-model).
const CLASS_DEFINITION: &str = "0101010d01020000060e2b3402060101";
const PROPERTY_DEFINITION: &str = "0101010d02020000060e2b3402060101";

// Property ids of the meta-model.
const MD_CLASS_DEFINITIONS: u16 = 0x0003;
const MD_TYPE_DEFINITIONS: u16 = 0x0004;
const DEF_IDENTIFICATION: u16 = 0x0005;
const DEF_NAME: u16 = 0x0006;
const CLASS_PARENT: u16 = 0x0008;
const CLASS_PROPERTIES: u16 = 0x0009;
const CLASS_CONCRETE: u16 = 0x000A;
const PROP_TYPE: u16 = 0x000B;
const PROP_OPTIONAL: u16 = 0x000C;
const PROP_UNIQUE: u16 = 0x000E;
const PROP_LOCAL_ID: u16 = 0x000D;

fn hex(s: &str) -> Vec<u8> {
    s.as_bytes().chunks(2).filter_map(|c| std::str::from_utf8(c).ok().and_then(|c| u8::from_str_radix(c, 16).ok())).collect()
}

fn auid(s: &str) -> Auid {
    let mut a = [0u8; 16];
    for (d, b) in a.iter_mut().zip(hex(s)) {
        *d = b;
    }
    a
}

fn path(s: &str) -> Vec<u16> {
    s.split(',').filter_map(|p| p.parse().ok()).collect()
}

fn pid(s: &str) -> Option<u16> {
    u16::from_str_radix(s, 16).ok()
}

/// One field of a `type` line.
fn field(f: &str) -> Option<(u16, Value)> {
    let (id, rest) = f.split_once('=')?;
    let id = pid(id)?;
    let (kind, v) = rest.split_once(':')?;
    let value = match kind {
        "d" => Value::Data(hex(v)),
        "w" => {
            let mut p = v.splitn(3, ':');
            let (path_s, key_pid, key) = (p.next()?, p.next()?, p.next()?);
            Value::Weak(path(path_s), pid(key_pid)?, hex(key))
        }
        "v" => {
            let mut p = v.splitn(3, ':');
            let (path_s, key_pid, keys) = (p.next()?, p.next()?, p.next()?);
            Value::WeakVec(path(path_s), pid(key_pid)?, keys.split(',').map(hex).collect())
        }
        _ => return None,
    };
    Some((id, value))
}

/// The meta-dictionary object of the root.
pub(crate) fn dictionary() -> Obj {
    let mut classes: Vec<Obj> = Vec::new();
    let mut props: Vec<Obj> = Vec::new();
    let mut types: Vec<Obj> = Vec::new();
    // Properties belong to the class line before them.
    fn close(classes: &mut [Obj], props: &mut Vec<Obj>) {
        if let Some(c) = classes.last_mut()
            && !props.is_empty()
        {
            c.props.insert(1, (CLASS_PROPERTIES, Value::StrongSet(std::mem::take(props), DEF_IDENTIFICATION)));
        }
    }
    for line in MODEL.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        let f: Vec<&str> = line.split('\t').collect();
        match f.as_slice() {
            ["class", id, name, parent, concrete] => {
                close(&mut classes, &mut props);
                classes.push(
                    Obj::new(auid(CLASS_DEFINITION))
                        .with(CLASS_CONCRETE, Value::Data(hex(concrete)))
                        .with(CLASS_PARENT, Value::Weak(vec![1, 3], DEF_IDENTIFICATION, hex(parent)))
                        .with(DEF_IDENTIFICATION, Value::Data(hex(id)))
                        .with(DEF_NAME, Value::Data(utf16z(name))),
                );
            }
            ["prop", id, local, name, ty, optional, unique] => {
                let local = pid(local).unwrap_or(0);
                let mut def = Obj::new(auid(PROPERTY_DEFINITION));
                if !unique.is_empty() {
                    def.set(PROP_UNIQUE, Value::Data(hex(unique)));
                }
                props.push(
                    def.with(PROP_LOCAL_ID, Value::Data(local.to_le_bytes().to_vec()))
                        .with(PROP_OPTIONAL, Value::Data(hex(optional)))
                        .with(PROP_TYPE, Value::Data(hex(ty)))
                        .with(DEF_IDENTIFICATION, Value::Data(hex(id)))
                        .with(DEF_NAME, Value::Data(utf16z(name))),
                );
            }
            ["type", class, id, name, fields] => {
                let mut o = Obj::new(auid(class));
                for (p, v) in fields.split(' ').filter_map(field) {
                    o.set(p, v);
                }
                o.set(DEF_IDENTIFICATION, Value::Data(hex(id)));
                o.set(DEF_NAME, Value::Data(utf16z(name)));
                types.push(o);
            }
            _ => {}
        }
    }
    close(&mut classes, &mut props);
    Obj::new(META_DICTIONARY)
        .with(MD_CLASS_DEFINITIONS, Value::StrongSet(classes, DEF_IDENTIFICATION))
        .with(MD_TYPE_DEFINITIONS, Value::StrongSet(types, DEF_IDENTIFICATION))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn set_of(o: &Obj, pid: u16) -> &[Obj] {
        match o.get(pid) {
            Some(Value::StrongSet(items, _)) => items,
            _ => &[],
        }
    }

    #[test]
    fn every_line_of_the_model_is_read() {
        let lines = MODEL.lines().filter(|l| !l.starts_with('#') && !l.is_empty());
        let (mut classes, mut props, mut types) = (0, 0, 0);
        for l in lines {
            match l.split('\t').next() {
                Some("class") => classes += 1,
                Some("prop") => props += 1,
                Some("type") => types += 1,
                other => panic!("unknown line kind {other:?}"),
            }
        }
        let d = dictionary();
        assert_eq!(set_of(&d, MD_CLASS_DEFINITIONS).len(), classes);
        assert_eq!(set_of(&d, MD_TYPE_DEFINITIONS).len(), types);
        assert_eq!(set_of(&d, MD_CLASS_DEFINITIONS).iter().map(|c| set_of(c, CLASS_PROPERTIES).len()).sum::<usize>(), props);
    }

    /// Definitions are unique and every reference (parent class, property type, referenced class or
    /// type) resolves inside the dictionary; a reader looks them up by AUID.
    #[test]
    fn the_definitions_are_unique_and_closed() {
        let d = dictionary();
        let mut classes = HashSet::new();
        for c in set_of(&d, MD_CLASS_DEFINITIONS) {
            assert!(classes.insert(c.auid(DEF_IDENTIFICATION).unwrap()), "duplicate class");
        }
        let mut types = HashSet::new();
        for t in set_of(&d, MD_TYPE_DEFINITIONS) {
            assert!(types.insert(t.auid(DEF_IDENTIFICATION).unwrap()), "duplicate type");
        }
        for c in set_of(&d, MD_CLASS_DEFINITIONS) {
            let parent: Auid = c.weak_key(CLASS_PARENT).unwrap().try_into().unwrap();
            assert!(classes.contains(&parent), "{}: parent", c.string(DEF_NAME).unwrap_or_default());
            let mut locals = HashSet::new();
            for p in set_of(c, CLASS_PROPERTIES) {
                let name = p.string(DEF_NAME).unwrap_or_default();
                assert!(types.contains(&p.auid(PROP_TYPE).unwrap()), "{name}: type");
                assert!(locals.insert(p.u16(PROP_LOCAL_ID).unwrap()), "{name}: local id used twice in a class");
            }
        }
        for t in set_of(&d, MD_TYPE_DEFINITIONS) {
            for (_, v) in &t.props {
                let keys: Vec<&[u8]> = match v {
                    Value::Weak(_, _, k) => vec![k.as_slice()],
                    Value::WeakVec(_, _, ks) => ks.iter().map(Vec::as_slice).collect(),
                    _ => continue,
                };
                for k in keys {
                    let k: Auid = k.try_into().unwrap();
                    assert!(classes.contains(&k) || types.contains(&k), "{}: reference", t.string(DEF_NAME).unwrap_or_default());
                }
            }
        }
    }
}
