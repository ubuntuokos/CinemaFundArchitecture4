# VFX-1 / VFX-4 – a kifejezésmag jelöltjeinek statikus auditja (előzetes)

**Dátum:** 2026-11-22 · **Terv:** `CFA4-VFX-001` §5 (VFX-1, VFX-4), D-2 (egy kifejezésmag, gazdája a VFX-4) · **Állapot:** `PRELIMINARY`, csak javaslat; a döntés a tulajdonosé és a gazdáé (`coordination.md` §2.10)

Ez a dokumentum **csak olvasással** készült: a donorokat nem fordítottuk le és nem futtattuk, nem írtunk beléjük. A donor-SHA-k a `CFA4-VFX-001-VFX-0A-DONOR-SNAPSHOT.md` szerintiek. Hatókör: csak Linux. A márkaszűrő miatt a donor-repók neve nincs kiírva, a crate-ek neve általános.

## 1. Mit kerestünk
A VFX-4 kifejezésmotorja (valueAtTime, wiggle, loop, property-hivatkozás; determinisztikus seed; sandbox-korlátok). A `main`-ben ilyen nincs (`G-29`, #1532). A kérdés: van-e átvehető jelölt a donorokban, vagy önálló építés kell.

## 2. Jelöltek

| Jelölt | Mi ez valójában | Licenc | Becslés |
|---|---|---|---|
| EC-donor `expr` crate (SHA `171e92b0…`) | **Valódi kifejezésmotor**: boa JavaScript-futtatókörnyezet az AE-objektummodellel (`thisComp`, `thisLayer`, `valueAtTime`, `wiggle`, `loopIn/Out`, `seedRandom`, tömbaritmetika átírással). ~5 900 sor, ebből ~2 300 teszt és ~370 fuzz | `MIT OR Apache-2.0` | átvételre alkalmas jelölt |
| Lumit-donor `lumit-eval` | **Nem kifejezésmotor**, hanem a kompozíció kiértékelő gráfja (ütemező, cache, kiértékelési terv); az előzetes leltár (#1539) hipotézise („kifejezésmotor”) tévesnek bizonyult | `GPL-3.0-only` | a VFX-4-nek nem jelölt; a render-terv F3 hatáskörébe esik (referencia) |
| Lumit-donor `lumit-script` | JavaScript-szkriptfelület (QuickJS, `rquickjs`), nem property-kifejezés; MCP- és élő szkript-kapcsolat | `GPL-3.0-only` | nem jelölt a VFX-4-hez; a szkriptelés (T16) külön kérdés |
| Gaffer-donor | Python/C++ kifejezésrendszer (`Gaffer` modul) | BSD-3-Clause | nem jelölt (a „port vagy marad” szabály: Python nem kerül a szerkesztő folyamatába) |

Javítás az előző leltárhoz: a `lumit-eval` sor „kifejezésmotor” besorolása érvénytelen; a #1539 szövegét a következő frissítésben ehhez kell igazítani.

## 3. Az EC-donor `expr` crate: amit statikusan láttunk

- **Nyelv:** JavaScript + az AE régi tömbaritmetikája; a `+ - * /` átírása segédhívásokra (`rewrite`, ~1 050 sor).
- **Motor:** boa (`boa_engine` 0.22 a donor `Cargo.lock`-jában, forrás: crates.io). Új függőség lenne a CFA4-ben.
- **Determinizmus:** a `wiggle` és a `random` réteg-index, property-azonosító és idő szerint seedelt, így szálak és platformok között azonos.
- **Korlátok a donorban:** egymásba ágyazás max. 16 szint, max. 20 000 script-futás és **5 másodperc** egy kiértékelésre, verem-határ (`native_stack`), szintaktikai mélységkorlát és fuzz-teszt. Nincs `unwrap`/`panic` (`deny` lint).
- **Függőségek:** a crate az EC-donor saját `project`, `render`, `keyframe`, `time`, `geom`, `path`, `effects` crate-jeire épül (az `ExprHost` jellegű hídon át), ezért **közvetlenül nem emelhető át**; a gazdagép-felületet a CFA4 `Tick`-alapú, L1-adat-modelljére kell illeszteni.

## 4. Szabályütközések és kockázatok az átvételnél

| Terület | Megállapítás | Következmény |
|---|---|---|
| Idő | A donor `MAX_TIME` falióra-alapú (`std::time`), a CFA4 szabálya: az idő mindig `cfa4_time::Tick`; a determinisztikus kiértékeléshez falióra-korlát nem lehet a **kimenet** része | a korlátot üzemanyag-számlálóra (utasítás- vagy ciklusszám) kell cserélni; a falióra legfeljebb védőkorlát lehet, de eredményt nem befolyásolhat |
| Sandbox | A `cfa4-sandbox` (CFA4-SECURITY-001 B1) folyamatonkénti Landlock/seccomp profil; a motor ezen belül is futhat külön workerként | a VFX-4 sandbox-profilja külön tervpont: nincs fájl-, hálózat- és óra-hozzáférés a script számára |
| Függőség | A boa új, nagy függőség; a `cargo deny` és a `docs/licensing.md` §5 miatt nyilvántartandó (a boa licencét a csomag saját `Cargo.toml`-ja szerint kell rögzíteni az átvételi PR-ban, ezt itt nem állítjuk) | VFX-0b folytatása a tényleges átvételnél |
| Veremmélység | A donor a verem helyi változójának címéből méri a mélységet (`stack_here`); `unsafe` nincs (statikus keresés: nincs találat), de a módszer platform- és fordítófüggő | determinisztikus, explicit mélységszámlálóra érdemes cserélni (V-E3 miatt) |
| Márka | A donor `prelude.js` és a crate a donor márkanevét hordozhatja a fájlfejlécekben | átvételkor semleges név, eredetfejléc a VFX-0b szabálya szerint |
| Platform | A webes (`wasm32`) ág a donor `Cargo.toml`-jában külön szakaszban van | **nem** vesszük át (csak Linux) |
| Eredet | Az AE-kompatibilitás viselkedési referencia; a csatolt `prelude.js` Adobe-kódot nem tartalmazhat | átvétel előtt soronkénti eredetnézés (`AGENTS.md` §1 tiszta szoba) |

## 5. Javaslat (a VFX-1 audit előzetes döntése, tulajdonosi jóváhagyásra)

1. **Átvétel (`PORT_CODE`)** az EC-donor `expr` crate-ből, nem önálló építés: a kód létezik, MIT/Apache-2.0, fuzzolt, determinisztikus. Az átvétel a CFA4-ben új `crates/expr` crate-ként, a `VfxGraph` L1-adatra és `Tick`-re illesztve (VFX-4 a VFX-2 után).
2. **Kötelező átalakítások:** falióra helyett üzemanyag; a veremcím-trükk kiváltása; a gazdagép-felület átírása; a sandbox-profil megtervezése; golden-teszt a F0V-harnessen (V-E3: azonos kép lineáris és véletlen eléréssel).
3. **Nem jelölt:** `lumit-eval`, `lumit-script`, a Gaffer kifejezésrendszere.
4. **Feltételek az indításhoz** (a terv szerint): a VFX-2 adatmodell és a render C0/F3 kész; külön tulajdonosi engedély (§2.10). A boa-függőség és az átvett fájlok regisztersora a tényleges PR-ban.

## 6. Nyitott
- A boa licencének és a transzitív függőségeinek (`cargo deny`) rögzítése az átvételi PR-ban.
- A `prelude.js` eredet- és márkaellenőrzése soronként.
- Futásbeli igazolás (hány kifejezés, sebesség) **nem történt**: a donor nem futtatható (olvasás = csak olvasás); a mérés a CFA4-ben átvett kódon történik majd.
- A #1539 leltár `lumit-eval` sorának javítása.
