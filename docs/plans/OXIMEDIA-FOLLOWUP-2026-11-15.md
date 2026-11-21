# OXIMEDIA-FOLLOWUP – az OxiMedia-értékelés utáni feladatok (C2PA, FFmpeg-stílusú CLI)

**Dátum:** 2026-11-15 · **Állapot:** a tulajdonos (`ubuntuokos`) jóváhagyta 2026-11-15-én („Jóváhagyom, QC
nélkül”). Döntés: [ADR 0015](../adr/ADR-0015-c2pa.md) (C2PA). ·
**Előzmény:** az OxiMedia 0.2.1 értékelése (2026-10-30; átvétel: `cfa4-metrics`, #905/#908) és a 2026-11-08-i
terv („CFA4 – OxiMedia-ötletek terve”, Claude Doc), amelyet a tulajdonos 2026-11-15-én módosítva hagyott jóvá

## 1. Cél és hatókör

Az OxiMedia (Apache-2.0, „tiszta Rust FFmpeg + OpenCV”) maga nem használható, de két ötlete olyan hiányt fed
le, amelyre a CFA4-ben nincs megoldás és más projekt sem dolgozik rajta:

1. **C2PA tartalomhitelesítés exportkor** (CFA4-C2PA-001): aláírt jegyzék a kész JPEG, PNG és MP4 fájlban.
2. **FFmpeg-stílusú parancssor** (CFA4-FF-CLI-001): `cfa4-cli ff -i be.mov … ki.mp4`, hogy az ffmpeg-hez szokott
   felhasználók és szkriptek a CFA4 saját kodekjeit és exportját ismerős kapcsolókkal érjék el.

**Nem része** (a 2026-11-08-i tervből kikerült, átfedés miatt, `docs/coordination.md` §2.8):

| Eredeti pont | Miért nem | Hol van |
|---|---|---|
| Renderelt kimenet QC-je (BT.1702 villogás, fagyás, R103 gamut, EBU/ATSC profilok) | A MEDIA-QC projekt területe; a tulajdonos „QC nélkül” hagyta jóvá | `crates/qc`, [`MEDIA-QC-2026-10-27.md`](MEDIA-QC-2026-10-27.md) |
| IMF App 2E export | Már elkészült | `crates/export/src/imf_out.rs` (#1011) |

## 2. Sorrend és becslés

| # | Feladat | Becslés | Előfeltétel |
|---|---|---|---|
| 1 | C2PA ([ADR 0015](../adr/ADR-0015-c2pa.md), C2-0 … C2-5) | 42–71 ó | ez a terv és az ADR a `main`-ben |
| 2 | FFmpeg-stílusú CLI (FF-0 … FF-3) | 15–25 ó | nincs (a C2PA után, mert a C2PA kapcsolót is kiteszi) |

Minden fázis külön PR, friss `main`-ről, egy feladatazonosítóval commitonként (`C2-1: …`, `FF-1: …`). A
beolvasztás a gazda dolga ([`docs/coordination.md`](../coordination.md)).

## 3. C2PA

A részletek és az indoklás az [ADR 0015](../adr/ADR-0015-c2pa.md)-ben. Röviden:

- **C2-1** `cfa4-c2pa` (L1): saját determinisztikus CBOR-kódoló, JUMBF-dobozok, claim v2,
  `c2pa.actions.v2`, COSE_Sign1 ES256 `x5chain`-nel; helyben létrehozott saját gyökér-CA és általa kiadott,
  nem önaláírt aláíró tanúsítvány (a C2PA tanúsítványprofilja az önaláírt aláírót tiltja);
  kulcstárolás `$XDG_DATA_HOME/cfa4/c2pa/`, `0600`. Tesztek: CBOR-ismert vektorok (RFC 8949 A. függelék),
  ECDSA-ellenőrzés a saját láncon, fuzz a CBOR/JUMBF/COSE-olvasóra, sérült és csonka bemenetek.
- **C2-2** JPEG (APP11) és PNG (`caBX`) beágyazás `c2pa.hash.data`-val, utófeldolgozásként.
- **C2-3** MP4 felső szintű `uuid` doboz `c2pa.hash.bmff`-fel, a `cfa4-isobmff` íróban lefoglalt hellyel.
- **C2-4** Export paraméter `c2pa` (alapból ki): engine parancs, `cfa4-cli export --c2pa`, MCP, Qt6
  jelölőnégyzet automation id-vel; a gyökértanúsítvány exportja (`c2pa.root.export`).
- **C2-5** Importkor a médiainformációban a jegyzék összefoglalója és az ellenőrzési állapot.

**Elfogadási feltétel (C2-2, C2-3):** a `c2patool` beolvassa a kimenetet, a kötés és az aláírás érvényes, az
egyetlen várt állapot `signingCredential.untrusted`; egy bájt módosítása a médiában kötési hibát ad.
A `c2patool` hiányában a teszt kihagyja magát. Teljesítmény: a jegyzék írása a hash-számításon felül egy 1 GB-os
MP4-nél legfeljebb 1 s legyen a referenciagépen ([`docs/development/reference-machine.md`](../development/reference-machine.md)).

## 4. FFmpeg-stílusú CLI

Nem az ffmpeg újraírása, hanem egy fordítóréteg a meglévő `cfa4-cli` parancsokhoz: a kapcsolókból
`file.import` + `file.exportMedia` paraméterek lesznek, ugyanazon az engine-en, amit a `cfa4-cli export` hív.

- **FF-0** Leképezési táblázat (`docs/cli-ff.md`): mely ffmpeg-kapcsolók támogatottak (`-i`, `-c:v`/`-c:a`,
  `-b:v`/`-b:a`, `-crf`, `-preset`, `-s`/`-vf scale`, `-r`, `-ss`/`-to`/`-t`, `-an`/`-vn`, `-f`, `-y`/`-n`,
  `-map` szűk része, `-metadata`), melyik CFA4-paraméterre képződnek, és mi a viselkedés eltérése. A nem
  támogatott kapcsoló hibát ad (nem csendes figyelmen kívül hagyás).
- **FF-1** `cfa4-cli ff`: a kapcsolók feldolgozása a meglévő `args.rs` mintájára, `--dry-run` kiírja az
  előállított engine-hívást JSON-ban; idő `Tick`-ben (`-ss 1.5` → pontos tick, f64 nélkül).
- **FF-2** Végrehajtás és kilépési kódok; `-c:v copy` csak ahol a CFA4 smart-render/átcsomagolás tudja,
  különben érthető hiba.
- **FF-3** Orákulum: ugyanaz a parancssor ffmpeg 9.0.2-vel és `cfa4-cli ff`-fel; az `ffprobe` adatai
  (kodek, méret, képkocka-szám, időtartam) egyezzenek, a képminőséget a `cfa4-metrics` SSIM/ΔE méri
  (küszöb a kodektől függően, a tesztben rögzítve). ffmpeg hiányában a teszt kihagyja magát.

A részletes CLI-tervet az FF-0 PR hozza; a tervbe nem kerülhet új funkció tulajdonosi jóváhagyás nélkül
(§2.10).

## 5. Never crash és biztonság

- Minden új olvasó (CBOR, JUMBF, COSE, X.509, ffmpeg-kapcsolók) hibát ad rossz bemenetre, nem pánikol;
  méretkorlátok a foglalás előtt.
- Kulcsanyag sosem kerül naplóba, projektfájlba, repóba vagy hibaüzenetbe.
- CodeRabbit-review: a C2-1 és C2-3 kell (biztonság, konténer), a többi nem (§2.9).

## 6. Döntések

| Dátum | Döntés |
|---|---|
| 2026-11-15 | Sorrend: C2PA, majd CLI; a QC-kiegészítés és az IMF kimarad (a tulajdonos: „Jóváhagyom, QC nélkül”) |
| 2026-11-15 | C2PA: saját szűk író, nem a `c2pa` crate |
| 2026-11-15 | C2PA: saját tanúsítvány kezdetben; az ADR 0015 ezt saját gyökér-CA + aláíró láncként valósítja meg |
| 2027 eleje | Az OxiMedia újabb kiadásainak újraértékelése a [`docs/gaps.md`](../gaps.md) szerint |
