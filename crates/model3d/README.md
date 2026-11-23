# cfa4-model3d

Clean-room 3D scene readers for the CFA4 project, phase **U1** of
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
