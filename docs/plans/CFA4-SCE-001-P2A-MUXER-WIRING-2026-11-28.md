# CFA4-SCE-001 P2a, 2. szelet: a profil bekötése a TS-muxerbe (CBR, periódusok, SDT, időbélyeg-mód)

**Dátum:** 2026-11-28
**Státusz:** végleges, a tulajdonos jóváhagyta (2026-11-28); a fejlesztés a #1857 és a #1927 beolvasztása után indul
**Szülőterv:** [CFA4-SCE-001 v1.5](CFA4-SCE-001-STREAM-COPY-EXPORT-2026-10-27.md), 12.6/4–5 és 12.7 4–6. lépés · **Előző szelet:** [P2a.1](CFA4-SCE-001-P2A-PROFILE-SDT-2026-11-27.md) (`TsParams`, SDT, `MpegTsProfile`) · **Audit:** [SCE-001-P0-AUDIT-2026-11-08](../audit/SCE-001-P0-AUDIT-2026-11-08.md), 4.2/3, 6, 7, 8, 10

## 1. Előfeltétel

A szelet a `crates/mpegts/src/mux.rs`-t módosítja, amelynek csomagütemezését a nyitott #1857 (T-STD, a #277 projektje) írja át. Nyitott PR ágára nem építhetek (coordination.md §2.1), ezért a fejlesztés a **#1857 beolvasztása után** indul, friss `main`-ből. A #1857 a PCR-t már legfeljebb 40 ms-onként küldi, így az audit 4.2/7 pontja (PCR-intervallum) nagyrészt megoldódik; ezt a szelet teszttel rögzíti, nem programozza újra. Az első lépés a `main` friss állapotának felmérése, és ha a #1857 másképp alakul, ezt a tervet a tulajdonossal újra jóváhagyatom.

A P2a.1 (#1927) beolvasztása szintén előfeltétel: erre épül a `TsParams` és az `MpegTsProfile`.

## 2. Hatókör

1. **`TsParams` a muxerben.** Új konstruktor: `Muxer::with_params(out, format, streams, mux_rate, &TsParams)`. A mostani `Muxer::new` változatlan marad, és `TsParams::default()`-ot ad át, így minden meglévő hívó (`mpeg_out.rs`, `smart_cut_mpeg.rs`, `boundary_reencode_mpeg.rs`, `copy/compatible.rs`) kimenete **bájtra azonos** marad. A PAT/PMT a `TsParams` azonosítóit, PID-jeit és `tables_version` mezőjét használja; a `pcr_pid` opcionálisan külön PID.
2. **SDT.** A `TsParams::sdt == true` esetén az SDT-csomagok a PAT/PMT-vel együtt, az `sdt_period_ms` szerint mennek ki, saját continuity-számlálóval a 0x0011 PID-en.
3. **PSI- és PCR-periódus.** A mostani rögzített 100 ms (PSI) és 30 ms (PCR) a profil `psi_period_ms` és `pcr_period_ms` értéke lesz. Az alapérték a mostani viselkedés, a beállított értéknél a muxer nem ritkábban küld.
4. **CBR null-csomagokkal.** `TsMuxMode::Cbr { mux_rate }` esetén a muxer a rendszerórát a kért rátán futtatja, és ahol nincs adat, null-csomagot (PID 0x1FFF) ír, így a kimenet tényleges rátája a kértet adja. Ha a tartalom a kért rátán nem fér el (a T-STD vagy a 64 MiB-os sorkorlát miatt), a muxer érthető hibát ad, nem csendben túllépi.
5. **Időbélyeg-mód.** `Preserve` esetén a muxer nem adja hozzá a rögzített `DELAY` eltolást (audit 4.2/8); a PTS/DTS a forráséval egyezik, a PCR ehhez igazodik. `Shift` marad az alapérték.
6. **PMT-leírók.** A HEVC (registration `HEVC`), AC-3 (már van) és a nyelvi leíró (ISO 639) az ES-info hurokba kerül, ahol a profil vagy a stream kéri (audit 4.2/3).
7. **Hosszú GOP.** Az audit 4.2/10 mérését elvégzem (nagy bitrátájú, hosszú GOP-os másolás, egy elakadó stream mellett), és ha a 64 MiB-os korlát az átlapolást sérti, a korlátot a profilhoz kötöm. A mérés eredménye a PR leírásába kerül.
8. **`MpegTsProfile::to_params`.** A `NotYetSupported` válasz a CBR-re és a `Preserve`-re megszűnik; a profil minden mezője eljut a muxerig.

## 3. Nem része ennek a szeletnek

- Engine-parancs és Qt-felület (P7, a Qt-kapu után).
- Több program (12.5: az R1-ben nem cél), PSIP (ATSC).
- A meglévő exportok átállítása a profilra: a hívók a `Muxer::new`-t használják tovább, amíg a P7 felület nem kéri a profilt.
- A T-STD modell (#1857).

## 4. Tesztek és elfogadás

- **Bájtazonosság:** minden meglévő `mux_tests`, a `mpeg2_tests` és a `copy/compatible_tests` változatlanul zöld; egy új teszt `Muxer::new` és `Muxer::with_params(.., &TsParams::default())` kimenetét bájtra összeveti több streames bemeneten.
- **SDT a muxerben:** a kimenetben az SDT periódusa, CRC-je és continuity-je helyes; az ffprobe a szolgáltatás nevét és szolgáltatóját visszaolvassa a teljes, muxolt fájlból.
- **Periódusok:** a PAT/PMT, az SDT és a PCR a kért periódusnál nem ritkábban fordul elő (a kimenet saját demuxerével és az ffprobe-bal mérve, a PCR-intervallum a 40 ms-os alapértéken és egy 100 ms-os beállításon is).
- **CBR:** a kimenet mérete és az első és utolsó PCR közti idő a kért rátát ±0,5%-on belül adja; null-csomagok a 0x1FFF PID-en, a payload bájtra változatlan; a nem elférő tartalom hibát ad, nem sérült fájlt. M2TS módban a 192 bájtos csomagolással is.
- **Preserve:** a kimenet PTS/DTS értékei a bemenetéivel egyeznek, a PCR ≤ DTS marad.
- **Orákulum (CI, FFmpeg 9.1 / ffprobe):** H.264 + AAC TS, M2TS és CBR kimenet elfogadva; hiba esetén a teszt kihagyja magát, ha nincs ffprobe.
- **Robusztusság:** véletlen és szélsőértékes profil, csonkolt bemenet mellett nincs pánik.
- **Kapuk:** `cargo xtask ci`, `cargo test -p cfa4-mpegts -p cfa4-export --release`.

Kilépési feltétel: a fenti tesztek zöldek, a meglévő kimenetek bájtra változatlanok. Ezzel a P2a a szülőterv `MPEGTS_MUX_E2E` kapujáig a Qt-felület nélkül kész; a felület és az utóellenőrzés összefoglalója a P7-ben jön.

## 5. PR és átadás

Egy PR (`SCE-001 P2a.2`), friss `main`-ből. CodeRabbit: kell (`cfa4-mpegts`). Kész állapotban „Ready for review”, független Claude-ügynökös review, majd „Gazda: …” issue.

## 6. Kérdés a tulajdonosnak

Nincs nyitott döntés. Egyetlen megjegyzés: ha a #1857 a beolvasztás előtt a `mux.rs` PCR- vagy PSI-logikáját más alakra hozza, a 2–3. pont megvalósítása ehhez igazodik, és ezt a PR leírásában jelzem; a hatókör változása esetén új jóváhagyást kérek.
