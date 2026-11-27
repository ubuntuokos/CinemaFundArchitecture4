# CFA4-SCE-001 P2a, 1. szelet: MpegTsProfile és SDT

**Dátum:** 2026-11-27
**Státusz:** tervezet, tulajdonosi jóváhagyásra vár
**Szülőterv:** [CFA4-SCE-001 v1.5](CFA4-SCE-001-STREAM-COPY-EXPORT-2026-10-27.md), 12.6/2 (profilmodell) és 12.6/4 (SDT) · **Audit:** [SCE-001-P0-AUDIT-2026-11-08](../audit/SCE-001-P0-AUDIT-2026-11-08.md), 4.2/4–5
**Ütemezési kivétel:** a tulajdonos 2026-11-27-én a Qt6-kapu előtt (qt-parity 77/80 ✅) engedélyezte a P2a profil- és SDT-részének indítását, a P2-höz hasonlóan (2026-11-08).

## 1. Miért most, és miért csak ennyi

A P2a hátralévő része a szülőterv szerint: `MpegTsProfile`, SDT, CBR null-csomagokkal, PCR-periódus, eredeti időbélyeg mód. A nyitott #1857 (T-STD, a #277 projektje) a `crates/mpegts/src/mux.rs` csomagütemezését írja át. A CBR és a PCR-ütemezés ugyanabba a kódba nyúlna, ezért az a #1857 beolvasztása utánra marad (2. szelet).

Ez a szelet csak olyan új fájlokat hoz, amelyek a #1857-tel nem ütköznek. A `mux.rs`-hez nem nyúl.

## 2. Hatókör

### 2.1 `cfa4-mpegts` (L0, csak `cfa4-bitstream` és `thiserror`)

Új modul: `crates/mpegts/src/si.rs`.

- `TsParams`: a muxer jövőbeli bemenete, egyszerű Rust struktúra, serde nélkül (L0):
  `transport_stream_id`, `program_number` (= `service_id`), `original_network_id`, `pmt_pid`, `first_es_pid`, opcionális `pcr_pid`, `service_name`, `service_provider`, `sdt: bool`, `tables_version` (0–31).
  `TsParams::default()` pontosan a mai muxer értékeit adja (TSID 1, program 1, PMT 0x1000, ES 0x100-tól, SDT nincs), így a 2. szeletben az alapértelmezett kimenet bájtra azonos marad.
- `TsParams::validate()`: a PID-ek 0x0010–0x1FFE között, nem ütköznek egymással és a fenntartott PID-ekkel (0x0000 PAT, 0x0011 SDT, 0x1FFF null); a `program_number` nem 0; a nevek legfeljebb 255 bájtosak. Hibánál `Result`, pánik nélkül.
- `sdt_section(&TsParams) -> Result<Vec<u8>, …>`: DVB SDT actual (table_id 0x42, ETSI EN 300 468 5.2.3), egy szolgáltatással, service_descriptor (0x48) szolgáltatástípus 0x01 (digitális TV) vagy 0x02 (csak hang), CRC-32-vel. A szöveg ISO/IEC 8859-1-nél szélesebb karakterkészlet esetén UTF-8 jelölőbájttal (0x15) megy.
- `sdt_packets(&TsParams, cc) -> Vec<[u8; 188]>`: a szekció 188 bájtos csomagokra bontva a 0x0011 PID-en.

### 2.2 `cfa4-export` (serde)

Új modul: `crates/export/src/mpegts_profile.rs`, a `lib.rs`-ben egyetlen új `mod` sorral.

- `MpegTsProfile` (serde, `version: u32` mezővel, ismeretlen mezőket tűr):
  `output: Ts188 | M2ts192`, `mux: Vbr | Cbr { mux_rate }`, `system: SystemA | SystemB`, a 2.1 azonosítói és nevei, `pcr_period_ms`, `psi_period_ms`, `sdt_period_ms`, `timestamps: Shift | Preserve`, `tables_version`.
- Hiányzó mező: determinisztikus CFA4-alapérték. `SystemB` (ATSC) esetén nincs SDT (a PSIP nem cél).
- `MpegTsProfile::validate()`: tartományok (PCR 20–100 ms, PSI 25–500 ms, SDT 25–2000 ms, CBR muxráta 1–200 Mbit/s); hibás PID vagy muxráta elutasítva, érthető üzenettel.
- `MpegTsProfile::to_params() -> TsParams`; az export jelentése a ténylegesen használt értékeket mutatja (`effective()`).
- A CBR, a periódusok és a `Preserve` mód ebben a szeletben csak a modellben él: a muxer még nem kapja meg őket. A validáció ezt nem hallgatja el: amíg a 2. szelet nincs kész, a `Cbr` és a `Preserve` `NotYetSupported` választ ad.

## 3. Nem része ennek a szeletnek

- A `mux.rs` módosítása: a `TsParams` és az SDT bekötése, a CBR null-csomagokkal, a PCR- és PSI-periódus, a `Preserve` mód. Ez a 2. szelet, a #1857 beolvasztása után, ugyanebben a tervben.
- Engine-parancs, Qt-felület (P7).
- Több program (a 12.5 alapján az R1-ben nem cél).

## 4. Tesztek és elfogadás

- **Egység (mpegts):** az SDT szekció mezői és CRC-je; egy kézzel összerakott, EN 300 468 szerinti referencia-szekcióval bájtra egyezik; a hosszú nevek több csomagra bomlanak; minden érvénytelen `TsParams` hibát ad.
- **Egység (export):** a profil JSON-ja oda-vissza; a régi (verzió nélküli vagy kevesebb mezős) JSON alapértékekkel töltődik; minden tartománysértés elutasítva; a `to_params()` a profil azonosítóit adja.
- **Fuzz/robusztusság:** véletlen `TsParams` és profil-JSON mellett nincs pánik (proptest, ha a crate már használja; különben determinisztikus táblás teszt).
- **Orákulum:** az ffprobe (CI, FFmpeg 9.1) a `sdt_packets` kimenetét egy minimális TS-be téve (PAT + SDT + null) a szolgáltatás nevét és szolgáltatóját visszaolvassa. Ha a CI-ben nincs ffprobe, a teszt kihagyja magát.
- **Kapuk:** `cargo xtask ci`, `cargo test -p cfa4-mpegts -p cfa4-export --release`.

Kilépési feltétel: a fenti tesztek zöldek, a `main` mostani TS-kimenete változatlan (a meglévő `mux_tests` és export-tesztek változatlanul zöldek).

## 5. PR és átadás

Egy PR (`SCE-001 P2a.1`), CodeRabbit: kell (a `cfa4-mpegts` konténer-crate). Kész állapotban „Ready for review”, független Claude-ügynökös review, majd „Gazda: …” issue.

## 6. Kérdés a tulajdonosnak

Nincs nyitott döntés: a fenti alapértékek a szülőterv 12.6/2 pontját követik. Ha a 2. szelet idejére a #1857 még nyitva van, ezt jelzem, és nem építek az ágára.
