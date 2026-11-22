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
| N1-3 hostile files | host side tested (60 mutated/truncated files, crash/hang/garbage/flood fakes); long fuzz run **open** |
| N1-4 round trip | grid and metadata identity on the synthetic file; bit-identity over all supported types **open** |
| N1-5 image sequence import | done: `vdb.import` makes one `ItemKind::Volume` item for a file or a numbered `.vdb` sequence (gaps reported), no timeline; tested |
| N1-6 CPU only, GPUs hidden | **open** (the worker uses no GPU) |
| N1-7 project save/restore | done: the reference is saved with the project and restored (`*t.project == *s.project`), `vdb.check` reports missing files, `vdb.relink` repoints (undoable); tested |
| N1-8 sandbox profile | done: `cfa4-vdb-launch` confines itself (Landlock + seccomp, strict) and `exec`s the worker: reads only the granted file, writes only the granted output, no network, no other program; tested with a probe worker and the real worker (skips on a kernel without Landlock). The engine uses it when the launcher is found next to the worker, in `CFA4_VDB_LAUNCHER` or in `/opt/cfa4/bin` |
| N1-9 frame-time reference | **open** |
| N1-10 Qt6 panel | **open** |
