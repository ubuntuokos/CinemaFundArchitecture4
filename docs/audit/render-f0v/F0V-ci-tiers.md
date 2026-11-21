# F0V: CI-szintek definíciója (bekapcsolás nélkül)

A terv 9. pontja szerint a szintek itt csak **definiálva** vannak; workflow-fájlt ez a PR nem hoz. A bekapcsolás a fázis jóváhagyásával, a
CI-felelős döntésével történik. A repó jelenlegi kapuja (`cfa4-full-validation` címke, a `full-validation-gate` címkére van kötve) változatlan.

| Szint | Mikor | Mit futtat | Mit nem | Parancs (ma) | Bekapcsolási feltétel |
|---|---|---|---|---|---|
| **smoke** | minden PR | a harness egységtesztjei, a procedurális jelenet 10 futás, a mutációs önteszt, `render_fixtures.py list` + a manifest ellenőrzése | GPU, nagy fixture-ök, hosszú render | `cargo test -p cfa4-metrics -p cfa4-golden --release`; `python3 -m pytest python/tests/test_render_fixtures.py` | a smoke 10/10 zöld változatlan HEAD-en (AC-01-hez) |
| **nightly** | naponta, CPU | a teljes fixture-csomag generálása és tulajdonság-ellenőrzése, golden-összevetés, MQ-metrikák, A/V drift, a küszöbfájl érvényessége | GPU | `python3 python/cfa4_devtools/render_fixtures.py generate` majd `verify`, utána a harness | fixture-tár helye és mérete eldöntve (a tervben PENDING) |
| **hardware** | kézi vagy a referenciagépen | GPU-útvonal, NVENC/NVDEC, VRAM-mérés, gép-/GPU-zaj, `render_environment.py` | – | a referenciagépen | referenciagép elérhetősége |

Szabályok: `CFA4_REQUIRE_ORACLES=1` minden kihagyást hibává tesz; egy ellenőrzés, amelyet nem mértek meg, nem lehet PASS; a küszöbök módosítása csak
`CFA4_BLESS=1`-gyel, review-zva.
