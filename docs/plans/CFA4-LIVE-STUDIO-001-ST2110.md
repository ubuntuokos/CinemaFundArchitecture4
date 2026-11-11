# CFA4-LIVE-STUDIO-001 kiegészítés – SMPTE ST 2110 az élő stúdióban

**Verzió:** 0.1 (tervezet)
**Dátum:** 2026-11-11
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Kiegészíti:** [CFA4-LIVE-STUDIO-001](CFA4-LIVE-STUDIO-001.md) (v1.1), D4 (csak nyílt protokollok)
**Állapot:** a tulajdonos (`ubuntuokos`) 2026-11-11-én megerősítette, hogy az ST 2110 nyílt
protokollként a D4 szerint bekerül az élő stúdió tervébe, fázisbeosztással (ez a dokumentum). A
D8–D12 döntéseket a tulajdonos 2026-11-11-én eldöntötte; a fázisbeosztás e PR beolvasztásával jóváhagyott.
**Források:** a tulajdonos által 2026-11-11-én átadott SMPTE ST 2110 szabványszövegek (17 dokumentum,
II. fejezet). A szabványszövegek szerzői jog alatt állnak: **nem kerülnek a tárolóba**, csak a
kiadásuk és a fejezetszámaik (`AGENTS.md` §2: a megvalósított kiadást a crate README-je rögzíti).

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
| **-10** | rendszer, időzítési modell, RTP, SDP, PTP | 2017, **2022** | 2022 | Media Clock és RTP Clock pontosabb definíciója; a vevőnek tűrnie kell az RTP fejléc-kiterjesztést; a közös órára nem kötött folyamok (`ts-refclk:localmac`, `mediaclk:sender`); `TSMODE` (SAMP/NEW/PRES) és `TSDELAY`; a „traceable” PTP-jelzés javított szintaxisa (a vevő a 2017-es alakot is fogadja el); IPv6 *should* |
| **-20** | tömörítetlen aktív kép (RFC 4175 alapon) | 2017, **2022** | 2022 | kulcsjel (`sampling=KEY`, `colorimetry=ALPHA`); `TCS=ST2115LOGS3`; a -21 egyes előírásai ide kerültek; az `SSN` ST2110-20:2017 marad, kivéve ALPHA/LOGS3 esetén |
| **-21** | küldő forgalomformálás (N, NL, W), vevőtípusok | 2017, **2022** | 2022 | előírások átkerültek a -20-ba és a -22-be; a tömörített képre (-22) csak a hálózati kompatibilitási modell vonatkozik |
| **-22** | állandó bitsebességű tömörített kép | 2019, **2022** | 2022 | a forgalomformálás pontosítva; Type N is megengedett; `SSN` jelzés |
| **-30** | PCM hang (AES67 alapon) | 2017, **2025** | 2025 | a megfelelőségi szintek (A, AX, B, BX, C, CX) a 7. fejezetben kifejezetten, küldőre és vevőre külön; hivatkozások frissítve; SDP az RFC 8866 szerint |
| **-31** | AES3 átlátszó átvitel (AM824) | 2018, **2022** | 2022 | PICS-alapú pontosítások; AES3/AES10 különbségek (melléklet); IANA-regisztráció |
| **-40** | SMPTE ST 291-1 ANC-adat (RFC 8331 alapon) | 2018, **2023** | 2023 | két időzítési modell: kompatibilis (CTM, 1 ms) és alacsony késleltetésű (LLTM); `TM` paraméter; `SSN` ST2110-40:2023 csak `TM` esetén |
| **-41** | gyors metaadat-keretrendszer (Data Item Package) | 2024, **2026** | 2026 | csak az A melléklet két hibájának javítása; az `SSN` értéke továbbra is ST2110-41:2024 |
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
  `source-filter` (SSM); a stúdió által előállított folyam `TSMODE=NEW` (a mi programkimenetünk) vagy
  `SAMP` (ha a bemenet időbélyegét adjuk tovább), `TSDELAY` mikroszekundumban.
- **Kép (-20):** csomagonként legfeljebb 3 Sample Row Data fejléc; a hossz a pgroup többszöröse;
  az utolsó csomagon M=1. Csomagolási mód: **GPM** (általános) vagy **BPM** (7 × 180 bájt, kiterjesztett
  UDP nélkül); a vevőnek mindkettőt fogadnia kell. Kötelező fmtp: `sampling`, `depth`, `width`,
  `height`, `exactframerate`, `colorimetry`, `PM`, `SSN`; alapértelmezettel: `interlace`,
  `segmented`, `TCS`, `RANGE`, `MAXUDP`, `PAR`. Mintavétel: YCbCr/CLYCbCr/ICtCp 4:4:4, 4:2:2, 4:2:0,
  RGB, XYZ, KEY; mélység 8/10/12/16/16f; színtér BT601/709/2020/2100, ST2065-1/-3, XYZ, ALPHA.
- **Forgalomformálás (-21):** a küldő három típusa közül **a szoftveres küldőnek a Type W való**
  (a szabvány 7.1.4 3. megjegyzése éppen a szoftveres forrásokra szánja: V<sub>RX FULL</sub> legalább
  720 csomag, C<sub>MAX</sub> legalább 16, β = 1,10, lineáris olvasási ütemezés). Az SDP-ben
  `TP=2110TPW`, eltérő eltolásnál `TROFF`. Vevőtípusok: N és W szinkron (azonos PTP), A aszinkron
  (bármit fogad); **a mi vevőnk A típusú** lesz, puffereléssel.
- **Tömörített kép (-22):** állandó bájtszám és csomagszám képkockánként, 90 kHz, `b=AS:` bitsebesség,
  `TP` kötelező; a konkrét kodek payload formátuma külön RFC (JPEG XS: RFC 9134; VC-2: SMPTE ST 2042 /
  RFC 8450).
- **Hang (-30):** AES67 szerint; 48 kHz kötelező; minden eszköz legalább **A szint** (48 kHz, 1 ms
  csomagidő, 1–8 csatorna); a B szint 125 µs-os csomagidő; csatornarend `channel-order=SMPTE2110.(…)`
  (M, DM, ST, LtRt, 51, 71, 222, SGRP, Unn). Csak a szabványos UDP-méret.
- **AES3 (-31):** `AM824` alcímke, 32 bites alkeretek (B, F, PCUV, 24 bit adat), páros csatornaszám,
  `ptime` táblázat szerint. Így a nem-PCM tartalom (pl. ST 337 szerint csomagolt adat) érintetlenül
  átmegy; mi csak továbbítjuk, nem dekódoljuk.
- **ANC (-40):** RFC 8331; legalább egy csomag képkockánként (üres is, `ANC_Count=0`, M=1);
  `exactframerate` kötelező; `TM=CTM` (1 ms) vagy `TM=LLTM`; FID-csoportosítás tilos.
- **Gyors metaadat (-41):** Data Item Package (22 bites típus, K bit, 9 bites hossz, 32 bites
  egységek), legalább 500 ms-onként egy csomag, M=0, `SSN=ST2110-41:2024`, ajánlott `DIT` lista.
- **TTML (-43):** TTML2, IMSC1.2 profil, RFC 8759, 90 kHz; a dokumentum időpontjai az RTP-időbélyeghez
  relatívak (`timeBase="media"`).

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
   menedzsment-API `ts-refclk`-ja a PTP nagymester azonosítóját és tartományát jelzi.
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
| **T2** – Hang (-30) | AES67 küldő és vevő, A szint (48 kHz, 1 ms, 1–8 csatorna), utána B (125 µs); csatornarend; a libobs hangkeverőbe és -ből | T1, S3 (hangkeverő) | egy külső AES67-forrás hangja a keverőben; a program hangja egy AES67-vevőben; késleltetés mérve |
| **T3** – Képbemenet (-20, -21 vevő) | A típusú vevő (GPM és BPM, 4:2:2 10 bit, 1080p50/59,94, 720p, 2160p ha a kártya bírja; V. fejezet), 2022-7 redundáns pár, ST 2110 forrás a jelenetben | T1, S2 (rögzítő bemenetek) | egy 1080p50 4:2:2 10 bites folyam (a 2,5 GbE-n ez fér el, V. fejezet) elejtett képkocka nélkül egy órán át a referenciagépen; csomagvesztésnél kerethiba nélkül (eldobott képkocka, számlálva) |
| **T4** – Programkimenet (-20, -21 küldő) | W típusú küldő `SO_TXTIME` ütemezéssel, `TP=2110TPW`, saját -21 elemző, 2022-7 kimenet, SDP a menedzsment-API-n | T3, S4 (stúdió mód) | a saját elemző és egy külső vevő szerint W-megfelelő egy órán át; a program képe bitazonos a stúdióéval |
| **T5** – Adatfolyamok (-40, -43, -41, -31) | ANC: CEA-608/708 (ST 334) és időkód a `cfa4-captions`-ből, CTM, később LLTM; TTML/IMSC1.2 (-43) a `cfa4-captions` TTML-íróból; -41 keretrendszer (a regisztrált típusok közül, amire igény van); -31 AM824 átlátszó továbbítás | T4 | az élő feliratok egy vevőben a képpel szinkronban; az AM824 tartalom bitazonosan átmegy |
| **T6** – Tömörített kép (-22) | VC-2 (D10), CBR, 90 kHz, `b=AS`, N vagy W forgalom | T4, D10 | egy -22 folyam be és ki a referenciagépen, bitsebesség és késleltetés mérve |
| **T7** – Kezelés és felület | NMOS IS-04/IS-05 (D11); Élő munkaterület: folyamok, SDP, PTP-állapot, hibaszámlálók (Qt6, a D7 szerint a Qt6 befejezése után) | T4, D7 | egy NMOS-vezérlő felderíti és bekapcsolja a CFA4 küldőit és vevőit |

**Ütemezés:** a T0 és a T1 nem érinti a Qt6 fájljait és a libobs-ot, ezért az S0-hoz hasonlóan
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
  - **nem fér el:** 1080p59,94 4:2:2 10 bit (≈ 2,49 hasznos, ≈ 2,6 a vonalon) és minden 2160p
    tömörítetlenül; ezek a hurokinterfészen tesztelhetők, valódi hálózaton -22-vel (tömörítve);
  - ezért a T3 és T4 „kész, ha” mérése a referenciagépen **1080p50 4:2:2 10 biten** fut; a
    1080p59,94 10 bites és a 2160p tömörítetlen mérés egy 10/25 GbE-s kártyára vár (a tulajdonos
    döntése, nem feltétele egyik fázis indulásának sem).
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
- **Szabadalom:** a -22-ben elterjedt JPEG XS nincs a CFA4-ben (D10), ezért JPEG XS-t küldő vagy váró eszközzel a CFA4 tömörítve nem, csak tömörítetlenül (-20) kapcsolható össze.
- **Szerzői jog:** a szabványszövegek nem kerülnek a tárolóba, a tesztekbe sem kerül belőlük szöveg;
  a példák (SDP) saját, a szabvány szerint felépített adatok.

## VIII. Kapcsolódás a fő tervhez

A fő terv (v1.1) I. 8. pontja és D4 sora erre a kiegészítésre hivatkozik; a IV. fejezet 2. (bemenetek)
és 6. (kimenetek) sorába az ST 2110 bekerül. A fázisbeosztás a fő terv fázisai közé illeszkedik
(T2 az S3 után, T3 az S2 után, T4 az S4 után), azokat nem változtatja meg.
