# cfa4-model3d

Clean-room 3D scene readers (and, since **U3a**, writers) for the CFA4 project, phase **U1** of
[CFA4-OPENUSD-3D-001](../../docs/plans/CFA4-OPENUSD-3D-001-1.0.md): glTF 2.0 (`.gltf`, `.glb`),
Wavefront OBJ (+ MTL) and STL (binary and ASCII). Layer L0: the only dependency is `serde_json`, no
`unsafe`, no file I/O, no network. Written from the public format specifications (Khronos glTF 2.0,
the OBJ and MTL file descriptions, the STL file description); no third-party reader code was copied or
consulted.

## API

```rust
use cfa4_model3d::{Limits, read};

let bytes = std::fs::read("scene.glb")?;
let mut resources = |uri: &str| std::fs::read(base.join(uri)).ok(); // the caller confines paths
let scene = read(&bytes, Some("scene.glb"), &mut resources, &Limits::default())?;
for item in &scene.report.items { println!("{:?} {}: {}", item.severity, item.path, item.message); }
```

`Scene` holds the node hierarchy (`nodes`, `roots`, parent/children), transforms (TRS or matrix),
cameras, triangle meshes (`Primitive`: positions, optional normals and texcoords, triangle list,
material index), keyframe animation (`Channel`: translation, rotation, scale; linear, step, cubic
spline), material references (factors and `TextureRef` paths, never pixels) and `Asset` metadata
(generator, copyright). The same type comes out of all three readers, so the project layer
(`scene3d`: path + SHA-256, relink, A/B route) and the ARF descriptors build on one model.

## What is carried and what is reported (U1-A1)

| Format | Carried | Reported, not dropped silently |
|---|---|---|
| glTF / GLB | scenes, nodes, TRS/matrix, perspective/orthographic cameras, triangles/strips/fans, `POSITION`, `NORMAL`, `TEXCOORD_0`, sparse accessors, material factors and texture references, translation/rotation/scale animation | skins, morph targets and weights animation, other vertex attributes, point and line primitives, every extension, non-default scenes, texture samplers, alpha modes |
| OBJ / MTL | `v`, `vt`, `vn`, `f` (polygons as fans, negative indices), `o`/`g` nodes, `usemtl` primitives, `mtllib`, `Kd`, `d`/`Tr`, `Ke`, texture maps | lines and points, free-form curves and surfaces, vertex colours, smoothing groups, other MTL statements, polygon triangulation |
| STL | triangle soup (not welded), facet normals (recomputed when zero) | attribute bytes, trailing bytes |

A *required* glTF extension (Draco, meshopt, …) is an `Error::Unsupported`: the file cannot be shown
correctly without it.

## Hostile input (U1-A2, U1-A3)

Every input-derived count is checked before use; a `Limits` memory budget (`max_memory_bytes`, the ACS
budget when the project layer passes it) is charged before every input-sized allocation, so a small
JSON that references one large accessor a thousand times fails with `Error::Limit`. Node count and
nesting depth are limited, cycles and shared children are errors, buffer and accessor ranges are
bounds-checked, non-finite numbers are rejected, and external URIs with a scheme, an absolute path or a
`..` component never reach the `Resolver`. Mutation tests live in the crate (`tests.rs`) and as the
`model3d` target in [`fuzz/`](../../fuzz/README.md) (libFuzzer plus the stable smoke run).

Region/LOD handling of very large meshes (U1-A3, the second half) is the ARF/`scene3d` layer's job:
this crate bounds memory and returns whole primitives.

## Writers and the loss report (U3a)

Phase **U3a** of [CFA4-OPENUSD-3D-EXPORT-001](../../docs/plans/CFA4-OPENUSD-3D-EXPORT-001-1.0.md): the
same [`Scene`] goes back out as `.glb`, `.gltf` + `.bin`, OBJ + MTL, binary or ASCII STL. Like the
readers the writers do no file I/O: `export` returns the bytes of each file and a loss report.

```rust
use cfa4_model3d::{ExportOptions, NoResources, Target, export};

let e = export(&scene, &ExportOptions::new(Target::Glb, "chair"), &mut NoResources)?;
for item in &e.report.items { println!("{:?} {}: {}", item.severity, item.path, item.message); } // show first
for f in &e.files { std::fs::write(out_dir.join(&f.name), &f.data)?; }                              // then write
```

| Target | Carries | Reported, never dropped silently |
|---|---|---|
| glTF / GLB | everything the model holds: hierarchy, TRS or matrix as stored, cameras, indexed triangles with normals and `TEXCOORD_0`, material factors and texture references (or embedded PNG/JPEG in a `.glb` on request), translation/rotation/scale keys (linear, step, cubic spline) | embedded source images, unknown texture slots, empty meshes and channels, alpha below 1 (`BLEND`), a source whose import was itself lossy |
| OBJ + MTL | meshes with normals and texcoords, materials (`Kd`, `d`, `Ke`, texture maps), `o` per node | hierarchy (flattened, world transforms baked into the vertices), cameras, animation, PBR factors, double-sided, units |
| STL | triangles of every instance as one solid, facet normals | everything else: materials, texcoords, names, copyright text, hierarchy, cameras, animation, smooth normals |

Rules (export rules 1 to 5): the output is **deterministic** (no clock, sorted JSON keys, shortest-`f32`
text, so an f32 survives glTF, OBJ and STL bit for bit); the source `asset.copyright` is kept and the
`generator` is the caller's; units and axes are never converted, the report says where the conventions
differ; an inconsistent scene (index out of range, cycle, NaN, mismatched arrays) is `Error::Invalid`,
never a panic; the output size is estimated against `max_memory_bytes` before anything is built
(`Error::Limit`). `sanitize_stem` turns any name into a safe file name stem. The folder, overwrite and
atomic-write rules belong to the caller: see `scene3d.export` in `crates/engine/src/scene3d_export.rs`.
The fuzz target `model3d` also writes every scene its readers accept and reads the result back.
