# cfa4-vdb-worker (N1, CFA4-DNEG-TECHNOLOGY-ADAPTATION-001)

Separate-process OpenVDB reader (C++, Apache-2.0 library; owner approval Q3, 2026-11-21). The Rust side is `crates/vdb`
(`cfa4-vdb`): it runs one command per process, bounds time and output, and turns every worker failure into an `Err`.

Build (optional, never part of `cargo` or CI; the user installs the packages, nothing is downloaded):

    sudo apt install cmake g++ libopenvdb-dev libtbb-dev libboost-iostreams-dev libboost-system-dev
    cmake -S workers/vdb -B target/vdb-worker -DCMAKE_BUILD_TYPE=Release && cmake --build target/vdb-worker
    cargo build -p cfa4-vdb --release   # builds the launcher too
    CFA4_VDB_WORKER=$PWD/target/vdb-worker/cfa4-vdb-worker cargo test -p cfa4-vdb --release

Without the worker the real-worker tests return early and the engine works as before (`Error::Unavailable`).

## N1 acceptance status
| Criterion | State |
|---|---|
| N1-1 `vdb.inspect` | done: worker, host and the engine command `vdb.inspect`, tested on a synthetic file; Qt panel open (N1-10) |
| N1-2 ≥ 1 GiB, memory | done with a deviation: a 650³ dense file (2.7·10⁸ active voxels = 1.02 GiB of data, 122 MB on disk) is inspected with a peak RSS of ~160 MB (the worker reports `VmHWM`; `tests/big.rs`, `--ignored`). Measured as the worker's own peak RSS, **not** under a cgroup limit (not available in this container); sparse files are covered by the small synthetic test |
| N1-3 hostile files | done for the unit: 7 500 mutated/truncated files (`CFA4_VDB_FUZZ_ITERS`) never panic the host or hang; the first 5 000-file run showed an out-of-memory kill of a worker, so the worker now caps its address space (`CFA4_VDB_MEM_LIMIT_MB`, default 4096) and the 2 500-file rerun was clean. Crash/hang/garbage/flood fakes also covered. A coverage-guided fuzzer (cargo-fuzz style) is not set up |
| N1-4 round trip | grid and metadata identity on the synthetic file; bit-identity over all supported types **open** |
| N1-5 image sequence import | done: `vdb.import` makes one `ItemKind::Volume` item for a file or a numbered `.vdb` sequence (gaps reported), no timeline; tested |
| N1-6 CPU only, GPUs hidden | by construction (the worker links no GPU library and opens no device; the sandbox grants no GPU nodes); the run on the reference machine with both cards hidden and `--hw off` is **open** (no GPU here) |
| N1-7 project save/restore | done: the reference is saved with the project and restored (`*t.project == *s.project`), `vdb.check` reports missing files, `vdb.relink` repoints (undoable); tested |
| N1-8 sandbox profile | done: `cfa4-vdb-launch` confines itself (Landlock + seccomp, strict) and `exec`s the worker: reads only the granted file, writes only the granted output, no network, no other program; tested with a probe worker and the real worker (skips on a kernel without Landlock). The engine uses it when the launcher is found next to the worker, in `CFA4_VDB_LAUNCHER` or in `/opt/cfa4/bin` |
| N1-9 frame-time reference | **open**: needs the reference-machine measurement before/after (docs/development/reference-machine.md); not measurable in this container |
| N1-10 Qt6 panel | **open** |

## N2 acceptance status (volume preview as plain media)
`cfa4-vdb-worker render` ray-marches one float grid (orthographic, step 2 / 1 / 0.5 voxels for `preview` / `standard` / `high`) into an RGBA PNG; `vdb.render` imports the PNG as an ordinary still. EXR/AOV output is not done (single-part PNG, as the plan allows for the first round).

| Criterion | State |
|---|---|
| N2-1 quality levels, no silent reduction | done: three levels; the level that was asked for is the level that runs (tested twice in a row); the engine never lowers it |
| N2-2 determinism | done: every pixel is independent, so two runs and one thread against many give **byte-identical** PNGs (no tolerance needed; tested) |
| N2-3 plain clip, no new render path | done: the PNG is imported with `file.import` and placed with `timeline.place` (tested). Note: N1 added `Volume` arms to three exhaustive `match`es in `crates/render` (no new path, but the literal "git diff of crates/render is 0" does not hold) |
| N2-4 cancel, time limit, OOM → error | done at the host: `Worker::render` kills the worker on cancel (<5 s) and on the time limit; the worker's address-space cap turns an out-of-memory render into an error (tested). The `vdb.render` command runs synchronously and has no cancel from the GUI yet |
| N2-5 background, GUI responsive | **open**: needs the engine's background-job queue (ACS-6, PR #1471, not on `main`); `vdb.render` currently blocks the engine command loop for the length of the render |

## X2: the CFX bridge (plan CFA4-AX-INTERPRETER-001)
`export-tiles <in> <grid> <out.tiles>` writes one float or `vec3s` grid as leaf tiles (active tiles become leaves first);
`import-tiles <ref.vdb> <ref-grid> <out.vdb> (<name> <tiles>)...` writes `ref.vdb`'s grids plus new grids made from tiles,
with the transform of `ref-grid` (a new grid replaces an old one of the same name). The tile layout is documented in
`crates/vdb/src/tiles.rs`; the Rust side checks every length against the file size, the worker does the same on import.
`Worker::apply_expression` (`crates/vdb/src/expr_run.rs`) evaluates a CFX program on every active voxel of a *domain*
grid with a thread pool and writes the result; `expr.run_preview` (engine) imports the result as a new volume item and,
with `render: true`, renders a preview of the written grid through `vdb.render`.

| Criterion | State |
|---|---|
| N3-S1′/N3-S2′ results independent of thread count | done: 1, 2, 7 and 16 threads give byte-identical output tiles (tested, incl. NaN and `noise`/`hash`); every leaf has its own result slot and the first error (lowest leaf) is the one reported |
| fuel interrupts and errors | done: `R-FUEL` (and `R-INT`) stop the run, name the voxel, import nothing; leaves are claimed in increasing order so the reported error does not depend on timing |
| Q-X2 limits: 30 s and 1 GiB for a preview | done as limits: a time limit of 30 s for the whole run (`Error::Timeout`), 1 GiB of active voxel data (`Error::Limit`); a caller can only lower them. **Measured once here** (4 vCPU cloud container, not the reference machine): 650³ = 274 625 000 voxels (1.02 GiB), `@o = @big * 2.0 + P.x`, 4 threads, **26.9 s**. The reference-machine figure is a phase X5 task |
| cancel | done: a cancel flag stops the evaluation between 16 voxels and kills a running worker step; temporary files are removed on every exit |
| hostile tile files | done: unit tests with truncation, every header byte mutated, absurd leaf count (no allocation before the size check) |

Not yet: integer and bool grids, points context, grids with a rotation or translation (a voxel's world position is
`ijk * voxelSize`), partial (incremental) re-evaluation, a GUI cancel (the command is synchronous, like `vdb.render`; N2-5/ACS-6).
Attributes that are both read and written start from the existing value, so a path that does not assign leaves it as it was.
