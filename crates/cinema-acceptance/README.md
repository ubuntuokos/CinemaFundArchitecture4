# cfa4-cinema-acceptance

CineCert pre-release acceptance tests ([`docs/plans/CFA4-CINECERT-001-v2.3.md`](../../docs/plans/CFA4-CINECERT-001-v2.3.md)
ch. 8) on real CFA4 output. Test-only crate: the library is empty, everything is a dev-dependency.

```sh
cargo test --release -p cfa4-cinema-acceptance                       # oracles skipped when absent
eval "$(bash scripts/dev/setup-cinema-oracles.sh --print-env | sed 's/^/export /')"
CFA4_ACCEPTANCE_REQUIRE_ORACLES=1 cargo test --release -p cfa4-cinema-acceptance -- --nocapture
```

`tests/acceptance.rs` exports a 48-frame 2K SMPTE DCP and an IMF App 2E package with
`cfa4-export`, then checks them:

| Ch. 8 item | Check |
|---|---|
| 6 | MXF-DIGEST of every track file (stable over two reads); IMF virtual track fingerprints of CFA4's own CPL: the image resource split into two continuations or written with explicit defaults keeps the fingerprint, a shorter or shifted timeline changes it |
| 8 | verified delivery (bit-identical, manifest) and a valid BagIt bag of both packages; the corruption, interruption and path-traversal cases are in `cfa4-cinema-pkg`'s own tests |
| 10 | external oracles: `asdcp-info -i` on every DCP track file (`CFA4_ASDCPLIB_PREFIX`), Clairmeta `check -type dcp` (`CFA4_CLAIRMETA`), Photon `IMPAnalyzer` on the IMP (`CFA4_PHOTON_CP`); `CFA4_ACCEPTANCE_REQUIRE_ORACLES=1` turns a missing oracle into a failure |
| naming | ISDCF naming of the export's folder name and CPL `ContentTitleText`, reported as warnings |

Each run writes its evidence, a provenance report listing every check with tool, version and
result, to `target/tmp/cinema-acceptance/<package>.provenance.json`, ready to attach to a release
issue or PR.

The other ch. 8 items (install, router regression, J2K and colour, KDM, GUI, accessibility,
performance, long content, determinism) are tested by the projects that own those features.
