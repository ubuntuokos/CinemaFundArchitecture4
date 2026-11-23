//! Runs a CFX expression over the active voxels of an OpenVDB file (plan CFA4-AX-INTERPRETER-001, X2).
//!
//! The worker exports the grids the expression reads as leaf tiles ([`crate::tiles`]); this module evaluates the
//! program on every active voxel of the *domain* grid with a pool of threads and gives the new grids to the worker,
//! which writes them next to the original ones. Each voxel is independent and every leaf's result goes into its own
//! slot, so the output is bit-identical for any thread count. A run is bounded three ways: fuel per voxel (an
//! `R-FUEL` error), a time limit (30 s by default, [`Error::Timeout`]) and an active-data limit (1 GiB by default,
//! [`Error::Limit`]); cancelling stops it between voxels. Nothing here can panic on worker output.
//!
//! Limits of this first bridge: float and `vec3s` grids only; a voxel's world position is `ijk * voxelSize` (grids
//! with a rotation or translation are not supported yet); an attribute that does not exist in the file is created
//! from the type given in [`ExprRequest::outputs`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use cfa4_expr::{Context, Env, Schema, Ty, Value};

use crate::tiles::{LEAF_VOXELS, Leaf, TileSet};
use crate::{Error, Worker};

/// Default time limit of a preview run (owner decision Q-X2, 2026-11-23).
pub const PREVIEW_TIMEOUT: Duration = Duration::from_secs(30);
/// Default most active voxel data a preview run takes: 1 GiB.
pub const PREVIEW_MAX_BYTES: u64 = 1 << 30;

/// What to run.
#[derive(Debug, Clone)]
pub struct ExprRequest {
    /// The CFX source.
    pub source: String,
    /// The grid whose active voxels the expression runs over (a float or `vec3s` grid).
    pub domain: String,
    /// Attributes the expression creates: `(name, type)`, float or vec3 only. An existing grid keeps its own type.
    pub outputs: Vec<(String, Ty)>,
    /// Parameter values by name; the others take their declared defaults.
    pub params: Vec<(String, Value)>,
    /// Instruction budget per voxel.
    pub fuel: u32,
    /// Worker threads; 0 takes the number of CPUs.
    pub threads: usize,
    /// Time limit of the whole run, worker steps included.
    pub timeout: Duration,
    /// The most active voxel data (4 bytes per channel and voxel, all grids that are read or written) the run takes.
    pub max_bytes: u64,
}

impl ExprRequest {
    /// A preview request: default fuel, all CPUs, 30 s, 1 GiB.
    #[must_use]
    pub fn new(source: impl Into<String>, domain: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            domain: domain.into(),
            outputs: Vec::new(),
            params: Vec::new(),
            fuel: cfa4_expr::DEFAULT_FUEL,
            threads: 0,
            timeout: PREVIEW_TIMEOUT,
            max_bytes: PREVIEW_MAX_BYTES,
        }
    }
}

/// What a run reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprReport {
    /// Active voxels evaluated.
    pub voxels: u64,
    /// Leaves evaluated.
    pub leaves: u64,
    /// Instructions executed, all voxels.
    pub fuel_used: u64,
    /// Some output got a NaN (stored as the canonical NaN).
    pub wrote_nan: bool,
    /// Threads used.
    pub threads: usize,
    /// Names of the grids written.
    pub written: Vec<String>,
    /// Compiler warnings, one line each.
    pub warnings: Vec<String>,
    /// Wall time in milliseconds.
    pub elapsed_ms: u64,
}

fn ty_of_grid(value_type: &str) -> Option<Ty> {
    match value_type {
        "float" => Some(Ty::Float),
        "vec3s" => Some(Ty::Vec3),
        _ => None,
    }
}

fn channels(ty: Ty) -> usize {
    if ty == Ty::Vec3 { 3 } else { 1 }
}

fn read_value(ts: &TileSet, leaf: Option<&Leaf>, i: usize, ty: Ty) -> Value {
    match leaf {
        Some(l) => match ty {
            Ty::Vec3 => Value::Vec3([l.values[i * 3], l.values[i * 3 + 1], l.values[i * 3 + 2]]),
            _ => Value::Float(l.values[i]),
        },
        None => match ty {
            Ty::Vec3 => Value::Vec3(ts.background),
            _ => Value::Float(ts.background[0]),
        },
    }
}

struct LeafOut {
    /// One buffer per written attribute, `LEAF_VOXELS * channels` values each.
    values: Vec<Vec<f32>>,
    fuel: u64,
    nan: bool,
}

/// The outcome of one leaf: its output, a run-time error with the voxel it happened at, or a stop (cancel/timeout).
enum Done {
    Out(LeafOut),
    Err(cfa4_expr::Diagnostic, [i32; 3]),
    Stopped(Error),
}

impl Worker {
    /// Exports one float or `vec3s` grid as leaf tiles (X2).
    ///
    /// # Errors
    /// Any [`Error`].
    pub fn export_tiles(&self, input: &Path, grid: &str, output: &Path, timeout: Duration, cancel: Option<&AtomicBool>) -> Result<(), Error> {
        let out = self.run_with(&["export-tiles".as_ref(), input.as_os_str(), grid.as_ref(), output.as_os_str()], &[input], &[output], timeout, cancel)?;
        ack(&out)
    }

    /// Writes `reference`'s grids plus one new grid per `(name, tiles)` to `output`; the new grids take the transform
    /// of `reference_grid` (X2).
    ///
    /// # Errors
    /// Any [`Error`].
    pub fn import_tiles(
        &self,
        reference: &Path,
        reference_grid: &str,
        output: &Path,
        grids: &[(String, PathBuf)],
        timeout: Duration,
        cancel: Option<&AtomicBool>,
    ) -> Result<(), Error> {
        let mut args: Vec<&std::ffi::OsStr> = vec!["import-tiles".as_ref(), reference.as_os_str(), reference_grid.as_ref(), output.as_os_str()];
        let mut reads: Vec<&Path> = vec![reference];
        for (name, path) in grids {
            args.push(name.as_ref());
            args.push(path.as_os_str());
            reads.push(path);
        }
        let out = self.run_with(&args, &reads, &[output], timeout, cancel)?;
        ack(&out)
    }

    /// Evaluates a CFX expression over the active voxels of `input`'s domain grid and writes the result, with all
    /// original grids, to `output` (X2). `work_dir` holds the temporary tile files and is cleaned up.
    ///
    /// # Errors
    /// [`Error::Expression`] (compile or run-time diagnostic), [`Error::Limit`], [`Error::Timeout`],
    /// [`Error::Cancelled`], or any worker [`Error`].
    pub fn apply_expression(&self, input: &Path, output: &Path, work_dir: &Path, req: &ExprRequest, cancel: Option<&AtomicBool>) -> Result<ExprReport, Error> {
        let started = Instant::now();
        let deadline = started + req.timeout;
        let left = || deadline.saturating_duration_since(Instant::now());
        let expr_err = |d: cfa4_expr::Diagnostic| Error::Expression(d.to_string());

        let info = self.inspect(input)?;
        let mut schema = Schema { context: Context::Volume, attrs: Vec::new() };
        let mut grid_of: Vec<Option<f64>> = Vec::new(); // Some(voxel size) when the attribute is a grid of the file
        for g in &info.grids {
            if let Some(t) = ty_of_grid(&g.value_type)
                && schema.attrs.iter().all(|(n, _)| n != &g.name)
            {
                schema.attrs.push((g.name.clone(), t));
                grid_of.push(Some(g.voxel_size[0]));
            }
        }
        for (name, ty) in &req.outputs {
            if !matches!(ty, Ty::Float | Ty::Vec3) {
                return Err(Error::Expression(format!("output @{name}: only float and vec3 grids can be written")));
            }
            match schema.attrs.iter().find(|(n, _)| n == name) {
                Some((_, t)) if t != ty => return Err(Error::Expression(format!("output @{name}: the file's grid has another type"))),
                Some(_) => {}
                None => {
                    schema.attrs.push((name.clone(), *ty));
                    grid_of.push(None);
                }
            }
        }
        let dom = schema.attrs.iter().position(|(n, _)| n == &req.domain).filter(|i| grid_of[*i].is_some());
        let Some(dom) = dom else {
            return Err(Error::Expression(format!("domain grid '{}' is missing or is not a float or vec3s grid", req.domain)));
        };
        let prog = cfa4_expr::compile(&req.source, &schema).map_err(expr_err)?;
        let warnings: Vec<String> = prog.warnings().iter().map(ToString::to_string).collect();
        let writes: Vec<usize> = prog.writes().to_vec();
        if writes.is_empty() {
            return Err(Error::Expression("the expression writes nothing".into()));
        }
        // parameters by declaration order
        let mut pvals: Vec<Value> = Vec::new();
        for d in prog.params() {
            let v = req.params.iter().find(|(n, _)| n == &d.name).map_or(d.default, |(_, v)| *v);
            // a JSON `2` for a float parameter is the float 2
            pvals.push(match (v, d.ty) {
                (Value::Int(i), Ty::Float) => Value::Float(i as f32),
                (v, _) => v,
            });
        }
        for (n, _) in &req.params {
            if prog.params().iter().all(|d| &d.name != n) {
                return Err(Error::Expression(format!("unknown parameter '{n}'")));
            }
        }
        // which grids are needed in memory: the domain, what is read and what is both written and present
        let mut need: Vec<usize> = vec![dom];
        for &i in prog.reads().iter().chain(&writes) {
            if grid_of[i].is_some() && !need.contains(&i) {
                need.push(i);
            }
        }
        let mut bytes: u64 = 0;
        for &i in &need {
            let name = &schema.attrs[i].0;
            let active = info.grids.iter().find(|g| &g.name == name).map_or(0, |g| g.active_voxels);
            bytes = bytes.saturating_add(active.saturating_mul(4 * channels(schema.attrs[i].1) as u64));
        }
        if bytes > req.max_bytes {
            return Err(Error::Limit(format!("{bytes} bytes of active voxel data exceed the limit of {} bytes", req.max_bytes)));
        }
        std::fs::create_dir_all(work_dir).map_err(|e| Error::Spawn(format!("{}: {e}", work_dir.display())))?;
        let guard = Cleanup(work_dir.to_path_buf());
        let mut tiles: Vec<Option<TileSet>> = (0..schema.attrs.len()).map(|_| None).collect();
        for &i in &need {
            let f = work_dir.join(format!("in-{i}.tiles"));
            self.export_tiles(input, &schema.attrs[i].0, &f, left(), cancel)?;
            tiles[i] = Some(TileSet::read(&f)?);
            let _ = std::fs::remove_file(&f);
        }
        let domain = tiles[dom].as_ref().ok_or_else(|| Error::BadAnswer("domain tiles missing".into()))?;
        let voxel_size = domain.voxel_size as f32;
        let index: Vec<Option<HashMap<[i32; 3], usize>>> =
            tiles.iter().map(|t| t.as_ref().map(|t| t.leaves.iter().enumerate().map(|(k, l)| (l.origin, k)).collect())).collect();

        let n_leaves = domain.leaves.len();
        let threads = if req.threads == 0 { std::thread::available_parallelism().map_or(1, usize::from) } else { req.threads }.clamp(1, 256);
        let next = AtomicUsize::new(0);
        let first_err = AtomicUsize::new(usize::MAX);
        let stopped = AtomicBool::new(false);
        let eval_leaf = |vm: &mut cfa4_expr::Vm, k: usize| -> Done {
            let leaf = &domain.leaves[k];
            let mut out = LeafOut { values: writes.iter().map(|&w| vec![0f32; LEAF_VOXELS * channels(schema.attrs[w].1)]).collect(), fuel: 0, nan: false };
            let src: Vec<Option<&Leaf>> = (0..schema.attrs.len())
                .map(|a| tiles[a].as_ref().and_then(|t| index[a].as_ref().and_then(|ix| ix.get(&leaf.origin)).map(|&j| &t.leaves[j])))
                .collect();
            let mut inputs = vec![Value::Float(0.0); schema.attrs.len()];
            let mut outputs = inputs.clone();
            for i in 0..LEAF_VOXELS {
                if !leaf.is_active(i) {
                    continue;
                }
                if i & 15 == 0 && (stopped.load(Ordering::Relaxed) || cancel.is_some_and(|c| c.load(Ordering::Relaxed)) || Instant::now() >= deadline) {
                    let e = if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) { Error::Cancelled } else { Error::Timeout };
                    return Done::Stopped(e);
                }
                let ijk = leaf.coord(i);
                for a in 0..schema.attrs.len() {
                    let ty = schema.attrs[a].1;
                    inputs[a] = match &tiles[a] {
                        Some(ts) => read_value(ts, src[a], i, ty),
                        None => Value::zero(ty),
                    };
                    outputs[a] = inputs[a];
                }
                let env = Env { p: [ijk[0] as f32 * voxel_size, ijk[1] as f32 * voxel_size, ijk[2] as f32 * voxel_size], ijk, voxel_size, id: 0 };
                match prog.run(vm, req.fuel, &env, &pvals, &inputs, &mut outputs) {
                    Ok(st) => {
                        out.fuel += u64::from(st.fuel_used);
                        out.nan |= st.wrote_nan;
                    }
                    Err(d) => return Done::Err(d, ijk),
                }
                for (slot, &w) in writes.iter().enumerate() {
                    match outputs[w] {
                        Value::Float(f) => out.values[slot][i] = f,
                        Value::Vec3(v) => out.values[slot][i * 3..i * 3 + 3].copy_from_slice(&v),
                        _ => {}
                    }
                }
            }
            // inactive voxels (and the non-active part of a leaf) keep the existing grid's values when there are any
            for (slot, &w) in writes.iter().enumerate() {
                if let (Some(src_leaf), ch) = (src[w], channels(schema.attrs[w].1)) {
                    for i in 0..LEAF_VOXELS {
                        if !leaf.is_active(i) {
                            out.values[slot][i * ch..(i + 1) * ch].copy_from_slice(&src_leaf.values[i * ch..(i + 1) * ch]);
                        }
                    }
                }
            }
            Done::Out(out)
        };
        let mut results: Vec<(usize, Done)> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads.min(n_leaves.max(1)))
                .map(|_| {
                    s.spawn(|| {
                        let mut vm = prog.new_vm();
                        let mut mine = Vec::new();
                        loop {
                            let k = next.fetch_add(1, Ordering::Relaxed);
                            // leaves are claimed in increasing order, so every leaf below an error is finished by someone
                            if k >= n_leaves || k > first_err.load(Ordering::Relaxed) {
                                break;
                            }
                            let d = eval_leaf(&mut vm, k);
                            match &d {
                                Done::Err(..) => {
                                    first_err.fetch_min(k, Ordering::Relaxed);
                                }
                                Done::Stopped(_) => stopped.store(true, Ordering::Relaxed),
                                Done::Out(_) => {}
                            }
                            mine.push((k, d));
                        }
                        mine
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        results.sort_by_key(|(k, _)| *k);
        if let Some(pos) = results.iter().position(|(_, d)| matches!(d, Done::Stopped(_)))
            && let Done::Stopped(e) = results.swap_remove(pos).1
        {
            return Err(e);
        }
        if let Some((_, Done::Err(d, ijk))) = results.iter().find(|(_, d)| matches!(d, Done::Err(..))) {
            return Err(Error::Expression(format!("{d} (voxel {}, {}, {})", ijk[0], ijk[1], ijk[2])));
        }
        if results.len() != n_leaves {
            return Err(Error::WorkerDied("a thread of the evaluation pool died".into()));
        }
        // assemble one tile set per written attribute and hand them to the worker
        let mut fuel_used = 0u64;
        let mut wrote_nan = false;
        let mut sets: Vec<TileSet> = writes
            .iter()
            .map(|&w| TileSet {
                channels: channels(schema.attrs[w].1),
                voxel_size: domain.voxel_size,
                background: tiles[w].as_ref().map_or([0.0; 3], |t| t.background),
                leaves: Vec::with_capacity(n_leaves),
            })
            .collect();
        for (k, d) in results {
            let Done::Out(o) = d else { continue };
            fuel_used += o.fuel;
            wrote_nan |= o.nan;
            for (set, vals) in sets.iter_mut().zip(o.values) {
                set.leaves.push(Leaf { origin: domain.leaves[k].origin, active: domain.leaves[k].active, values: vals });
            }
        }
        let mut files = Vec::new();
        for (slot, &w) in writes.iter().enumerate() {
            let f = work_dir.join(format!("out-{slot}.tiles"));
            sets[slot].write(&f)?;
            files.push((schema.attrs[w].0.clone(), f));
        }
        self.import_tiles(input, &req.domain, output, &files, left(), cancel)?;
        drop(guard);
        Ok(ExprReport {
            voxels: domain.active_voxels(),
            leaves: n_leaves as u64,
            fuel_used,
            wrote_nan,
            threads,
            written: files.into_iter().map(|(n, _)| n).collect(),
            warnings,
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }
}

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ack(out: &str) -> Result<(), Error> {
    match serde_json::from_str::<serde_json::Value>(out) {
        Ok(v) if v.get("ok").and_then(serde_json::Value::as_bool) == Some(true) => Ok(()),
        _ => Err(Error::BadAnswer(out.chars().take(200).collect())),
    }
}
