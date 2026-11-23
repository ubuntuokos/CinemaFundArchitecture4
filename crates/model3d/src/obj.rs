//! Wavefront OBJ reader (with MTL material libraries through the [`Resolver`]).
//!
//! Carried: `v`, `vt`, `vn`, `f` (polygons expanded by a triangle fan, negative indices), `o` / `g`
//! (one node per object or group, a primitive per `usemtl`), `usemtl`, `mtllib` (name, `Kd`, `d`/`Tr`,
//! `Ke`, texture maps as references). Everything else lands in the report: lines and points, free-form
//! curves and surfaces, vertex colours, smoothing groups, the other MTL statements. OBJ has no
//! hierarchy, cameras or animation.

use std::collections::HashMap;

use crate::util::{Budget, finite_f32, safe_relative_uri};
use crate::{Error, Format, Limits, Material, Mesh, Node, Primitive, Resolver, Result, Scene, TextureRef, Transform, Units, invalid};

type Key = (u32, u32, u32);

struct PrimBuild {
    material: Option<usize>,
    map: HashMap<Key, u32>,
    prim: Primitive,
    with_normal: usize,
    with_uv: usize,
}

struct ObjectBuild {
    name: Option<String>,
    prims: Vec<PrimBuild>,
}

struct Reader<'a> {
    limits: &'a Limits,
    budget: Budget,
    scene: Scene,
    pos: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    nrm: Vec<[f32; 3]>,
    cur: Option<ObjectBuild>,
    cur_mat: Option<usize>,
    seen_kw: Vec<String>,
    polygon_noted: bool,
}

const FREEFORM: &[&str] = &["cstype", "deg", "bmat", "step", "curv", "curv2", "surf", "parm", "trim", "hole", "scrv", "sp", "end", "con", "vp"];

pub(crate) fn read(data: &[u8], resolver: &mut dyn Resolver, limits: &Limits) -> Result<Scene> {
    let mut r = Reader {
        limits,
        budget: Budget::new(limits.max_memory_bytes),
        scene: Scene::empty(Format::Obj, Units::Unspecified, limits.max_report_items),
        pos: Vec::new(),
        uv: Vec::new(),
        nrm: Vec::new(),
        cur: None,
        cur_mat: None,
        seen_kw: Vec::new(),
        polygon_noted: false,
    };
    let mut pending = String::new();
    for (i, raw) in data.split(|b| *b == b'\n').enumerate() {
        let line = String::from_utf8_lossy(raw);
        let line = line.trim_end_matches('\r');
        if let Some(stem) = line.strip_suffix('\\') {
            pending.push_str(stem);
            pending.push(' ');
            if pending.len() > limits.max_input_bytes {
                return Err(Error::Limit("OBJ line continuation too long".into()));
            }
            continue;
        }
        if pending.is_empty() {
            r.line(line, i + 1, resolver)?;
        } else {
            pending.push_str(line);
            let joined = std::mem::take(&mut pending);
            r.line(&joined, i + 1, resolver)?;
        }
    }
    r.finish_object()?;
    if r.scene.nodes.is_empty() {
        return invalid("OBJ file contains no faces");
    }
    Ok(r.scene)
}

impl Reader<'_> {
    fn note_once(&mut self, kw: &str, ln: usize, message: &str) {
        if !self.seen_kw.iter().any(|k| k == kw) {
            self.seen_kw.push(kw.to_string());
            self.scene.report.unsupported(format!("line {ln}: {kw}"), message);
        }
    }

    fn line(&mut self, line: &str, ln: usize, resolver: &mut dyn Resolver) -> Result<()> {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut tok = line.split_whitespace();
        let Some(kw) = tok.next() else { return Ok(()) };
        match kw {
            "v" => {
                let c: Vec<&str> = tok.collect();
                if c.len() < 3 {
                    return invalid(format!("line {ln}: vertex needs three coordinates"));
                }
                if c.len() == 6 {
                    self.note_once("v-color", ln, "vertex colours are not carried");
                }
                self.budget.take(1, std::mem::size_of::<[f32; 3]>())?;
                self.pos.push([finite_f32(c[0])?, finite_f32(c[1])?, finite_f32(c[2])?]);
            }
            "vt" => {
                let c: Vec<&str> = tok.collect();
                if c.is_empty() {
                    return invalid(format!("line {ln}: texcoord needs a coordinate"));
                }
                let v = c.get(1).map(|s| finite_f32(s)).transpose()?.unwrap_or(0.0);
                self.budget.take(1, std::mem::size_of::<[f32; 2]>())?;
                self.uv.push([finite_f32(c[0])?, v]);
            }
            "vn" => {
                let c: Vec<&str> = tok.collect();
                if c.len() < 3 {
                    return invalid(format!("line {ln}: normal needs three components"));
                }
                self.budget.take(1, std::mem::size_of::<[f32; 3]>())?;
                self.nrm.push([finite_f32(c[0])?, finite_f32(c[1])?, finite_f32(c[2])?]);
            }
            "f" => self.face(tok.collect(), ln)?,
            "o" | "g" => {
                let name = line.get(kw.len()..).unwrap_or("").trim();
                self.start_object(name)?;
            }
            "usemtl" => {
                let name = line.get(kw.len()..).unwrap_or("").trim();
                self.cur_mat = Some(self.material_named(name)?);
            }
            "mtllib" => {
                let name = line.get(kw.len()..).unwrap_or("").trim();
                self.mtllib(name, ln, resolver)?;
            }
            "l" | "p" => self.note_once(kw, ln, "lines and points are not carried"),
            "s" => self.note_once("s", ln, "smoothing groups are not carried (stated normals are)"),
            k if FREEFORM.contains(&k) => self.note_once(k, ln, "free-form curves and surfaces are not carried"),
            k => self.note_once(k, ln, "unknown statement ignored"),
        }
        Ok(())
    }

    fn material_named(&mut self, name: &str) -> Result<usize> {
        if let Some(i) = self.scene.materials.iter().position(|m| m.name.as_deref() == Some(name)) {
            return Ok(i);
        }
        let name = self.budget.string(name)?;
        self.budget.take(1, std::mem::size_of::<Material>())?;
        self.scene.materials.push(Material::named(Some(name)));
        Ok(self.scene.materials.len() - 1)
    }

    fn start_object(&mut self, name: &str) -> Result<()> {
        // a group statement before any face just renames the pending object
        if let Some(c) = &mut self.cur
            && c.prims.is_empty()
        {
            c.name = if name.is_empty() { None } else { Some(name.to_string()) };
            return Ok(());
        }
        self.finish_object()?;
        let name = if name.is_empty() { None } else { Some(self.budget.string(name)?) };
        self.cur = Some(ObjectBuild { name, prims: Vec::new() });
        Ok(())
    }

    fn finish_object(&mut self) -> Result<()> {
        let Some(obj) = self.cur.take() else { return Ok(()) };
        if obj.prims.is_empty() {
            return Ok(());
        }
        if self.scene.nodes.len() >= self.limits.max_nodes {
            return Err(Error::Limit("too many OBJ objects".into()));
        }
        let mut mesh = Mesh { name: obj.name.clone(), primitives: Vec::with_capacity(obj.prims.len()) };
        for mut pb in obj.prims {
            if pb.with_normal != 0 && pb.with_normal != pb.prim.positions.len() {
                self.scene.report.approximated("normals", "some vertices have no normal: zero-filled");
            } else if pb.with_normal == 0 {
                pb.prim.normals.clear();
            }
            if pb.with_uv != 0 && pb.with_uv != pb.prim.positions.len() {
                self.scene.report.approximated("texcoords", "some vertices have no texcoord: zero-filled");
            } else if pb.with_uv == 0 {
                pb.prim.texcoords.clear();
            }
            mesh.primitives.push(pb.prim);
        }
        let mi = self.scene.meshes.len();
        self.scene.meshes.push(mesh);
        let ni = self.scene.nodes.len();
        self.scene.nodes.push(Node { name: obj.name, parent: None, children: Vec::new(), transform: Transform::IDENTITY, mesh: Some(mi), camera: None });
        self.scene.roots.push(ni);
        Ok(())
    }

    fn resolve_index(raw: &str, len: usize, what: &str, ln: usize) -> Result<u32> {
        let n: i64 = raw.parse().map_err(|_| Error::Invalid(format!("line {ln}: bad {what} index {raw:?}")))?;
        let len_i = i64::try_from(len).map_err(|_| Error::Limit("too many OBJ elements".into()))?;
        let idx = match n {
            0 => return invalid(format!("line {ln}: {what} index 0")),
            n if n > 0 => n - 1,
            n => len_i + n,
        };
        if idx < 0 || idx >= len_i {
            return invalid(format!("line {ln}: {what} index {n} out of range (have {len})"));
        }
        u32::try_from(idx + 1).map_err(|_| Error::Limit("OBJ index too large".into()))
    }

    fn face(&mut self, verts: Vec<&str>, ln: usize) -> Result<()> {
        if verts.len() < 3 {
            return invalid(format!("line {ln}: face with fewer than three vertices"));
        }
        if verts.len() > 3 && !self.polygon_noted {
            self.polygon_noted = true;
            self.scene.report.approximated(format!("line {ln}: f"), "polygons with more than three vertices are triangulated as a fan (assumes convex)");
        }
        let mut keys: Vec<Key> = Vec::with_capacity(verts.len());
        for v in &verts {
            let mut parts = v.split('/');
            let vi = Self::resolve_index(parts.next().unwrap_or(""), self.pos.len(), "vertex", ln)?;
            let ti = match parts.next() {
                Some(s) if !s.is_empty() => Self::resolve_index(s, self.uv.len(), "texcoord", ln)?,
                _ => 0,
            };
            let ni = match parts.next() {
                Some(s) if !s.is_empty() => Self::resolve_index(s, self.nrm.len(), "normal", ln)?,
                _ => 0,
            };
            if parts.next().is_some() {
                return invalid(format!("line {ln}: too many index parts"));
            }
            keys.push((vi, ti, ni));
        }
        if self.cur.is_none() {
            self.cur = Some(ObjectBuild { name: None, prims: Vec::new() });
        }
        let mat = self.cur_mat;
        let mut ids = Vec::with_capacity(keys.len());
        for key in keys {
            ids.push(self.vertex(key, mat)?);
        }
        let Some(obj) = self.cur.as_mut() else { return Ok(()) };
        let Some(pb) = obj.prims.iter_mut().find(|p| p.material == mat) else { return Ok(()) };
        let tris = ids.len() - 2;
        self.budget.take(tris * 3, 4)?;
        for t in 0..tris {
            pb.prim.indices.extend_from_slice(&[ids[0], ids[t + 1], ids[t + 2]]);
        }
        Ok(())
    }

    /// Index of the deduplicated vertex in the primitive for the current object and material.
    fn vertex(&mut self, key: Key, mat: Option<usize>) -> Result<u32> {
        let Some(obj) = self.cur.as_mut() else { return invalid("face outside an object") };
        let at = match obj.prims.iter().position(|p| p.material == mat) {
            Some(i) => i,
            None => {
                obj.prims.push(PrimBuild {
                    material: mat,
                    map: HashMap::new(),
                    prim: Primitive { material: mat, ..Primitive::default() },
                    with_normal: 0,
                    with_uv: 0,
                });
                obj.prims.len() - 1
            }
        };
        let Some(pb) = obj.prims.get_mut(at) else { return invalid("primitive vanished") };
        if let Some(&id) = pb.map.get(&key) {
            return Ok(id);
        }
        // charged: position + normal + uv + the dedup map entry
        self.budget.take(1, std::mem::size_of::<[f32; 3]>() * 2 + std::mem::size_of::<[f32; 2]>() + 32)?;
        let id = u32::try_from(pb.prim.positions.len()).map_err(|_| Error::Limit("too many OBJ vertices".into()))?;
        let (vi, ti, ni) = key;
        pb.prim.positions.push(self.pos.get(vi as usize - 1).copied().unwrap_or([0.0; 3]));
        pb.prim.normals.push(if ni > 0 { self.nrm.get(ni as usize - 1).copied().unwrap_or([0.0; 3]) } else { [0.0; 3] });
        pb.prim.texcoords.push(if ti > 0 { self.uv.get(ti as usize - 1).copied().unwrap_or([0.0; 2]) } else { [0.0; 2] });
        if ni > 0 {
            pb.with_normal += 1;
        }
        if ti > 0 {
            pb.with_uv += 1;
        }
        pb.map.insert(key, id);
        Ok(id)
    }

    fn mtllib(&mut self, name: &str, ln: usize, resolver: &mut dyn Resolver) -> Result<()> {
        if !safe_relative_uri(name) {
            self.scene.report.unsupported(format!("line {ln}: mtllib"), format!("unsafe or empty material library path {name:?} not opened"));
            return Ok(());
        }
        let Some(bytes) = resolver.resolve(name) else {
            self.scene.report.unsupported(format!("line {ln}: mtllib"), format!("material library {name:?} was not supplied"));
            return Ok(());
        };
        if bytes.len() > self.limits.max_input_bytes {
            return Err(Error::Limit(format!("material library {name:?} is too large")));
        }
        self.parse_mtl(&bytes, name)
    }

    fn parse_mtl(&mut self, bytes: &[u8], file: &str) -> Result<()> {
        let mut cur: Option<usize> = None;
        let mut noted: Vec<String> = Vec::new();
        for (i, raw) in bytes.split(|b| *b == b'\n').enumerate() {
            let line = String::from_utf8_lossy(raw);
            let line = line.split('#').next().unwrap_or("").trim();
            let mut tok = line.split_whitespace();
            let Some(kw) = tok.next() else { continue };
            let ln = i + 1;
            let rest = line.get(kw.len()..).unwrap_or("").trim();
            if kw == "newmtl" {
                cur = Some(self.material_named(rest)?);
                continue;
            }
            let Some(mi) = cur else {
                return invalid(format!("{file} line {ln}: {kw} before newmtl"));
            };
            let args: Vec<&str> = tok.collect();
            let rgb = |a: &[&str]| -> Result<[f32; 3]> {
                let r = finite_f32(a.first().copied().unwrap_or(""))?;
                let g = a.get(1).map(|s| finite_f32(s)).transpose()?.unwrap_or(r);
                let b = a.get(2).map(|s| finite_f32(s)).transpose()?.unwrap_or(r);
                Ok([r, g, b])
            };
            match kw {
                "Kd" => {
                    let c = rgb(&args)?;
                    if let Some(m) = self.scene.materials.get_mut(mi) {
                        m.base_color = [c[0], c[1], c[2], m.base_color[3]];
                        m.metallic = 0.0;
                    }
                }
                "Ke" => {
                    let c = rgb(&args)?;
                    if let Some(m) = self.scene.materials.get_mut(mi) {
                        m.emissive = c;
                    }
                }
                "d" | "Tr" => {
                    let v = finite_f32(args.first().copied().unwrap_or(""))?;
                    if let Some(m) = self.scene.materials.get_mut(mi) {
                        m.base_color[3] = if kw == "d" { v } else { 1.0 - v };
                    }
                }
                k if k.starts_with("map_") || k == "bump" || k == "disp" || k == "decal" => {
                    let Some(path) = args.last() else { continue };
                    self.budget.take(1, std::mem::size_of::<TextureRef>())?;
                    let slot = self.budget.string(k)?;
                    let uri = if safe_relative_uri(path) {
                        Some(self.budget.string(path)?)
                    } else {
                        self.scene.report.unsupported(format!("{file} line {ln}: {k}"), format!("unsafe texture path {path:?} dropped"));
                        None
                    };
                    if uri.is_some()
                        && let Some(m) = self.scene.materials.get_mut(mi)
                    {
                        m.textures.push(TextureRef { slot, uri, embedded: false, texcoord: 0 });
                    }
                }
                k => {
                    if !noted.iter().any(|n| n == k) {
                        noted.push(k.to_string());
                        self.scene.report.unsupported(format!("{file} line {ln}: {k}"), "MTL statement is not carried (illumination model, Ka/Ks/Ns/Ni, …)");
                    }
                }
            }
        }
        Ok(())
    }
}
