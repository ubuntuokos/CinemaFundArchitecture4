# ADR 0015 – C2PA tartalomhitelesítés exportkor, saját szűk íróval (CFA4-C2PA-001)

**Dátum:** 2026-11-15 · **Állapot:** javasolt; a tulajdonos (`ubuntuokos`) 2026-11-15-én jóváhagyta a tervet és a
két alapdöntést (saját író, saját tanúsítvány), ez az ADR a részleteket rögzíti ·
**Terv:** [`docs/plans/OXIMEDIA-FOLLOWUP-2026-11-15.md`](../plans/OXIMEDIA-FOLLOWUP-2026-11-15.md) ·
**Érintett rész:** új crate (`cfa4-c2pa`, L1) a C2-1 fázistól, később `cfa4-isobmff`, `cfa4-export`, a
`cfa4-cli serve` és a Qt6 exportpárbeszéd; ez az ADR kódot nem változtat

## Háttér

A C2PA (Coalition for Content Provenance and Authenticity) tartalomhitelesítése („Content Credentials”) a
fájlba ágyazott, aláírt nyilatkozat arról, ki és milyen szoftverrel készítette vagy módosította a médiát. A
célalkalmazás exportkor tud ilyet írni ([`docs/target-app-parity.md`](../target-app-parity.md)), a CFA4 nem.
Az OxiMedia-értékelés (2026-10-30) utáni terv első pontja ez lett; az OxiMedia nem tartalmaz C2PA-t, ötletet
adott, kódot nem.

A tulajdonos döntései (2026-11-15):

1. Saját, szűk C2PA-író készül, nem a `c2pa` crate (a teljes SDK nagy, sok függőséggel, és a CFA4-nek csak az
   írás és a saját jegyzékek ellenőrzése kell).
2. Az aláíró tanúsítvány kezdetben saját (nem kereskedelmi CA-tól vásárolt).

A „saját tanúsítvány” megvalósításához egy tény számít: a C2PA tanúsítványprofilja (C2PA 2.x, §14.5.1
„Certificate Profiles”) **tiltja az önaláírt aláíró tanúsítványt**, és az aláíró nem lehet CA. A referencia-
ellenőrző (contentauth `c2pa-rs`, `sdk/src/crypto/cose/certificate_profile.rs`) ezt így ellenőrzi:
v3 tanúsítvány, nem CA, nem önaláírt, kötelező EKU a megengedett listából (pl. `id-kp-documentSigning`
1.3.6.1.5.5.7.3.36, C2PA Signing 1.3.6.1.4.1.62558.2.1), `anyExtendedKeyUsage` tilos, `keyCertSign` csak CA-n,
AKI és SKI kell, ECDSA P-256/P-384/P-521, Ed25519 vagy legalább 2048 bites RSA(-PSS).

## Döntés

1. **Új L1 crate: `cfa4-c2pa`**, `unsafe` nélkül, a never-crash szabályokkal (`AGENTS.md` §0): ellenőrzött
   hosszak, korlátozott foglalás, minden olvasó hibát ad, nem pánikol. CFA4-függősége nincs; a konténerbe
   írást a hívó (export, `cfa4-isobmff`) végzi a crate által adott bájtokkal és kizárási tartományokkal.
2. **Saját, szűk író** (a tulajdonos 1. döntése), a C2PA 2.x szerint:
   - JUMBF (ISO/IEC 19566-5) jegyzéktár: `c2pa` szuperdoboz, benne egy jegyzék (`c2ma`) az
     állítástárral (`c2as`), a claimmel (`c2cl`) és az aláírással (`c2cs`);
   - claim v2 (`c2pa.claim.v2`) determinisztikus CBOR-ban, saját kis CBOR-kódolóval;
   - állítások az első változatban: `c2pa.actions.v2` (`c2pa.created`, a `softwareAgent` a CFA4 neve és
     verziója) és a kemény kötés: JPEG és PNG esetén `c2pa.hash.data`, MP4 esetén `c2pa.hash.bmff`;
   - aláírás: COSE_Sign1 (RFC 9052), ES256, az `x5chain` fejlécben a végfelhasználói és a gyökértanúsítvány.
   A titkosítási primitíveket nem írjuk meg magunk: ECDSA P-256 és SHA-256 a RustCrypto crate-jeiből
   (`p256`/`ecdsa`, `sha2`; Apache-2.0/MIT, tiszta Rust), a tanúsítványokhoz a RustCrypto `der`/`x509-cert`.
3. **Saját tanúsítványlánc, nem önaláírt aláíró** (a tulajdonos 2. döntésének értelmezése a fenti profil
   miatt): első használatkor a CFA4 helyben létrehoz egy saját gyökér-CA-t (önaláírt, `cA=true`,
   `keyCertSign`) és egy általa kiadott aláíró tanúsítványt (v3, `cA=false`, KeyUsage `digitalSignature`,
   EKU `id-kp-documentSigning`, AKI/SKI, ECDSA P-256). Mindkettő a felhasználó gépén keletkezik; a CFA4 nem
   szállít kulcsot, a kulcs sosem kerül a repóba vagy a projektfájlba.
   - Tárolás: `$XDG_DATA_HOME/cfa4/c2pa/` (kulcsok PKCS#8-ban, `0600`, a könyvtár `0700`); a gyökér
     tanúsítványa exportálható, hogy a felhasználó vagy a címzett felvehesse egy ellenőrző bizalmi listájára.
   - Következmény: egy általános ellenőrző (pl. c2patool, Content Credentials Verify) az aláírást érvényesnek,
     de a láncot **nem megbízhatónak** mutatja (`signingCredential.untrusted`), amíg a gyökér nincs a bizalmi
     listáján. Ezt a CFA4 kezelőfelülete kimondja; „megbízható” jelölést nem állítunk.
   - Később (külön döntéssel): a felhasználó saját, CA által kiadott tanúsítványának és kulcsának betöltése.
4. **Nincs hálózat:** az első változat nem kér RFC 3161 időbélyeget és nem ellenőriz OCSP-t, ezért exportkor
   nincs kimenő kapcsolat. Időbélyeg nélkül az aláírás az aláíró tanúsítvány érvényességi idejéhez kötött; az
   aláíró tanúsítvány 2 évig, a gyökér 20 évig érvényes, lejárat előtt a CFA4 új aláírót ad ki.
5. **Adatvédelem alapértelmezésként:** a jegyzék nem tartalmaz személyes adatot (név, e-mail, gépnév), csak a
   szoftver nevét és verzióját és a műveletet. A szerző megnevezése (`cawg`/`schema.org` szerzői állítás)
   későbbi, kifejezett bekapcsolású lehetőség.
6. **Beágyazás:** JPEG-ben APP11 szegmens(ek), PNG-ben `caBX` darab, MP4-ben felső szintű `uuid` doboz
   (a C2PA UUID-jával). A kizárási tartomány a jegyzék helye; JPEG és PNG utófeldolgozással íródik, MP4-ben a
   `cfa4-isobmff` írója foglal helyet, hogy a `stco`/`co64` eltolásokat ne kelljen utólag átírni. Az MP4-es
   doboz pontos helyét a C2-3 fázis a c2patool orákulummal rögzíti.
7. **Parancs és kezelőfelület** (*everything is a command*): az export egy `c2pa` paramétert kap (alapból
   kikapcsolva), a CLI-ben és az MCP-ben ugyanaz a paraméter, a Qt6 exportpárbeszédben jelölőnégyzet
   automation id-vel. Import oldalon a médiainformáció megmutatja a jegyzék összefoglalóját és az ellenőrzés
   eredményét (saját jegyzékeinkre teljes, idegenre a kötés és az aláírás matematikai ellenőrzése).
8. **Ellenőrzés:** a crate saját tesztjei mellett független orákulum a `c2patool` (contentauth,
   Apache-2.0/MIT, csak tesztidőben, külső program); ha nincs telepítve, a teszt kihagyja magát, mint az
   ffmpeg-orákulumok ([`docs/testing.md`](../testing.md)). Az elvárás: c2patool hibamentesen beolvassa,
   a kötés és az aláírás érvényes, egyetlen várt állapot a `signingCredential.untrusted`. A CBOR-, JUMBF- és
   COSE-olvasó fuzz-tesztet kap.

## Fázisok

| Fázis | Tartalom | Becslés |
|---|---|---|
| C2-0 | Ez az ADR és a terv a `main`-ben | – |
| C2-1 | `cfa4-c2pa`: CBOR, JUMBF, claim v2, állítások, COSE_Sign1 ES256, tanúsítványlánc létrehozása és tárolása; egység- és fuzz-tesztek | 15–25 ó |
| C2-2 | JPEG és PNG beágyazás `c2pa.hash.data`-val; c2patool orákulum | 8–12 ó |
| C2-3 | MP4 beágyazás `c2pa.hash.bmff`-fel a `cfa4-isobmff` írójában; c2patool orákulum | 10–18 ó |
| C2-4 | Export paraméter (engine parancs, CLI, MCP, Qt6 jelölőnégyzet), gyökértanúsítvány exportja | 4–8 ó |
| C2-5 | Olvasás és megjelenítés importkor, ellenőrzési állapot | 5–8 ó |

Összesen 42–71 óra, a 2026-11-08-i terv 40–70 órás becslésével egyezően.

## Következmények

- Új függőségek (C2-1-től): `p256`, `ecdsa`, `x509-cert` (RustCrypto, Apache-2.0/MIT); a `sha2` és a `der`
  már a munkaterületen van. Felvételük a [`docs/licensing.md`](../licensing.md) szerint, a `cargo xtask
  layers` táblájában a `c2pa` L1.
- Az export szerződése egy opcionális paraméterrel bővül; alapértelmezésben semmi nem változik.
- A kulcsok a felhasználó gépén maradnak; a CFA4 nem vállal kulcsvédelmet a fájlrendszer jogosultságain túl.
- A „nem megbízható lánc” állapot várt és dokumentált; aki megbízható láncot akar, a későbbi saját-tanúsítvány
  betöltéssel kap.

## Kapcsolódó dokumentumok

- [`docs/plans/OXIMEDIA-FOLLOWUP-2026-11-15.md`](../plans/OXIMEDIA-FOLLOWUP-2026-11-15.md)
- C2PA Technical Specification 2.x (c2pa.org), ISO/IEC 19566-5 (JUMBF), RFC 9052 (COSE), RFC 8949 (CBOR),
  RFC 5280 (X.509)
- [`docs/licensing.md`](../licensing.md) §5, [`crates/metrics/README.md`](../../crates/metrics/README.md)
  (az OxiMedia-értékelés korábbi átvétele)
