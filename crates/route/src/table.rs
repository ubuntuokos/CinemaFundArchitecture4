//! The routing table: an ordered backend list per family, and per-task rows on top.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{BackendId, Error, Family, TaskId};

/// Largest routing file read (a hostile or corrupted file cannot use unbounded memory).
pub const MAX_TABLE_BYTES: usize = 256 * 1024;
/// Most backends in one ordered list.
pub const MAX_ORDER_LEN: usize = 32;
/// Most per-task rows.
pub const MAX_TASK_RULES: usize = 1024;

/// The table CFA4 ships (`crates/route/routing.toml`, installed as `/usr/share/cfa4/routing.toml`).
pub const BUILTIN_TOML: &str = include_str!("../routing.toml");

/// A per-task row. Anything left out comes from the family's default.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TaskRule {
    /// Replaces the family's order for this task.
    pub order: Option<Vec<BackendId>>,
    /// A GPU or NPU route is taken only when the device reports at least this much free memory.
    pub min_free_vram_mb: Option<u64>,
}

/// Default orders per family plus per-task rows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Table {
    defaults: BTreeMap<Family, Vec<BackendId>>,
    tasks: BTreeMap<TaskId, TaskRule>,
}

// The file format. Other top-level sections (a future `[media]` router) are ignored here; inside
// `[compute]` unknown keys are errors, so a typo is reported instead of silently routing elsewhere.
#[derive(Deserialize)]
struct RawFile {
    compute: Option<RawCompute>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCompute {
    #[serde(default)]
    default: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    task: BTreeMap<String, RawTask>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTask {
    order: Option<Vec<String>>,
    min_free_vram_mb: Option<u64>,
}

fn order_from(raw: Vec<String>, place: &str) -> Result<Vec<BackendId>, Error> {
    if raw.len() > MAX_ORDER_LEN {
        return Err(Error::Table(format!("{place}: more than {MAX_ORDER_LEN} backends")));
    }
    let mut out: Vec<BackendId> = Vec::with_capacity(raw.len());
    for s in raw {
        let id = BackendId::new(s).map_err(|e| Error::Table(format!("{place}: {e}")))?;
        if out.contains(&id) {
            return Err(Error::Table(format!("{place}: {id} is listed twice")));
        }
        out.push(id);
    }
    Ok(out)
}

impl Table {
    /// The shipped table.
    pub fn builtin() -> Result<Table, Error> {
        Table::parse(BUILTIN_TOML)
    }

    /// Reads a routing file. The whole file is refused on any error: a half-read table would
    /// route tasks somewhere the user did not ask for.
    pub fn parse(text: &str) -> Result<Table, Error> {
        if text.len() > MAX_TABLE_BYTES {
            return Err(Error::Table(format!("file larger than {MAX_TABLE_BYTES} bytes")));
        }
        let raw: RawFile = toml::from_str(text).map_err(|e| Error::Table(e.message().to_string()))?;
        let Some(compute) = raw.compute else {
            return Ok(Table::default());
        };
        let mut table = Table::default();
        for (key, list) in compute.default {
            let family = Family::from_key(&key).ok_or_else(|| Error::Table(format!("unknown family {key:?}")))?;
            table.defaults.insert(family, order_from(list, key.as_str())?);
        }
        if compute.task.len() > MAX_TASK_RULES {
            return Err(Error::Table(format!("more than {MAX_TASK_RULES} task rows")));
        }
        for (key, raw) in compute.task {
            let id = TaskId::new(key).map_err(|e| Error::Table(format!("task row: {e}")))?;
            let order = match raw.order {
                Some(list) => Some(order_from(list, id.as_str())?),
                None => None,
            };
            table.tasks.insert(id, TaskRule { order, min_free_vram_mb: raw.min_free_vram_mb });
        }
        Ok(table)
    }

    /// The shipped table with the user's file on top, when there is one. A missing file is the
    /// shipped table; an unreadable, oversized or invalid one is refused whole and the shipped table
    /// is used, with the reason returned for the log.
    pub fn builtin_with_user_file(path: &std::path::Path) -> Result<(Table, Option<String>), Error> {
        use std::io::Read;
        let builtin = Table::builtin()?;
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((builtin, None)),
            Err(e) => return Ok((builtin, Some(format!("{}: {e}", path.display())))),
        };
        let mut text = String::new();
        // one byte more than allowed, so an oversized file is seen as such by `parse`
        let limit = u64::try_from(MAX_TABLE_BYTES).unwrap_or(u64::MAX).saturating_add(1);
        if let Err(e) = file.take(limit).read_to_string(&mut text) {
            return Ok((builtin, Some(format!("{}: {e}", path.display()))));
        }
        match Table::parse(&text) {
            Ok(user) => Ok((builtin.merged(user), None)),
            Err(e) => Ok((builtin, Some(format!("{}: {e}", path.display())))),
        }
    }

    /// The user's routing file: `$XDG_CONFIG_HOME/cfa4/routing.toml`, else
    /// `$HOME/.config/cfa4/routing.toml`; `None` when neither variable is set.
    pub fn user_file_path() -> Option<std::path::PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").filter(|v| !v.is_empty()).map(|h| std::path::PathBuf::from(h).join(".config")))?;
        Some(base.join("cfa4").join("routing.toml"))
    }

    /// This table with `over` on top: a family `over` lists replaces this one's order, a task row
    /// `over` has replaces this one's fields that it sets.
    pub fn merged(mut self, over: Table) -> Table {
        self.defaults.extend(over.defaults);
        for (id, rule) in over.tasks {
            let slot = self.tasks.entry(id).or_default();
            if rule.order.is_some() {
                slot.order = rule.order;
            }
            if rule.min_free_vram_mb.is_some() {
                slot.min_free_vram_mb = rule.min_free_vram_mb;
            }
        }
        self
    }

    /// The order for a task: its own row if it has one, otherwise its family's. Empty when the
    /// table names neither.
    pub fn order(&self, task: &TaskId, family: Family) -> &[BackendId] {
        if let Some(order) = self.tasks.get(task).and_then(|r| r.order.as_deref()) {
            return order;
        }
        self.defaults.get(&family).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn rule(&self, task: &TaskId) -> Option<&TaskRule> {
        self.tasks.get(task)
    }

    pub fn family_order(&self, family: Family) -> &[BackendId] {
        self.defaults.get(&family).map(Vec::as_slice).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> BackendId {
        BackendId::new(s).unwrap()
    }

    #[test]
    fn the_builtin_table_reads_and_ends_every_family_on_the_cpu() {
        let t = Table::builtin().unwrap();
        for f in Family::ALL {
            let order = t.family_order(f);
            assert!(!order.is_empty(), "{f:?} has an order");
            let last = order.last().unwrap().as_str();
            assert!(["cpu-compositor", "software", "candle-cpu", "cpu"].contains(&last), "{f:?} ends on a CPU path, not {last}");
        }
        let whisper = TaskId::new("ai.speech.whisper").unwrap();
        assert_eq!(t.order(&whisper, Family::Ai)[0], id("burn-cuda"));
        assert_eq!(t.rule(&whisper).unwrap().min_free_vram_mb, Some(2048));
    }

    #[test]
    fn our_engine_comes_before_the_supplementary_backends_in_the_codec_families() {
        let t = Table::builtin().unwrap();
        for f in [Family::CodecDecode, Family::CodecEncode] {
            let order = t.family_order(f);
            let pos = |s: &str| order.iter().position(|b| b.as_str() == s).unwrap();
            let gst = order.iter().position(|b| b.as_str().starts_with("gstreamer")).unwrap();
            assert!(pos("vaapi") < pos("ffmpeg"), "{f:?}: our VA-API path first");
            assert!(pos("ffmpeg") < gst, "{f:?}: FFmpeg 9 before GStreamer");
            assert!(gst < pos("software"), "{f:?}: our software codecs stay the last resort");
        }
    }

    #[test]
    fn a_task_without_a_row_uses_its_family() {
        let t = Table::builtin().unwrap();
        let task = TaskId::new("video.decode.hevc").unwrap();
        assert_eq!(t.order(&task, Family::CodecDecode), t.family_order(Family::CodecDecode));
        assert!(t.rule(&task).is_none());
    }

    #[test]
    fn user_file_overrides_family_and_task_fields() {
        let user = Table::parse(
            r#"
            [compute.default]
            ai = ["candle-cpu"]
            [compute.task."ai.speech.whisper"]
            min_free_vram_mb = 4096
            "#,
        )
        .unwrap();
        let t = Table::builtin().unwrap().merged(user);
        assert_eq!(t.family_order(Family::Ai), &[id("candle-cpu")]);
        let whisper = TaskId::new("ai.speech.whisper").unwrap();
        let rule = t.rule(&whisper).unwrap();
        assert_eq!(rule.min_free_vram_mb, Some(4096), "the user's budget wins");
        assert_eq!(rule.order.as_ref().unwrap()[0], id("burn-cuda"), "the shipped order is kept where the user set none");
        assert_eq!(t.family_order(Family::Display)[0], id("wgpu"), "families the user did not list are kept");
    }

    #[test]
    fn other_sections_are_ignored_and_an_empty_file_is_an_empty_table() {
        assert_eq!(Table::parse("").unwrap(), Table::default());
        assert_eq!(Table::parse("[media]\nprimary = \"own\"\n").unwrap(), Table::default());
    }

    #[test]
    fn bad_files_are_refused_whole() {
        let bad = [
            "[compute.default]\nvideo = [\"x\"]\n",                // unknown family
            "[compute.default]\nai = [\"Burn\"]\n",                // invalid id
            "[compute.default]\nai = [\"a\", \"a\"]\n",            // listed twice
            "[compute.default]\nai = \"burn\"\n",                  // not a list
            "[compute]\nprefer = 1\n",                             // unknown key
            "[compute.task.\"x\"]\norder = [\"a\"]\nbudget = 3\n", // unknown task field
            "[compute.task.\"Bad Task\"]\norder = [\"a\"]\n",      // invalid task id
            "[compute.task.\"x\"]\nmin_free_vram_mb = -1\n",       // negative
            "[compute.default\nai = [",                            // broken TOML
        ];
        for text in bad {
            assert!(matches!(Table::parse(text), Err(Error::Table(_))), "refused: {text:?}");
        }
    }

    #[test]
    fn hostile_sizes_are_refused() {
        let long_list = format!("[compute.default]\nai = [{}]\n", (0..=MAX_ORDER_LEN).map(|i| format!("\"b{i}\"")).collect::<Vec<_>>().join(","));
        assert!(Table::parse(&long_list).is_err());

        let mut many = String::new();
        for i in 0..=MAX_TASK_RULES {
            many.push_str(&format!("[compute.task.\"t{i}\"]\nmin_free_vram_mb = 1\n"));
        }
        assert!(Table::parse(&many).is_err());

        let huge = "#".repeat(MAX_TABLE_BYTES + 1);
        assert!(Table::parse(&huge).is_err());
    }

    #[test]
    fn the_user_file_is_merged_and_a_bad_one_falls_back_to_the_shipped_table() {
        let dir = std::env::temp_dir().join(format!("cfa4-route-user-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("routing.toml");
        let builtin = Table::builtin().unwrap();

        assert_eq!(Table::builtin_with_user_file(&path).unwrap(), (builtin.clone(), None), "no file: the shipped table");

        std::fs::write(&path, "[compute.default]\ncodec_decode = [\"vaapi\", \"software\"]\n").unwrap();
        let (t, warn) = Table::builtin_with_user_file(&path).unwrap();
        assert!(warn.is_none());
        assert_eq!(t.family_order(Family::CodecDecode), &[id("vaapi"), id("software")]);

        for bad in ["[compute.default]\nvideo = [\"x\"]\n".to_string(), "#".repeat(MAX_TABLE_BYTES + 10)] {
            std::fs::write(&path, bad).unwrap();
            let (t, warn) = Table::builtin_with_user_file(&path).unwrap();
            assert_eq!(t, builtin);
            assert!(warn.is_some());
        }
        // a directory where the file should be
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(&path).unwrap();
        let (t, warn) = Table::builtin_with_user_file(&path).unwrap();
        assert_eq!((t, warn.is_some()), (builtin, true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mutated_builtin_files_never_panic() {
        // Truncations and byte flips of the shipped file: every result is Ok or Err, never a panic.
        let bytes = BUILTIN_TOML.as_bytes();
        for cut in 0..bytes.len() {
            if let Ok(s) = std::str::from_utf8(&bytes[..cut]) {
                let _ = Table::parse(s);
            }
        }
        let mut seed: u32 = 0x1234_5678;
        for _ in 0..2000 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let mut b = bytes.to_vec();
            let i = seed as usize % b.len();
            b[i] = (seed >> 8) as u8 & 0x7f;
            if let Ok(s) = std::str::from_utf8(&b) {
                let _ = Table::parse(s);
            }
        }
    }
}
