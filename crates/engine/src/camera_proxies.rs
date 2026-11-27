//! Camera proxy discovery (Clip ▸ Proxy ▸ Find Camera Proxies): many cameras write a small proxy
//! next to every clip (GoPro `.LRV`, DJI `.LRF`, Akaso `.LRV`, Insta360 `.lrv`, Sony XDCAM `Sub/`).
//! `media.findCameraProxies` looks for those files, checks them like `media.attachProxies` does
//! (duration within one frame, same frame rate) and attaches the ones that pass. Nothing is
//! transcoded.
//!
//! The rule table is derived from Kdenlive's `data/externalproxies.rc` (CC0-1.0, see
//! `ATTRIBUTION.md`). A rule has six fields: proxy directory, proxy prefix, proxy suffix (relative
//! to the proxy directory), then clip directory, clip prefix, clip suffix. Directories are
//! relative: the proxy directory to the clip's folder, the clip directory to the proxy's folder.

use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p};
use crate::proxies::{attach, check_proxy, items_param, media_path};
use crate::{Result, Session};

/// One camera family: `(name, [(proxy dir, proxy prefix, proxy suffix, clip dir, clip prefix, clip suffix)])`.
type Rule = (&'static str, &'static str, &'static str, &'static str, &'static str, &'static str);

const FAMILIES: &[(&str, &[Rule])] = &[
    ("Sony PXW", &[("../Sub", "", "S03.MP4", "../Clip", "", ".MXF")]),
    (
        "GoPro LRV",
        &[
            ("./", "GL", ".LRV", "./", "GX", ".MP4"),
            ("./", "GP", ".LRV", "./", "GP", ".MP4"),
            ("./", "GOPR", ".LRV", "./", "GOPR", ".MP4"),
            ("./", "GL", ".LRV", "./", "GH", ".MP4"),
        ],
    ),
    ("Akaso LRV", &[("./", "", ".LRV", "./", "", ".MP4")]),
    ("DJI LRF", &[("./", "", ".LRF", "./", "", ".MP4")]),
    ("Insta360 AcePro LRV", &[("./", "LRV", ".lrv", "./", "VID", ".mp4"), ("./", "PRO_LRV", ".lrv", "./", "PRO_VID", ".mp4")]),
];

/// Remove `.` and resolve `..` textually (no filesystem access).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

fn strip_suffix_ci<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = s.len().checked_sub(suffix.len())?;
    s.get(cut..)?.eq_ignore_ascii_case(suffix).then(|| &s[..cut])
}

/// The candidate proxy paths of `clip` under one rule (not yet checked for existence).
fn candidate(clip: &Path, rule: &Rule) -> Option<PathBuf> {
    let (pdir, ppre, psuf, cdir, cpre, csuf) = *rule;
    let name = clip.file_name()?.to_str()?;
    let stem = strip_suffix_ci(strip_prefix_ci(name, cpre)?, csuf)?;
    if stem.is_empty() {
        return None;
    }
    let folder = clip.parent()?;
    let proxy_dir = normalize(&folder.join(pdir));
    // the clip directory is relative to the proxy's folder: it must lead back to the clip's folder
    if normalize(&proxy_dir.join(cdir)) != normalize(folder) {
        return None;
    }
    Some(proxy_dir.join(format!("{ppre}{stem}{psuf}")))
}

/// An existing file named like `want`, ignoring case in the file name.
fn existing(want: &Path) -> Option<PathBuf> {
    if want.is_file() {
        return Some(want.to_path_buf());
    }
    let dir = want.parent()?;
    let name = want.file_name()?.to_str()?;
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_str().is_some_and(|n| n.eq_ignore_ascii_case(name)) && e.path().is_file())
        .map(|e| e.path())
}

/// Every existing proxy file for `clip`, with the camera family that matched.
pub fn find_for(clip: &Path) -> Vec<(&'static str, PathBuf)> {
    let mut out: Vec<(&'static str, PathBuf)> = Vec::new();
    for (family, rules) in FAMILIES {
        for rule in *rules {
            if let Some(found) = candidate(clip, rule).and_then(|c| existing(&c))
                && !out.iter().any(|(_, p)| *p == found)
            {
                out.push((family, found));
            }
        }
    }
    out
}

/// `media.findCameraProxies {items?, attach?: true, force?: false}`.
pub fn find(s: &mut Session, p: &Value) -> Result<Value> {
    let attach_found = bool_p(p, "attach").unwrap_or(true);
    let force = bool_p(p, "force").unwrap_or(false);
    let mut found = Vec::new();
    let mut skipped = Vec::new();
    let mut pairs = Vec::new();
    for item in items_param(s, p) {
        let Some((_, path, name)) = media_path(&s.project, item) else {
            skipped.push(json!({"item": item.0, "reason": "not file media"}));
            continue;
        };
        let Some((family, proxy)) = find_for(Path::new(&path)).into_iter().next() else {
            skipped.push(json!({"item": item.0, "name": name, "reason": "no camera proxy next to the clip"}));
            continue;
        };
        let proxy = proxy.to_string_lossy().into_owned();
        if !force && let Err(e) = check_proxy(s, item, &proxy) {
            skipped.push(json!({"item": item.0, "name": name, "reason": e.to_string()}));
            continue;
        }
        found.push(json!({"item": item.0, "name": name, "camera": family, "path": proxy}));
        pairs.push((item, proxy));
    }
    let attached = attach_found && !pairs.is_empty();
    if attached {
        attach(s, &pairs, true)?;
    }
    Ok(json!({"found": found, "attached": attached, "skipped": skipped}))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "media.findCameraProxies",
        label: "Find Camera Proxies",
        menu: &["Clip", "Proxy"],
        shortcut: None,
        params: r#"{"items":[id]?,"attach":bool=true,"force":bool=false}"#,
        enabled: |s| crate::proxies::has_file_media(s),
        run: |s, p| {
            if p.get("items").is_some_and(|v| !v.is_array()) {
                return Err(bad("media.findCameraProxies", "`items` must be an array of item ids"));
            }
            find(s, p)
        },
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn gopro_dji_and_sony_rules() {
        let gopro = FAMILIES[1].1;
        assert_eq!(candidate(&p("/v/GX010001.MP4"), &gopro[0]), Some(p("/v/GL010001.LRV")));
        assert_eq!(candidate(&p("/v/GOPR0042.MP4"), &gopro[2]), Some(p("/v/GOPR0042.LRV")));
        assert_eq!(candidate(&p("/v/other.MP4"), &gopro[0]), None);
        assert_eq!(candidate(&p("/v/DJI_0001.mp4"), &FAMILIES[3].1[0]), Some(p("/v/DJI_0001.LRF")));
        assert_eq!(candidate(&p("/card/Clip/C0001.MXF"), &FAMILIES[0].1[0]), Some(p("/card/Sub/C0001S03.MP4")));
        // the rule only applies to clips inside a folder named `Clip`
        assert_eq!(candidate(&p("/card/Other/C0001.MXF"), &FAMILIES[0].1[0]), None);
    }

    #[test]
    fn insta360_prefixes_are_not_confused() {
        let r = FAMILIES[4].1;
        assert_eq!(candidate(&p("/v/VID_20240101.mp4"), &r[0]), Some(p("/v/LRV_20240101.lrv")));
        assert_eq!(candidate(&p("/v/PRO_VID_2024.mp4"), &r[1]), Some(p("/v/PRO_LRV_2024.lrv")));
    }

    #[test]
    fn empty_stem_and_non_ascii_names_do_not_panic() {
        assert_eq!(candidate(&p("/v/.MP4"), &FAMILIES[3].1[0]), None);
        assert_eq!(candidate(&p("/v/é.mp4"), &FAMILIES[1].1[0]), None);
    }
}
