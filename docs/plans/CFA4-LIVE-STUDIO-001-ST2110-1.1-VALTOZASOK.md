# CFA4-LIVE-STUDIO-001-ST2110 – változáslista (1.0 → 1.1, jóváhagyva 2026-11-24)

**Állapot:** a tulajdonos 2026-11-24-én jóváhagyta; a T1b indítható, a T4b a T4 után indul (`docs/coordination.md` §2.10). A T2–T7 fázisok előfeltételei és sorrendje nem változnak.

**Készült:** 2026-11-24, a tulajdonos kérésére („vedd bele a tervbe a projektnek hasznosakat”).
**Összevetés alapja:** `main` `19ad4e24` (#1798); a `crates/st2110` T0/T1 állapota (README) is ellenőrizve.
**Átnézett szövegek:** ST 2110-10:2017/2022, -20:2017/2022, -21:2017/2022, -22:2019/2022, -30:2017/2025,
-31:2018/2022, -40:2018/2023, -41:2024/2026-06, -43:2021; ST 2112-10/-20 (2018, 2020), ST 2113:2018,
ST 2114:2017, ST 2115:2019/2020, ST 2116:2019, ST 2117-1:2020/2023, ST 2117-10:2024. A szövegek szerzői
jog alatt állnak, a tárolóba nem kerülnek. A -10, -20, -21, -22 szövegei a projekt `uploads/` mappájában
vannak (a nyitóoldali fájllista csak a többit mutatja).
**Megjegyzés a forrásokról:** az ST 2112-10/-20 az OBID (hangvízjeles tartalomazonosító), **nem** az
ST 2110-10/-20; a kettő számozása csak hasonlít.

**Súlyosság:** **blokkoló**: e nélkül a terv hibás vagy ütközik egy döntéssel; **fontos**: jelentős
pontosítás vagy hiány; **javaslat**: minőségi javítás.

**Kötött döntések (nem változtak):** D8 saját Rust kód (GStreamer csak tesztben); D9 `linuxptp`;
D10 csak VC-2, **nincs JPEG XS** egyetlen CFA4-csomagban sem; D11 CFA4-parancsok előbb, NMOS később;
a gép 2,5 GbE-s.

| ID | Súlyosság | Érintett fejezet | Változás | Bizonyíték |
|---|---|---|---|---|
| V-01 | fontos | fejléc | Verzió 1.1 JAVASLAT; változásnapló-hivatkozás; a fő terv hivatkozása v1.1 → v1.4; a T0/T1 már `main`-ben van, az érintő tételek T1b-be kerültek | `docs/plans/CFA4-LIVE-STUDIO-001.md:3`; `crates/st2110/README.md`; `docs/coordination.md` §2.10 |
| V-02 | fontos | fejléc, II., X. | Forráskészlet: 17 ST 2110 dokumentum + az ST 2112–2117 csomag; az ST 2112-10/-20 más szabvány (OBID) | az ST 2112-10:2020 címe „Open Binding of Content Identifiers (OBID)”; ST 2112-20: „OBID-TLC” |
| V-03 | fontos | II. SDP-pont, II.1 | `TSMODE`: a programkimenet `SAMP` is lehet, ha a képkockaóra a PTP-hez zárt (szintetikus tartalom, N × T<sub>FRAME</sub> ±T<sub>FRAME</sub>); `PRES` a továbbvitt időbélyegre; hiányzó paraméter = `NEW`; a vevői felülbírálat a menedzsment-API-n (*should*). Az 1.0 szerint mindig `NEW` | ST 2110-10:2022 7.6.3, 7.9, 8.7, C melléklet |
| V-04 | fontos | II. SDP-pont, II.1 | a „traceable” PTP-jelzés három feltétele: PTP-időskála, nagymester `timeTraceable`, `clockAccuracy` ≤ 250 ns; különben a nagymester azonosítója és a tartomány kerül az SDP-be. A kód ma csak a nagymestert nevezi meg és fogadja a traceable alakot, a feltételeket nem ellenőrzi | ST 2110-10:2022 8.2; `crates/st2110/src/sdp.rs:286-313`; README „`sender_clocks`” |
| V-05 | fontos | II.1, III.4 | `slaveOnly` PTP: a CFA4 nem lehet vezető; minta-konfiguráció és kényszerítési mód; AES67-hangcseréhez a PTP-üzenetsebesség az AES67 Media Profile határain belül (AES-R16-2016) | ST 2110-10:2022 7.2 és Bibliography |
| V-06 | fontos | II.1, IV. (T3, T4) | Link Offset Delay (vevő: állandó, dokumentált, állítható, a puffer legalább ekkora) és Transmission Delay (küldő: mért, `TSDELAY`) | ST 2110-10:2022 7.8, 8.7 |
| V-07 | fontos | II.1, IV. (T1b, T3) | hálózati szabályok az SDP-ellenőrzőben: tiltott multicast-tartományok (224.0.0.0/24, 224.0.1.0/24), nincs IP-darabolás, nincs RTP-multiplexelés, RTCP és fejléckiterjesztés tűrése, 2022-7 pár szabályai (azonos fejléc és adat; nem lehet azonos forrás- és célcím), RTCP SR az `mediaclk:sender` forrásnál | ST 2110-10:2022 6.2–6.5, 8.4, 8.5, B melléklet |
| V-08 | fontos | II. kép-pont, II.1, IV. (T3, T4, T4b) | P3 nem jelezhető → átalakítás küldés előtt; `UNSPECIFIED` csak kifejezett választásra, bejövőnél kézi felülbírálat; fill + key (`KEY`/`ALPHA`, `SSN=ST2110-20:2022`) új T4b fázisként a C10 élő grafikához; `ST2115LOGS3` bejövő átalakítás; hiányzó paraméterek alapértékei (`TCS`=SDR, `RANGE`=NARROW, `PAR`=1:1) | ST 2110-20:2022 6.2.6, 7.3–7.6; ST 2113:2018 5, 6; ST 2115:2020 6, 7; a kód `video.rs` már ismeri a `ST2115LOGS3`-at |
| V-09 | fontos | II. forgalom-pont, II.1, VII. | az N típusú vevő csak N küldőtől köteles fogadni (NL-től *should*), a W vevő N/NL/W-től; a szoftveres W küldő ezért nem minden kereskedelmi vevővel kapcsolható; CMAX csak 900 000 csomag/s alatt; hiányzó `MAXUDP` = 1460 (a -21 számításában 1500) | ST 2110-21:2022 7.1.4, 7.2.3–7.2.5 |
| V-10 | fontos | II.1, IX. | -22: `b=AS` az IP-fejlécekkel, kbit/s, felfelé kerekítve; `SSN` és `CMAX` választható; **a `jxsv` rtpmap elutasítása** (D10) tesztelt tiltás (T1b); a JPEG XS-re vonatkozó rész kimarad | ST 2110-22:2022 5.3, 7.2–7.5; D10 |
| V-11 | fontos | II. hang-pont, II.1, IV. (T2) | -30:2025 szintek: küldő 2. táblázat, vevő 3. táblázat; a vevő a sáv minden kombinációját; a B vevő az 1 ms-os folyamot is; a küldő felsorolja a szintjeit; csatornarend szabályai (hiányzó vagy eltérő darabszám → Undefined, U01–U64); AES67-2023; SIP nem kell | ST 2110-30:2025 6.2.2, 7, 2–3. táblázat |
| V-12 | fontos | II. AES3-pont, IV. (T5) | -31:2022: vevői szintek A–DX, `ptime` 1/0,12/0,08 ms (44,1 kHz: 1,09/0,14/0,09), `M=0`, `CC=0`, `X` bit, `AES3` csatornacsoport-jel, időbélyeg az X/Z előjelnél; az ST 2116 Serial ADM átlátszóan átmegy | ST 2110-31:2022 5.3–5.5, 6.1–6.2, 7; ST 2116:2019 |
| V-13 | fontos | II. ANC-pont, IV. (T5) | -40:2023: `SSN`/`TM` szabály, hiányzó `TM` = CTM, mindkét modell fogadása, LLTM `T_D = 8/(képsebesség × teljes sorszám)` (1080p50: ≈ 142 µs), keep-alive, FID tilos, beágyazott hang és EDH nem küldhető, `VPID_Code` csak pontos SDI-hely javaslatánál; `Line_Number=0x7FF` a feliratokhoz | ST 2110-40:2023 5.2.1–5.5, 6.4, 6.5, 7 |
| V-14 | javaslat | II. TTML-pont | -43: `timeBase="media"`, `#rtp-relative-media-time`, 0 hosszú keep-alive csomag; a -43 a -10:2017-re hivatkozik, mi a -10:2022-t követjük | ST 2110-43:2021 4.2, 4.3, 5.1 |
| V-15 | javaslat | II. táblázat, II.1 | -41:2026-06: csak az A melléklet két hibája javítva, az `SSN` marad `ST2110-41:2024`; kódhatás nincs | ST 2110-41:2026-06 Foreword |
| V-16 | fontos | V., VII. | pontosabb sávszélesség-számítás (BPM ≈ 93,2% hatásfok): 1080p50 10 bit ≈ 2,22 Gbit/s a vonalon = a 2,5 GbE ≈ 89%-a; a 1080p59,94 10 bit ≈ 2,7 (nem 2,6); méréslétra 1080p25 10 bit → 1080p50 8 bit → 1080p50 10 bit; a 2022-7 egy kártyán a forgalmat duplázza (≈ 4,4 Gbit/s), ezért kisebb formátummal vagy `veth`-en mérendő | ST 2110-10:2022 A melléklet (18 + 4 + 20 bájt keretenkénti többlet); ST 2110-20:2022 6.3.3, A. táblázat (7 × 180 bájt); számítás: 1920 × 1080 × 20 bit × 50 = 2,0736 Gbit/s, csomagonként 1260 bájt |
| V-17 | javaslat | X. | ST 2112-10/-20 (OBID): **nem veszi át** – nem ST 2110, hangvízjel-beágyazót/dekódert kívánna | az ST 2112 szövegei (Scope, 3. fejezet) |
| V-18 | javaslat | X. | ST 2113 (P3 primerek és fehérpontok) részben; ST 2114 (C4 ID) nincs teendő, `cfa4-cinema-pkg` már tartalmazza; ST 2115 részben; ST 2116 nincs teendő | `crates/cinema-pkg/src/base58.rs`; ST 2113:2018 5.1, 6.2–6.4 |
| V-19 | javaslat | X. | ST 2117-1/-10 (VC-6): **nem veszi át** – nem a D10 szerinti VC-2, RTP-leképezés nincs a -22 szövegeiben, szabadalmi vizsgálat nélkül nem kerül be | ST 2117-1:2023 Intellectual Property; `docs/patent-review.md` (VC-6 sor nincs) |
| V-20 | javaslat | IV. | új fázisok: **T1b** (a kész T0/T1 kiegészítése, jóváhagyás kell) és **T4b** (fill + key); a T0–T7 előfeltételei és a D8–D12 változatlanok | `docs/coordination.md` §2.10 |

## Amit megvizsgáltam és nem kellett változtatni

- A -10:2022 átvett pontjai (UDP-méret 1460/8960, `a=mediaclk:direct=0`, PTP-alapú `ts-refclk`,
  `localmac`, dinamikus payload 96–127, big-endian, 2022-7 `a=group:DUP`) az 1.0-ban helyesek.
- A -20 GPM/BPM (7 × 180 bájt), a kötelező és alapértelmezett `fmtp` paraméterek, az `SSN` szabály
  (2017, kivéve ALPHA/LOGS3), a -21 W típus paraméterei (V<sub>RX FULL</sub> ≥ 720, C<sub>MAX</sub> ≥ 16,
  β = 1,10, lineáris olvasás) és a -22 `TP` jelzés az 1.0-ban helyesek; a `crates/st2110` README ezekkel egyezik.
- A 1080p50/59,94 sávszélesség-becslések rendje (1080p59,94 4:2:2 10 bit nem fér a 2,5 GbE-re) helyes; csak a
  vonali érték pontosodott (V-16).

## 1.2 (2026-11-24): állapot a `main`-hez igazítva

Nincs új szabványi tétel (V-01–V-20 változatlan). Az 1.2 csak az állapotot és a hivatkozásokat frissíti: a T0 és a T1 kész, a C10 a
`main`-ben van, a T1b/T4b indítását és a D7 pontosítását (D7a) a tulajdonos 2026-11-24-én jóváhagyta (fő terv 1.5, XI. fejezet).
