# cfa4-vdb-worker (N1, CFA4-DNEG-TECHNOLOGY-ADAPTATION-001)

Separate-process OpenVDB reader (C++, Apache-2.0 library; owner approval Q3, 2026-11-21). The Rust side is `crates/vdb`
(`cfa4-vdb`): it runs one command per process, bounds time and output, and turns every worker failure into an `Err`.

Build (optional, never part of `cargo` or CI; the user installs the packages, nothing is downloaded):

    sudo apt install cmake g++ libopenvdb-dev libtbb-dev libboost-iostreams-dev libboost-system-dev
    cmake -S workers/vdb -B target/vdb-worker -DCMAKE_BUILD_TYPE=Release && cmake --build target/vdb-worker
    CFA4_VDB_WORKER=$PWD/target/vdb-worker/cfa4-vdb-worker cargo test -p cfa4-vdb --release

Without the worker the real-worker tests return early and the engine works as before (`Error::Unavailable`).

## N1 acceptance status
| Criterion | State |
|---|---|
| N1-1 `vdb.inspect` | worker + host done, tested on a synthetic file; engine command not yet registered |
| N1-2 ≥ 1 GiB, cgroup memory | **open** |
| N1-3 hostile files | host side tested (60 mutated/truncated files, crash/hang/garbage/flood fakes); long fuzz run **open** |
| N1-4 round trip | grid and metadata identity on the synthetic file; bit-identity over all supported types **open** |
| N1-5 image sequence import | **open** |
| N1-6 CPU only, GPUs hidden | **open** (the worker uses no GPU) |
| N1-7 project save/restore | **open** |
| N1-8 sandbox profile | **open** (worker not yet confined) |
| N1-9 frame-time reference | **open** |
| N1-10 Qt6 panel | **open** |
