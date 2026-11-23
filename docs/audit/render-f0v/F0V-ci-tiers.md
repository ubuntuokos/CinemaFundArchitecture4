# F0V: CI-szintek definíciója és bekapcsolása

**Állapot (2026-11-23, István jóváhagyta a védőintézkedések alkalmazását):** a smoke és a nightly szint be van kapcsolva a
[`.github/workflows/cfa4-render-validation.yml`](../../../.github/workflows/cfa4-render-validation.yml) fájlban; a hardware szint nincs (referenciagép kell).
A repó jelenlegi kapuja (`cfa4-full-validation` címke, `full-validation-gate`) változatlan.

**Eltérés a fenti táblától, indoklással:** a repó szabálya, hogy fordító CI-feladat nem fut minden PR-on (a `ci-linux.yml` címkéhez köti), ezért a PR-os
**smoke fordítás nélküli**: a fixture-generátor tesztjei és a `list`. A harness cargo-tesztjei (10 futás, mutációs önteszt, A/V drift) a **nightly**-ban
futnak, és címkével bármelyik PR-on is lefuttathatók (`workflow_dispatch`). A nightly első futása még nem történt meg: a fixture-generálás az
`ffmpeg` kódolóitól függ, és `CFA4_REQUIRE_ORACLES=1` mellett minden kihagyás hiba, tehát egy hiányzó kódoló pirosat ad, nem csendes PASS-t.

| Szint | Mikor | Mit futtat | Mit nem | Parancs (ma) | Bekapcsolási feltétel |
|---|---|---|---|---|---|
| **smoke** | minden PR | a harness egységtesztjei, a procedurális jelenet 10 futás, a mutációs önteszt, `render_fixtures.py list` + a manifest ellenőrzése | GPU, nagy fixture-ök, hosszú render | `cargo test -p cfa4-metrics -p cfa4-golden --release`; `python3 -m pytest python/tests/test_render_fixtures.py` | a smoke 10/10 zöld változatlan HEAD-en (AC-01-hez) |
| **nightly** | naponta, CPU | a teljes fixture-csomag generálása és tulajdonság-ellenőrzése, golden-összevetés, MQ-metrikák, A/V drift, a küszöbfájl érvényessége | GPU | `python3 python/cfa4_devtools/render_fixtures.py generate` majd `verify`, utána a harness | fixture-tár helye és mérete eldöntve (a tervben PENDING) |
| **hardware** | kézi vagy a referenciagépen | GPU-útvonal, NVENC/NVDEC, VRAM-mérés, gép-/GPU-zaj, `render_environment.py` | – | a referenciagépen | referenciagép elérhetősége |

Szabályok: `CFA4_REQUIRE_ORACLES=1` minden kihagyást hibává tesz; egy ellenőrzés, amelyet nem mértek meg, nem lehet PASS; a küszöbök módosítása csak
`CFA4_BLESS=1`-gyel, review-zva.
