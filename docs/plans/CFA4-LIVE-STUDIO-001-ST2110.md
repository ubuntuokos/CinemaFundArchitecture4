# CFA4-LIVE-STUDIO-001 kiegészítés – SMPTE ST 2110 az élő stúdióban

**Verzió:** 1.2 (a tulajdonos 2026-11-24-én jóváhagyta; az 1.1 (#1806) tartalmát is, és az állapotot a `main`-hez igazítja)
**Dátum:** 2026-11-24 (1.0: 2026-11-11)
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Kiegészíti:** [CFA4-LIVE-STUDIO-001](CFA4-LIVE-STUDIO-001.md) (v1.5; a döntések a fő terv XI. fejezetében), D4 (csak nyílt protokollok)
**Változásnapló:** [CFA4-LIVE-STUDIO-001-ST2110-1.1-VALTOZASOK.md](CFA4-LIVE-STUDIO-001-ST2110-1.1-VALTOZASOK.md)
(V-01–V-20; a szakaszok végén zárójelben a tételazonosító).
**Állapot:** a tulajdonos (`ubuntuokos`) 2026-11-11-én megerősítette, hogy az ST 2110 nyílt
protokollként a D4 szerint bekerül az élő stúdió tervébe, fázisbeosztással (ez a dokumentum). A
D8–D12 döntéseket és az 1.0-t (fázisok T0–T7) a tulajdonos 2026-11-11-én jóváhagyta. Az 1.1-et a
tulajdonos kérésére („vedd bele a tervbe a projektnek hasznosakat”) a legfrissebb kiadások újra
átvizsgálása után készítettem; a D8–D12 döntések változatlanok. A T0 és a T1 (`crates/st2110`) már a
`main`-ben van: az őket érintő tételek a **T1b** pontba kerültek (IV. fejezet), és
(`docs/coordination.md` §2.10) a tulajdonos jóváhagyásáig nem indulnak el.
**Források:** a tulajdonos által 2026-11-11-én és 2026-11-24-én átadott SMPTE-szövegek: 17 ST 2110
dokumentum (-10, -20, -21, -22, -30, -31, -40, -41, -43; régi és új kiadás; II. fejezet), továbbá az
ST 2112-10/-20, ST 2113, ST 2114, ST 2115, ST 2116, ST 2117-1 és ST 2117-10 (X. fejezet). A
szabványszövegek szerzői jog alatt állnak: **nem kerülnek a tárolóba**, csak a kiadásuk és a
fejezetszámaik (`AGENTS.md` §2: a megvalósított kiadást a crate README-je rögzíti).

---

## I. Cél és hatókör

Az SMPTE ST 2110 a műsorszóró stúdiók IP-alapú, tömörítetlen (vagy könnyen tömörített) kép-,
hang- és adatátvitele RTP/UDP-n, közös PTP-órához igazítva. Nyílt szabvány, zárt SDK nem kell hozzá
(szemben az NDI-vel), ezért a D4 szerint befér a CFA4-be.

Mit ad az élő stúdiónak:

1. **Bemenet:** ST 2110 kamera-, hang- és adatfolyamok forrásként a jelenetekben (S2 rögzítő
   bemenetei és az SRT/RIST/WHIP hálózati bemenetek mellé).
2. **Kimenet:** a program (és később az előnézet, az egyes források) ST 2110 folyamként a stúdió
   hálózatára, képkocka-pontosan, PTP-hez igazítva (az S6 streamelése mellé).
3. **Adat:** feliratok (CEA-608/708 ANC-ban, TTML), időkód és egyéb metaadat a képpel szinkronban,
   a meglévő `cfa4-captions` crate-re építve.

Nem cél (ebben a kiegészítésben): SDI-hardver kezelése, ST 2022-6 (SDI IP-be csomagolva), hardveres
FPGA-alapú küldés, zárt menedzsmentrendszerek.

## II. A szabványok és kiadásaik

Mindegyiknél a **legújabb kiadást** valósítjuk meg; a régebbit csak ott, ahol a vevőnek a régi
jelzést is el kell fogadnia (a táblázat utolsó oszlopa).

| Rész | Tárgy | Átadott kiadások | Megvalósítandó | Változás a régihez képest (ami minket érint) |
|---|---|---|---|---|
| **-10** | rendszer, időzítési modell, RTP, SDP, PTP | 2017, **2022** | 2022 | Media Clock és RTP Clock pontosabb definíciója; a vevőnek tűrnie kell az RTP fejléc-kiterjesztést; a közös órára nem kötött folyamok (`ts-refclk:localmac`, `mediaclk:sender`); `TSMODE` (SAMP/NEW/PRES) és `TSDELAY`; a „traceable” PTP-jelzés javított szintaxisa (a vevő a 2017-es alakot is fogadja el), a jelzés három feltétele; Link Offset Delay (vevő) és Transmission Delay (küldő) fogalma; `slaveOnly` kényszerítés; IPv6 *should* |
| **-20** | tömörítetlen aktív kép (RFC 4175 alapon) | 2017, **2022** | 2022 | kulcsjel (`sampling=KEY`, `colorimetry=ALPHA`); `TCS=ST2115LOGS3`; a -21 egyes előírásai ide kerültek; az `SSN` ST2110-20:2017 marad, kivéve ALPHA/LOGS3 esetén |
| **-21** | küldő forgalomformálás (N, NL, W), vevőtípusok | 2017, **2022** | 2022 | előírások átkerültek a -20-ba és a -22-be; a tömörített képre (-22) csak a hálózati kompatibilitási modell vonatkozik |
| **-22** | állandó bitsebességű tömörített kép | 2019, **2022** | 2022 | a forgalomformálás pontosítva; Type N is megengedett; `SSN` jelzés |
| **-30** | PCM hang (AES67 alapon) | 2017, **2025** | 2025 | a megfelelőségi szintek (A, AX, B, BX, C, CX) a 7. fejezetben kifejezetten, küldőre (2. táblázat) és vevőre (3. táblázat) külön; AES67-2023 és RFC 8866 az alap; a 44,1 és 96 kHz *should*, nem *shall* |
| **-31** | AES3 átlátszó átvitel (AM824) | 2018, **2022** | 2022 | PICS-alapú pontosítások; AES3/AES10 különbségek (melléklet); IANA-regisztráció; vevői szintek A, AX, B, BX, C, CX, D, DX (3. táblázat); `AES3` csatornacsoport-jel |
| **-40** | SMPTE ST 291-1 ANC-adat (RFC 8331 alapon) | 2018, **2023** | 2023 | két időzítési modell: kompatibilis (CTM, 1 ms) és alacsony késleltetésű (LLTM); `TM` paraméter; `SSN` ST2110-40:2023 csak `TM` esetén |
| **-41** | gyors metaadat-keretrendszer (Data Item Package) | 2024, **2026-06** | 2026-06 | csak az A melléklet két hibájának javítása; az `SSN` értéke továbbra is ST2110-41:2024 |
| **-43** | TTML feliratok (RFC 8759, IMSC1.2) | **2021** | 2021 | egy kiadás |

### Amit a szövegekből a tervhez rögzítünk

- **Szállítás (-10):** RTP (RFC 3550) UDP fölött; IPv4 multicast IGMPv3-mal kötelező, IPv6 ajánlott;
  dinamikus payload type 96–127; a többbájtos mezők big-endianok. UDP-méret: **szabványos 1460 bájt**,
  kiterjesztett 8960 bájt csak `MAXUDP` jelzéssel. Redundancia: ST 2022-7 kettőzött folyamok, SDP-ben
  `a=group:DUP` (RFC 7104), azonos forrás- és célcím egyszerre nem lehet.
- **Idő (-10):** közös referenciaóra a PTP (IEEE 1588-2008, ST 2059-2 profil); az RTP Clock a Media
  Clockból, eltolás nélkül (`a=mediaclk:direct=0`); a képre 90 kHz, a hangra a mintavételi frekvencia.
  A képkocka-időbélyeg a SMPTE Epoch óta eltelt N × T<sub>FRAME</sub>-hez igazodik. Ha nincs PTP:
  `a=ts-refclk:localmac=…` és `a=mediaclk:sender`.
- **SDP (-10):** minden folyamhoz egy SDP (RFC 4566), amelyet az eszköz **menedzsment-API-n** tesz
  elérhetővé (küldő), illetve fogad és alkalmaz (vevő). `ts-refclk`, `mediaclk` kötelező; ajánlott a
  `source-filter` (SSM). `TSMODE`/`TSDELAY` (V-03): `SAMP`, ha a küldő időbélyege a mintavételi
  pillanatot vagy a szintetikus előírást adja (lásd II.1), `NEW`, ha a küldő a kimeneten új időbélyeget
  képez (ez az alapértelmezés, ha a paraméter hiányzik), `PRES`, ha a bemenet időbélyegét adjuk tovább,
  de a bemenet nem jelezte a `SAMP`-ot; `TSDELAY` mikroszekundumban. A „traceable” PTP-jelzést csak
  akkor adjuk, ha mindhárom feltétel teljesül (II.1, V-04).
- **Kép (-20):** csomagonként legfeljebb 3 Sample Row Data fejléc; a hossz a pgroup többszöröse;
  az utolsó csomagon M=1. Csomagolási mód: **GPM** (általános) vagy **BPM** (7 × 180 bájt, kiterjesztett
  UDP nélkül); a vevőnek mindkettőt fogadnia kell. Kötelező fmtp: `sampling`, `depth`, `width`,
  `height`, `exactframerate`, `colorimetry`, `PM`, `SSN`; alapértelmezettel: `interlace`,
  `segmented`, `TCS`, `RANGE`, `MAXUDP`, `PAR`. Mintavétel: YCbCr/CLYCbCr/ICtCp 4:4:4, 4:2:2, 4:2:0,
  RGB, XYZ, KEY; mélység 8/10/12/16/16f; színtér BT601/709/2020/2100, ST2065-1/-3, XYZ, ALPHA,
  UNSPECIFIED (kézi egyeztetéssel). Hiányzó `TCS` = SDR, hiányzó `RANGE` = NARROW, hiányzó `PAR` = 1:1,
  hiányzó `interlace` = progresszív; `segmented` `interlace` nélkül tilos. A **P3 színtér** (ST 2113)
  nem szerepel a megengedett `colorimetry` értékek közt (V-08).
- **Forgalomformálás (-21):** a küldő három típusa közül **a szoftveres küldőnek a Type W való**
  (a szabvány 7.1.4 3. megjegyzése éppen a szoftveres forrásokra szánja: V<sub>RX FULL</sub> legalább
  720 csomag, C<sub>MAX</sub> legalább 16, β = 1,10, lineáris olvasási ütemezés). Az SDP-ben
  `TP=2110TPW`, eltérő eltolásnál `TROFF`. Vevőtípusok: N és W szinkron (azonos PTP), A aszinkron
  (bármit fogad); **a mi vevőnk A típusú** lesz, puffereléssel. A W küldő jelzése a -21 szerint nem
  teszi vevhetővé minden vevőnek: az **N típusú vevő csak N típusú küldőtől** (a NL-től *should*)
  köteles fogadni, a W vevő N, NL és W küldőtől (V-09, VII. fejezet).
- **Tömörített kép (-22):** állandó bájtszám és csomagszám képkockánként, 90 kHz, `b=AS:` bitsebesség,
  `TP` kötelező; a konkrét kodek payload formátuma külön RFC (JPEG XS: RFC 9134; VC-2: SMPTE ST 2042 /
  RFC 8450).
- **Hang (-30):** AES67 szerint; 48 kHz kötelező; minden eszköz legalább **A szint** (48 kHz, 1 ms
  csomagidő, 1–8 csatorna). Küldői szintek (2. táblázat): A 48 kHz/1 ms/1–8, AX 96 kHz/1 ms/1–4,
  B 48 kHz/125 µs/1–8, BX 96 kHz/125 µs/1–8, C 48 kHz/125 µs/9–64, CX 96 kHz/125 µs/9–32. A küldő
  *legalább egy* csatornaszámot támogat a sávból, a **vevő a sáv minden kombinációját**; a B szintű
  vevő ezért az 1 ms-os A folyamot is fogadja (3. táblázat). Csatornarend `channel-order=SMPTE2110.(…)`
  (M, DM, ST, LtRt, 51, 71, 222, SGRP, Unn; U01–U64); ha hiányzik vagy nem egyezik a darabszámmal,
  a csatornák *Undefined*. A 16 SDI-csatornát több A folyamra kell osztani. Csak a szabványos UDP-méret.
  Az AES67 SIP-jét nem kell támogatni (V-11).
- **AES3 (-31):** `AM824` alcímke, 32 bites alkeretek (B, F, PCUV, 24 bit adat), páros csatornaszám,
  `ptime` táblázat szerint (1 ms; 0,12 és 0,08 ms 48 és 96 kHz-en, 1,09/0,14/0,09 ms 44,1 kHz-en;
  a 0,125 ms „0,12”-nek jelzett). A fejléc: `M=0`, `CC=0`; az `X` bit RFC 8285 kiterjesztést jelez, a
  vevő tűri. Az időbélyeg az első alkeret X vagy Z előjelének pillanata. A vevői szintek A–DX (3.
  táblázat); az A szint 1 ms, legfeljebb 6 alkeret csomagonként (3 AES3 jel). Így a nem-PCM tartalom
  (pl. ST 337 szerint csomagolt adat, ST 2116 Serial ADM) érintetlenül átmegy; mi csak továbbítjuk,
  nem dekódoljuk (V-12).
- **ANC (-40):** RFC 8331; legalább egy csomag képkockánként (üres is, `ANC_Count=0`, M=1);
  `exactframerate` kötelező; `TM=CTM` (1 ms) vagy `TM=LLTM`; FID-csoportosítás tilos. `SSN=ST2110-40:2018`,
  kivéve ha `TM` is jelezve van: akkor `ST2110-40:2023` (a vevő a `ST2110-40:2021`-et ezzel egyenlőnek
  veszi); hiányzó `TM` = CTM; minden vevő mindkét modellt fogadja. LLTM: `T_D = 8 / (képsebesség ×
  teljes sorszám)`, pl. 1080p50-nél 8 / (50 × 1125) ≈ 142 µs; CTM: `T_D` = 1 ms. A beágyazott hangot
  és az EDH-csomagot nem szabad így küldeni. Pontos SDI-helyet (`Line_Number`, `Horizontal_Offset`)
  csak `VPID_Code` jelzéssel szabad javasolni; a feliratfolyam `Line_Number=0x7FF` („nincs javaslat”)
  értékkel egyszerűbb. Az ANC-folyam képfolyam nélkül is létezhet; az időbélyeg köti a képhez (V-13).
- **Gyors metaadat (-41):** Data Item Package (22 bites típus, K bit, 9 bites hossz, 32 bites
  egységek), legalább 500 ms-onként egy csomag, M=0, `SSN=ST2110-41:2024`, ajánlott `DIT` lista.
- **TTML (-43):** TTML2, IMSC1.2 profil, RFC 8759, 90 kHz; a dokumentum időpontjai az RTP-időbélyeghez
  relatívak (`timeBase="media"`, `#rtp-relative-media-time`); a dokumentumok közt 0 hosszú, `M=1`
  „keep-alive” csomag küldhető. A -43 a -10:2017-re hivatkozik; mi a -10:2022 szerint építünk (V-14).

### II.1. Új és pontosított előírások, amelyeket az 1.0 nem rögzített (v1.1)

A 2026-11-24-i átvizsgálás a legújabb kiadásokat vetette össze az 1.0 szövegével. Hibát nem talált az
1.0 fázisbeosztásában; a lenti tételek pontosítások és hiányok. Mindegyik mellett: melyik fázisban
valósul meg.

| # | Előírás (szabvány, fejezet) | Mit jelent a CFA4-nek | Hol |
|---|---|---|---|
| V-03 | -10:2022 7.6.3, 7.9, 8.7 | A programkimenet szintetikus (kompozitált) tartalom. Ha a képkockaórát a PTP-hez zártuk, az időbélyeg N × T<sub>FRAME</sub> ±T<sub>FRAME</sub> pontosságú, ezért a küldő `TSMODE=SAMP`-ot adhat; zárás nélkül `NEW`. Az 1.0 mindig `NEW`-t írt. A vevő a bejövő `TSMODE`/`TSDELAY` értékét a menedzsment-API-n felülbírálhatja (*should*) | T1b, T4 |
| V-04 | -10:2022 8.2 | A `ptp=IEEE1588-2008:traceable` jelzés csak akkor adható, ha (1) a PTP-időskála használatban van, (2) a nagymester `timeTraceable` mezője igen, (3) a `clockAccuracy` 250 ns vagy jobb. Különben a nagymester azonosítója és a tartomány kerül az SDP-be | T1b |
| V-05 | -10:2022 7.2 | A CFA4 nem lehet PTP-vezető: a `ptp4l` `slaveOnly 1` beállítással fut (minta `ptp4l.conf` a dokumentációban), és az eszközön van mód ennek kényszerítésére. AES67-eszközökkel való hangcseréhez a PTP-üzenetsebességek az AES67 Media Profile határain belül maradjanak (AES-R16-2016) | T1b, T2 |
| V-06 | -10:2022 7.8 | A vevő **Link Offset Delay** (D<sub>LO</sub>) állandó, dokumentált, a menedzsment-API-n állítható érték; a csomagpuffernek legalább D<sub>LO</sub> idő csomagjait tartania kell. A küldő **Transmission Delay** (D<sub>TX</sub>) értékét a T4 elemzője méri, és `TSDELAY`-ként jelzi | T3, T4 |
| V-07 | -10:2022 6.2–6.5, 8.4, 8.5 | Hálózati ellenőrzések az SDP-ellenőrzőben: a cél nem lehet a 224.0.0.0/24 és a 224.0.1.0/24 tartományban; nincs IP-darabolás a küldő kimenetén; RTP-munkamenet-multiplexelés tilos; a vevő tűri az RTCP-t és az RTP-fejléckiterjesztést; a 2022-7 pár nem használhat egyszerre azonos forrás- és célcímet, és a két folyam fejléce és adata azonos. A `mediaclk:sender` forrásnál a vevő az RTCP Sender Reportból becsüli az óraviszonyt, ha van | T1b (ellenőrző), T3 |
| V-08 | -20:2022 7.3–7.6; ST 2113; ST 2115 | **P3** nem jelezhető (a `colorimetry` listában nincs): P3 tartalmat küldés előtt BT.2020-ra (vagy BT.709-re) alakítunk, a vágást jelezzük; `UNSPECIFIED`-et csak a felhasználó kifejezett választására adunk. Bejövő `UNSPECIFIED` színtér/átvitel kézi felülbírálatot kér a vevőn. **Fill + key:** a key külön -20 folyam (`sampling=KEY`, `colorimetry=ALPHA`, nincs `TCS`, `SSN=ST2110-20:2022`) – a C10 élő grafika (#803) külső keverőbe küldéséhez. `ST2115LOGS3` bejövő forrás bemeneti átalakítást kap (ST 2115 Camera Log S3), a hozzá tartozó Color VANC az ANC-ban jön (ST 2048-1 6.3) | T3, T4b |
| V-09 | -21:2022 7.1.4, 7.2 | A szoftveres küldő W típusú, de a kereskedelmi **N típusú vevők csak N küldőtől** kötelesek fogadni. Ezt a kockázatot rögzítjük (VII.), és a T4 mérése W-képes vevővel (vagy a saját vevőnkkel) történik. A CMAX-előírás 900 000 csomag/s alatt érvényes (1080p50 10 bit ≈ 206 000 csomag/s); az SDP-ben a `MAXUDP` hiánya 1460-at jelent, a -21 számításában 1500 | T4 |
| V-10 | -22:2022 5.3, 7.2–7.5 | A `b=AS:` az átlagos bitsebesség kbit/s-ban, felfelé kerekítve, **az IP-fejlécekkel együtt**, Ethernet nélkül; a képsebesség `a=framerate` vagy `exactframerate`; `SSN` (`ST2110-22:2019` vagy `:2022`) és `CMAX` választható; állandó bájt- és csomagszám képkockánként. **D10-ellenőrzés:** a vevő a `jxsv` (JPEG XS) `rtpmap`-ot érthető hibaüzenettel elutasítja, és ez tesztelt | T6 (a tiltás T1b-ben) |
| V-11 | -30:2025 6–7 | A CFA4 A szintet követel és jelez; a B szintet a T2 második lépése adja (vevő: 1 ms és 125 µs, 1–8 csatorna). A küldő státusza felsorolja a támogatott szinteket (*should*). A 44,1 és 96 kHz (AX, BX, CX) nem része a T2-nek | T2 |
| V-12 | -31:2022 5–7 | Lásd az AES3-pontot fent: `AM824/<ütem>/<n>` páros `n`-nel, `ptime` a táblázatból, `AES3` csatornacsoport-jel, A szintű vevő; időbélyeg az X/Z előjelnél | T5 |
| V-13 | -40:2023 5–7 | Lásd az ANC-pontot fent (`SSN`/`TM` szabály, keep-alive, FID tilos, LLTM `T_D`, hiányzó `TM` = CTM) | T5 |
| V-14 | -43:2021 4–5 | Lásd a TTML-pontot fent; a TTML-író `timeBase="media"` kimenetet ad | T5 |
| V-15 | -41:2026-06 | A kiadás csak az A melléklet két hibáját javítja; az `SSN` marad `ST2110-41:2024`; kódváltozást nem okoz, a megvalósítás a 2026-06 szövegből készül | T5 |

## III. Felépítés

```
            stúdióhálózat (10/25 GbE, PTP)                         cfa4-studio-host
 ST 2110 forrás ──RTP──► vevő (A típus) ─► Rust libobs-forrás ─► jelenet ─► program
                                                                              │
 ST 2110 vevő  ◄──RTP── küldő (W típus, ütemezett) ◄── nyers kép/hang ◄──────┘
                         ▲
              ptp4l + phc2sys (linuxptp, ST 2059-2 profil) → CLOCK_TAI
              SDP: menedzsment-API (motor: live.studio.st2110.*), később NMOS IS-04/IS-05
```

1. **Saját Rust megvalósítás** egy új crate-ben (munkanév: `crates/st2110`, `cfa4-st2110`): RTP,
   SDP-olvasás és -írás, a -20 pgroup-csomagolás és -bontás, a -21 modellek (küldőoldali ellenőrzés
   és saját mérőeszköz), a -30/-31/-40/-41/-43 payloadok. A nyilvános SMPTE- és IETF-szövegekből,
   clean-room (`AGENTS.md` §2); a crate README-je rögzíti a kiadásokat (II. fejezet). Nincs `unsafe`;
   a socket- és időbeállítások (`SO_TXTIME`, multicast-tagság, `CLOCK_TAI`) a `rustix`/`socket2` biztonságos
   API-ján át, ami nincs bennük, az az ADR 0001 szerint a `crates/platform`-ba kerül.
2. **Hol fut:** a `cfa4-studio-host` folyamatban (D2), a libobs mellett: a vevő Rust libobs-forrásként
   adja a képkockákat és a hangot, a küldő a libobs nyers kép- és hangkimenetét kapja (mint az S0
   frame ringje). A libobs későbbi Rust-cseréje (S12+) ezt nem érinti, mert a crate nem függ a
   libobs-tól. A réteg (L-szint) a `cargo xtask layers` szerint, a megvalósításkor dől el.
3. **Hálózati bemenet nem megbízható adat** (never-crash, `AGENTS.md` §0): minden fejléc és hossz
   ellenőrzött, a pufferek korlátosak, a hibás csomag eldobva és számlálva; sérült bemenetes
   fuzz-tesztek, mint a kodekeknél.
4. **PTP:** a rendszer `linuxptp` csomagja (Ubuntu-tároló, GPL-2) futtatja a `ptp4l`-t az ST 2059-2
   profillal és a `phc2sys`-t; a CFA4 csak olvassa az órát (`CLOCK_TAI`) és az állapotot (D9). A
   menedzsment-API `ts-refclk`-ja a PTP nagymester azonosítóját és tartományát jelzi, a „traceable”
   alakot csak a II.1 V-04 feltételei mellett. A `ptp4l` `slaveOnly 1` módban fut (V-05).
   A PTP-olvasó (`pmc`-lekérdezés, `PtpReport`) a `cfa4-io-core` crate-ben él (READY-V1), a `cfa4-obs` csak
   újra-exportálja; a T1 erre épít, nem készít másodikat.
5. **Küldési ütemezés:** a Type W tűrése a szoftveres küldőnek is elérhető. Az első lépés a kernel
   `SO_TXTIME` + ETF qdisc időzített küldése (nincs kernelmegkerülés); ha a referenciagépen mérve nem
   elég, az AF_XDP a következő lépés (külön döntés). Saját -21 elemző méri a kimenetet
   (C<sub>INST</sub>, V<sub>RX</sub>) a hálózati kártya időbélyegeiből.
6. **Menedzsment:** a -10 előírja, hogy az SDP menedzsment-API-n legyen elérhető. Először a motor
   parancsai (`live.studio.st2110.sender.*`, `.receiver.*`, SDP be/ki), vagyis a vezérlőcsatorna, az
   MCP és a Qt6 felület ugyanazt látja („minden parancs”). Később az AMWA NMOS IS-04 (felderítés) és
   IS-05 (kapcsolatkezelés) nyílt specifikációi (D11).
7. **Tesztelés:** a hurokinterfészen (multicast-hurokkal) a CI-ban is futó oda-vissza tesztek; a
   GStreamer `rtpvrawpay`/`rtpvrawdepay` (RFC 4175) külső összevetésként, a meglévő FFmpeg-oracle
   mintájára (csak teszt, nem termékút). Valódi stúdióeszközzel és PTP-vel a referenciagépen.

## IV. Fázisok

A betűjel **T** (transzport); minden fázis végén működő, tesztelt, beolvasztható állapot, zöld
kapukkal. A fő terv fázisaihoz kötve (VI. fejezet a fő tervben):

| Fázis | Tartalom | Előfeltétel | Kész, ha |
|---|---|---|---|
| **T0** – Alap | `cfa4-st2110` crate: RTP-fejléc, kiterjesztett sorszám, SDP olvasás/írás a -10/-20/-21/-30 paramétereivel, -20 pgroup-csomagolás (4:2:2 8/10 bit, 4:4:4 RGB 8/10 bit) GPM és BPM, bontás; fuzz-tesztek | S0 (#820) | a -10 B mellékletének és a -20 7.7 példájának SDP-je oda-vissza azonos; egy képkocka csomagolva–bontva bitazonos; a GStreamer `rtpvrawdepay` a mi csomagjainkból ugyanazt a képet adja |
| **T1** – Idő | PTP-állapot és idő olvasása (`CLOCK_TAI`, `ptp4l` állapot), SMPTE Epoch szerinti képkocka-időbélyeg, `ts-refclk` (ptp és localmac), `mediaclk:direct=0` / `sender`; állapot a host státuszában | T0 | PTP-vel és anélkül is helyes SDP és időbélyeg; a 59,94 Hz-es 1501/1502 lépés tesztelve |
| **T1b** – Pontosítások (v1.1, V-03–V-07, V-10) | a kész T0/T1 kiegészítése: `PtpStatus` `traceable` mező és a három feltétel (V-04); `TSMODE=SAMP` döntés a képkockaóra zárása szerint (V-03); SDP-ellenőrző a hálózati szabályokkal (tiltott multicast-tartományok, 2022-7 pár, `MAXUDP`/UDP-méret; V-07); `jxsv` elutasítása (V-10, D10); `slaveOnly` minta-konfiguráció és `docs/` leírás (V-05) | T1, **a tulajdonos jóváhagyása** (§2.10) | a meglévő tesztek zöldek; új tesztek: a három „traceable” feltétel minden kombinációja, tiltott cím elutasítása, `jxsv` elutasítása, SDP oda-vissza |
| **T2** – Hang (-30) | AES67 küldő és vevő, A szint (48 kHz, 1 ms, 1–8 csatorna), utána B (125 µs; a B szintű vevő az 1 ms-os folyamot is fogadja); csatornarend (`Unn` tartalékkal); a támogatott szintek a státuszban; a libobs hangkeverőbe és -ből; a PTP-üzenetsebesség ellenőrzése az AES67 Media Profile szerint (V-05, V-11) | T1, S3 (hangkeverő) | egy külső AES67-forrás hangja a keverőben; a program hangja egy AES67-vevőben; késleltetés mérve |
| **T3** – Képbemenet (-20, -21 vevő) | A típusú vevő (GPM és BPM, 4:2:2 10 bit, 1080p50/59,94, 720p, 2160p ha a kártya bírja; V. fejezet), 2022-7 redundáns pár, ST 2110 forrás a jelenetben; állítható Link Offset Delay (V-06); RTCP-alapú óra-becslés `mediaclk:sender` forrásnál (V-07); kézi `colorimetry`/`TCS` felülbírálat `UNSPECIFIED` forrásnál és `ST2115LOGS3` bemeneti átalakítás (V-08) | T1, S2 (rögzítő bemenetek) | egy 1080p50 4:2:2 10 bites folyam (a 2,5 GbE-n ez fér el, V. fejezet) elejtett képkocka nélkül egy órán át a referenciagépen; csomagvesztésnél kerethiba nélkül (eldobott képkocka, számlálva) |
| **T4** – Programkimenet (-20, -21 küldő) | W típusú küldő `SO_TXTIME` ütemezéssel, `TP=2110TPW`, saját -21 elemző (D<sub>TX</sub> mérése és `TSDELAY`; V-06), 2022-7 kimenet, SDP a menedzsment-API-n; P3 → BT.2020/709 átalakítás küldés előtt (V-08) | T3, S4 (stúdió mód) | a saját elemző és egy külső vevő szerint W-megfelelő egy órán át; a program képe bitazonos a stúdióéval |
| **T4b** – Fill + key (v1.1, V-08) | a C10 élő grafika kimenete két -20 folyamként: fill (`sampling` a program szerint) és key (`sampling=KEY`, `colorimetry=ALPHA`, `SSN=ST2110-20:2022`); bejövő key-folyam fogadása | T4, C10 (#803) | egy külső vevő a fill + key párból helyesen keveri a grafikát; a key pgroup-táblázata (8/10/12/16 bit) bitazonos |
| **T5** – Adatfolyamok (-40, -43, -41, -31) | ANC: CEA-608/708 (ST 334) és időkód a `cfa4-captions`-ből, CTM, később LLTM; TTML/IMSC1.2 (-43) a `cfa4-captions` TTML-íróból; -41 keretrendszer (a regisztrált típusok közül, amire igény van); -31 AM824 átlátszó továbbítás | T4 | az élő feliratok egy vevőben a képpel szinkronban; az AM824 tartalom bitazonosan átmegy |
| **T6** – Tömörített kép (-22) | VC-2 (D10), CBR, 90 kHz, `b=AS`, N vagy W forgalom | T4, D10 | egy -22 folyam be és ki a referenciagépen, bitsebesség és késleltetés mérve |
| **T7** – Kezelés és felület | NMOS IS-04/IS-05 (D11); Élő munkaterület: folyamok, SDP, PTP-állapot, hibaszámlálók (Qt6, a D7 szerint a Qt6 befejezése után) | T4, D7 | egy NMOS-vezérlő felderíti és bekapcsolja a CFA4 küldőit és vevőit |

**Megvalósítási állapot (2026-11-24):** T0 kész (#1002, 2026-11-13), T1 kész (#1128, 2026-11-15); T1b jóváhagyva (2026-11-24), indítható;
T4b elvileg jóváhagyva; a többi fázis nincs elkezdve. A C10 előfeltétele (`crates/live`) a `main`-ben van, így a T4b-nek csak a T4
hiányzik. **Döntés (fő terv XI.2):** a T1b indítható (CodeRabbit kell, `crates/st2110`); a T4b a T4 után indul.
**A T7 Qt6-oldala** a D7 szerint vár; a motor- és parancsoldalra a D7a (fő terv XI.1) vonatkozik.

**Ütemezés (v1.1):** a T1b a T2 előtt, a T4b a T4 és a C10 után jön; mindkettő a tulajdonos jóváhagyását
kéri (§2.10). **Ütemezés (1.0):** a T0 és a T1 nem érinti a Qt6 fájljait és a libobs-ot, ezért az S0-hoz hasonlóan
korán indulhat: ez a D7-től való eltérés, a tulajdonos 2026-11-11-én jóváhagyta (D12). A többi fázis a fő terv
előfeltételként megnevezett fázisai után.

## V. Mérés és hardver

- **Sávszélesség:** 1080p59,94 4:2:2 10 bit ≈ 2,49 Gbit/s hasznos adat (1920 × 1080 × 20 bit ×
  59,94), ehhez **10 GbE** kell; 2160p59,94 ≈ 9,95 Gbit/s, ehhez **25 GbE**. 1 GbE-n csak a hang, az
  adat és a tömörített kép (-22) próbálható, a tömörítetlen kép legfeljebb kis felbontásban.
- **Hálózati kártya:** a tulajdonos 2026-11-11-én jelezte, hogy a referenciagépben **2,5 GbE-s kártya**
  van, 10 GbE-s nincs (rögzítve a [`reference-machine.md`](../development/reference-machine.md)-ben). A
  kártya típusát és PTP-képességét a T1-et indító munkamenet a gépről olvassa ki (`lspci`, `ethtool -i`,
  `ethtool -T`) és rögzíti; több 2,5 GbE-s vezérlő tud PTP hardveres időbélyeget. Mit ad a 2,5 GbE
  (a csomag-, IP- és Ethernet-többlet kb. 6%):
  - **elfér:** 1080p50 4:2:2 10 bit (≈ 2,07 Gbit/s hasznos, ≈ 2,2 a vonalon), 1080p59,94 4:2:2 8 bit
    (≈ 1,99, ≈ 2,1), minden 1080p25/29,97 és 720p változat, a hang (-30/-31), az adat (-40/-41/-43)
    és a tömörített kép (-22);
  - **nem fér el:** 1080p59,94 4:2:2 10 bit (≈ 2,49 hasznos, ≈ 2,7 a vonalon) és minden 2160p
    tömörítetlenül; ezek a hurokinterfészen tesztelhetők, valódi hálózaton -22-vel (tömörítve);
  - ezért a T3 és T4 „kész, ha” mérése a referenciagépen **1080p50 4:2:2 10 biten** fut; a
    1080p59,94 10 bites és a 2160p tömörítetlen mérés egy 10/25 GbE-s kártyára vár (a tulajdonos
    döntése, nem feltétele egyik fázis indulásának sem).
- **Pontosabb sávszélesség-számítás (v1.1, V-16).** A -10 A melléklete szerint minden Ethernet-keret
  18 bájt fejlécet és FCS-t, 20 bájt előtagot és csomagközt visz (VLAN-címkével +4); a csomagonként
  RTP 12 + kiterjesztett sorszám és SRD-fejlécek (2 + 6 × 2) + IP 20 + UDP 8 bájt jön hozzá. BPM-nél
  (1260 bájt mintaadat csomagonként) ez ≈ 93,2% hatásfok, vagyis a 4:2:2 10 bites 1080p50 a vonalon
  ≈ 2,22–2,23 Gbit/s, a 2,5 Gbit/s-os kapcsolat **≈ 89%-a**; a hely a PTP-nek, a hangnak, az ANC-nak
  és a W küldő löketeinek kb. 11%. Ezért a mérés létrán halad: 1080p25 10 bit (≈ 1,1 Gbit/s) →
  1080p50 8 bit (≈ 1,8) → 1080p50 10 bit (≈ 2,2); a 10 bites 1080p50 a W-megfelelőség mérésekor a
  legszorosabb eset.
- **2022-7 egy kártyán (v1.1, V-16).** A redundáns pár két fizikai hálózatot kíván. Egyetlen 2,5 GbE-s
  kártyán a két folyam *ugyanazt a kábelt* terhelné, ezért a duplázott forgalom ≈ 4,4 Gbit/s lenne:
  a T3/T4 2022-7 „kész, ha” mérése a referenciagépen kisebb formátummal (720p50 8 bit, ≈ 0,8 Gbit/s
  folyamonként) vagy a hurokinterfészen / `veth`-párokon fut; a teljes méretű 2022-7 mérés a második
  kártyára vár (a tulajdonos döntése, mint a 10/25 GbE).
- **NUMA:** a kártya NUMA-csomópontjához rögzített szálak (a referenciagép két csomópontos; a mérési
  jegyzőkönyv rögzíti a magokat, `docs/performance.md`).

## VI. Döntések

| # | Kérdés | Döntés / javaslat |
|---|---|---|
| D4+ | ST 2110 mint nyílt protokoll | **Eldöntve (2026-11-11):** az ST 2110 a D4 nyílt protokolljai közé tartozik, a terv része ezzel a kiegészítéssel |
| D8 | Megvalósítás | **Eldöntve (2026-11-11): saját Rust crate** (`cfa4-st2110`) a nyilvános SMPTE/IETF-szövegekből; a GStreamer és más megvalósítás csak tesztbeli összevetés. Indok: a crate független a libobs-tól, így az S12+ Rust-csere után is marad |
| D9 | PTP | **Eldöntve (2026-11-11):** a rendszer `linuxptp`-je (`ptp4l` ST 2059-2 profillal, `phc2sys`), a CFA4 csak olvas; saját PTP-szolga nincs. Más lehetőség: saját Rust PTP-szolga (sok munka, a hardveres időbélyeg kezelése miatt kockázatos) |
| D10 | Tömörített kép (-22) kodekje | **Eldöntve (2026-11-11): csak VC-2** (SMPTE ST 2042, jogdíjmentes; RTP: RFC 8450). **JPEG XS-t a CFA4 semmilyen csomagban nem szállít**, a `cfa4-restricted-extras`-ban sem: a JPEG XS szabadalmi poolja (Vectis IP; Fraunhofer IIS, intoPIX) a kodeket tartalmazó termék terjesztőjét terheli, és az extra csomagot is a CFA4 terjeszti. A VC-2 szabadalmi átnézése a kiadás előtt a `patent-review.md`-be kerül |
| D11 | Menedzsment | **Eldöntve (2026-11-11):** először a CFA4 parancsai (SDP be/ki a vezérlőcsatornán és az MCP-n), a T7-ben az AMWA NMOS IS-04/IS-05 (nyílt specifikáció) |
| D12 | Ütemezés | **Eldöntve (2026-11-11):** a T0–T1 a D7-től eltérve, Qt6 nélkül indulhat az S0 beolvasztása után; a T2-től a fő terv fázisai szerint |

## VII. Kockázatok

- **Szoftveres időzítés:** egy általános Linux-rendszermag mellett a W típus tűrése is szoros lehet
  nagy felbontásnál; ezért az elemző az első naptól mér, és az AF_XDP csak mérés után jön.
- **Hardverfüggés:** a teljes érték 10/25 GbE-s, PTP-képes kártyát és PTP-nagymestert kíván; a
  referenciagépen 2,5 GbE van, ezért a T3–T4 valódi hálózaton 1080p50 10 bitig mérhető (V. fejezet).
- **Két óra:** a libobs a saját órájához igazít; a PTP-hez kötött be- és kimenet közt a libobs
  képkocka-ütemét a PTP-hez kell zárni, különben a küldő `mediaclk:sender` módban marad. Ezt a T1–T4
  méri; ha a libobs nem zárható, ez az S12+ csere (saját kompozitálás) egyik oka.
- **Vevőtípus-összeférhetetlenség (v1.1, V-09):** a szoftveres W küldő nem felel meg a -21 N típusának;
  az N típusú kereskedelmi vevők csak N küldőtől kötelesek fogadni, ezért egy ilyen vevő a CFA4 program-
  kimenetét elutasíthatja. A W vevők (és a saját A típusú vevőnk) fogadják. A T4 mérése ezért W-képes
  vevővel történik, és a felület jelzi, melyik típust állítja a küldő; ha a gyakorlatban elterjedt N-vevők
  elutasítják, az AF_XDP/hardveres pacing (külön döntés) a következő lépés.
- **Szabadalom:** a -22-ben elterjedt JPEG XS nincs a CFA4-ben (D10), ezért JPEG XS-t küldő vagy váró eszközzel a CFA4 tömörítve nem, csak tömörítetlenül (-20) kapcsolható össze.
- **Sávszélesség-tartalék (v1.1, V-16):** a 10 bites 1080p50 a 2,5 GbE ≈ 89%-át viszi; a PTP-, hang- és
  ANC-forgalom, valamint más gépek ugyanazon a kapcsolón a határra vihetik. A mérés létrán halad (V. fejezet).
- **Szerzői jog:** a szabványszövegek nem kerülnek a tárolóba, a tesztekbe sem kerül belőlük szöveg;
  a példák (SDP) saját, a szabvány szerint felépített adatok.

## VIII. Kapcsolódás a fő tervhez

A fő terv (v1.1) I. 8. pontja és D4 sora erre a kiegészítésre hivatkozik; a IV. fejezet 2. (bemenetek)
és 6. (kimenetek) sorába az ST 2110 bekerül. A fázisbeosztás a fő terv fázisai közé illeszkedik
(T2 az S3 után, T3 az S2 után, T4 az S4 után), azokat nem változtatja meg.

## IX. Mi maradt ki az ST 2110-22 tartalmából (v1.1, V-10)

- **JPEG XS** (RFC 9134 `jxsv`): a D10 szerint a CFA4 semmilyen csomagjában nincs. A -22 többi része
  (állandó bájtszám, `TP`, `b=AS`) a VC-2-re (D10) érvényes; a JPEG XS-re vonatkozó példák és
  paraméterek **kimaradnak**, legfeljebb tiltásként (a vevő elutasítja, T1b) szerepelnek.
- **-22 küldő forgalomformálása N/NL típussal:** a szoftveres küldő W (V-09); az N/NL jelzést a -22
  megengedi, de szoftveresen nem teljesíthető.

## X. Más szabványok az átadott csomagból (v1.1)

| Szabvány | Tárgya | Döntés | Indok |
|---|---|---|---|
| ST 2112-10:2018/2020, ST 2112-20:2018/2020 | OBID: tartalom- és terjesztési csatorna-azonosítók (Ad-ID, EIDR) hangvízjelben | **nem veszi át** | nincs köze az ST 2110-10/-20-hoz (a számozás csak hasonlít); hangvízjel-beágyazót és -dekódert kívánna, szabadalmi vizsgálat nélkül |
| ST 2113:2018 | P3 színterek (P3D65, P3D60, P3DCI) | **részben** | az elsődlegesek (0,680/0,320; 0,265/0,690; 0,150/0,060) és a fehérpontok a színkezelés táblájába kerülhetnek; az ST 2110-20 nem jelzi, ezért küldés előtt átalakítás (V-08) |
| ST 2114:2017 | C4 ID (SHA-512, Base58) | **nincs teendő** | a `cfa4-cinema-pkg` már tartalmazza (`base58.rs`); az élő stúdió felvételeinek azonosításához később felhasználható |
| ST 2115:2019/2020 | Free Scale Gamut/Log (Camera Log S3, Color VANC) | **részben** | a `TCS=ST2115LOGS3` jelzés (-20:2022) és bemeneti átalakítás a T3-ban; a Color VANC az ANC (T5) része (V-08) |
| ST 2116:2019 | Serial ADM metaadat AES3-ban (ST 337 szerint) | **nincs teendő** | a -31 átlátszó továbbítása érintetlenül átviszi; elemzőt nem írunk; a T5 teszt-vektora lehet (V-12) |
| ST 2117-1:2020/2023, ST 2117-10:2024 | VC-6 elemi bitfolyam és MXF-leképezés | **nem veszi át** | nem a D10 szerinti VC-2; a -22 szövegei nem tartalmaznak VC-6 RTP-leképezést; MXF-hez nem kell az élő stúdióban; felvétele előtt szabadalmi vizsgálat (`patent-review.md`) |

