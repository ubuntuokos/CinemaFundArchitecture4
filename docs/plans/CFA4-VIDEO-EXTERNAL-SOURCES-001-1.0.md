# CFA4-VIDEO-EXTERNAL-SOURCES-001 v1.0: külső videóforrások (metaadat, felirat, átirat, engedélyes letöltés)

> **Állapot:** JÓVÁHAGYVA – végleges (FINAL-1.0). Horváth István 2026-11-15-én 23:40-kor (Europe/Budapest) jóváhagyta. A V-1–V-14 döntéseket István 2026-11-15-én meghozta (11. fejezet). A jóváhagyás a tervre szól, nem megvalósítási engedély: minden fázis (F0–F6) külön engedélyt igényel, automatikus merge vagy kiadás nincs. A dokumentum a `main`-re nem került fel.
> **Pontosítás (2026-11-15, Europe/Budapest):** a korábban csak részben olvasott `lib.rs`, `request.rs`, `real_dcpomatic.rs` és `docs/agents.md` teljes olvasása után a 5.1, 5.3, 5.4, 7., 9., 12. és 13. fejezet frissült. Ezt követően a tulajdonos a dokumentumot jóváhagyta (lásd Állapot).
> **Kiinduló `main`:** `ce8057bb53fdee862a91633608af58d97a5a71c4` (#1385, 2026-11-15 20:59 CET). Ellenőrizve a `git fetch` után: a `main` nem változott. A négy fájl teljes olvasásakor a HEAD ugyanaz volt, fetch, pull és módosítás nélkül.
> **Előzmény:** CFA4-VIDEO-SCRAPING-APIS-ADOPTION-001 v2.0 (a tulajdonos elemzésre adta át) és annak teljes elemzése (2026-11-15). A terv a v2.0 helyébe lép.
> **Platform:** Ubuntu 26.04 LTS, AMD64 (VISION.md 47–48).
> **Jelölés:** `fájl:sor` a `ce8057bb` állapotra vonatkozik, ha nincs más SHA. **[nyitott]**: forrásból nem ellenőrizhető tény. A tervben nincs olyan pont, amely tulajdonosi döntésre várna.

## 0. Összefoglaló

A CFA4 háromféleképpen kaphat külső videóforrásról adatot. A forrás mindig a `.fcproj` v17-ben rögzül.

- **S0 – offline import:** a felhasználó saját fájlja (felirat, átirat, oEmbed-JSON). Nincs hálózat, és a PG-től független (V-8).
- **S1 – oEmbed-metaadat:** csak a `cfa4-fetch` folyamaton át. Alapból kikapcsolva, minden kérés előtt jóváhagyó ablak jelenik meg, és minden kérés naplózódik (V-5).
- **ÁSZF-engedélyes letöltés:** csak ott, ahol a platform ÁSZF-je kifejezetten megengedi. Ugyanazon az úton fut, mint az S1, a K18 közös letöltőre épül, és a motor csak a helyi fájlt kapja meg (V-9, V-11).

Harmadik fél scraperei (S2) teljesen ki vannak zárva (V-2). A donorkatalógust a terv nem veszi át (V-1).

## 1. Cél és hatókör

### 1.1 Cél
1. A már meglévő felirat- és átiratimportot kiegészíteni azzal, hogy honnan, mikor és milyen licenccel jött az adat (`ExternalRecord`).
2. Felhasználói jóváhagyással lekérni egy videó-URL nyilvános oEmbed-metaadatát: cím, szerző, szolgáltató, méret, bélyegkép-URL (oembed.com §2.3.4).
3. Videót letölteni, de csak ÁSZF-engedéllyel, és csak a felhasználó által kért, jóváhagyott esetben.
4. A közben szükséges biztonsági előfeltételeket megvalósítani: a `cfa4-fetch` programot, a host-engedélylistát, és a speech-letöltés átköltöztetését.

### 1.2 Hatókörben
- S0 a meglévő parancsokra építve:
  - `captions.import` (docs/captions.md:65)
  - SRT/VTT (captions.md:24–25)
  - `transcript.set` (docs/transcripts.md:26)
  - `ProjectItem::metadata` (crates/project/src/lib.rs:310–312)
- `.fcproj` v17 az `external` mezővel, `v16_to_v17` migrációval és `repair()`-rel.
- `cfa4-external` (L2) crate és `cfa4-fetch` (L6) program, a szolgáltatóleíró adatfájllal (K18).
- Qt6 felület: importpanel, jóváhagyó ablak, Beállítások ▸ Külső források, naplónézet. Az egui be van fagyasztva, abba nem kerül új funkció (VISION.md 37–39).
- A speech modellletöltésének átköltözése a `cfa4-fetch`-be (V-12).
- A `mi.external_ref` származtatott tábla, csak a PG-4 után (V-7).

### 1.3 Hatókörön kívül
- **Harmadik fél scraperei** (Apify, APIMesa és társaik), bármilyen formában (V-2).
- **A donortár tartalma:** szöveg, link, affiliate-paraméter (V-1; licensing.md:12–13 és 102).
- **Fizetős vagy kulcsos API-k**, a felhasználó saját kulcsával sem (V-3, V-4; licensing.md:113–132).
- **Bejelentkezés, cookie, fiókhoz kötött tartalom, kommentek** és a 6. fejezet többi tiltott kategóriája (V-10).
- **Letöltés olyan platformról, amelynek ÁSZF-je nem engedi kifejezetten.** Ilyen például a YouTube: az ÁSZF engedély nélkül tiltja az automatizált hozzáférést és a letöltést (https://www.youtube.com/static?template=terms, hatályos 2023-12-15).
- **Feltöltés vagy publikálás videómegosztóra.** Ez a VISION.md 184. sorában nyitott külön kérdés, nem ennek a tervnek a része.
- **Felhős MI**, például felhős átírás. Erre a VISION.md 186-os kérdése általánosságban nyitott marad. Ez a terv csak a metaadat- és letöltési hálózati utat dönti el.
- **Bélyegkép automatikus letöltése:** a `thumbnail_url` csak szövegként tárolódik.
- **OxiMedia.**
- **CFA3/FA3-tervek:** csak jelzésként szerepelhetnek, jóváhagyásként nem.

## 2. Szabálytérkép

| Szabály | Forrás | Mit jelent itt |
|---|---|---|
| „FINAL 2.0” adatküldési tilalom | **[nyitott]**: a CFA4-MEDIA-INTELLIGENCE-001 FINAL 2.0 szövege „nincs a tárolókban” (docs/plans/CFA4-AI-MODEL-TRAINING-001.md:22; DATA-PG terv:232) | Nem hivatkozunk rá. A tartalmát az alábbi valós szabályok fedik le. |
| A felhasználó adatai nem hagyják el a gépet | VISION.md:49 | S1 és letöltés csak kérésenkénti jóváhagyással, a kimenő adat megmutatásával |
| Nincs futásidejű hálózati függőség | docs/adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md:17–19 | Minden funkció hálózat nélkül is működik. A hálózat külön folyamat opcionális része. |
| A motor nem nyit kimenő kapcsolatot | ADR-F8:36; docs/plans/CFA4-SECURITY-001.md:122 (D5) | Hálózatot csak a `cfa4-fetch` használ |
| A `cfa4-fetch` szerepe | CFA4-SECURITY-001.md:82 és 122; crates/sandbox/src/profile.rs:21–22 és 234–238; crates/sandbox/README.md:22–31 | Csak a letöltés célját írhatja, csak 443-ra és DNS-re kapcsolódhat, programot nem indíthat. **Ma csak profil:** az `apps/` alatt csak `cfa4`, `cfa4-cli` és `cfa4-studio-host` van. |
| Ha hiányzik egy védelem, a rész nem indul | CFA4-SECURITY-001.md:121 (D4) | Sandbox nélkül a `cfa4-fetch` nem indul |
| A modell nem töltődik le a felhasználó tudta nélkül | VISION.md:154 | A V-12 költöztetés után is jóváhagyással megy |
| A letöltés ma a motorban fut | crates/speech/src/models.rs:248 és 315–328; docs/security/threat-model.md:50 és 57 | V-12: a B3 előtt át kell költöztetni (threat-model:57) |
| SEC-02: az adat engedély nélkül nem hagyja el a gépet | docs/plans/CFA4-FILE-SERVICES-001-1.0.md:27 és 116 | Ugyanaz az elv |
| Nincs külső adatküldés engedély nélkül; nincs automatikus letöltés | CFA4-AI-MODEL-TRAINING-001.md:196, 334, 388–389 (AC-17, AC-18) | Ugyanaz az elv |
| Nincs fizetős rész és nincs használati korlát | docs/licensing.md:15–20 és 113–132 | Nincs költségkeret. A korlátok technikai (biztonsági) korlátok. |
| Licenc nélküli forrás = „all rights reserved” | licensing.md:12–13 és 102 | Donor: V-1 |
| Külön folyamatként futó külső program nem érinti a CFA4 licencét | licensing.md:108–111 | `cfa4-fetch` saját kód, GPL-3.0-or-later |
| Tulajdonosi minta: kapcsolható külső szolgáltatás | docs/plans/SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md:220–222 és 253 (D3) | Külön folyamat, alapból KI, kifejezett bekapcsolás |
| Tulajdonosi minta: K18 közös letöltő | docs/plans/CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md:288, 324–326, 631 | Adatfájlban leírt szolgáltatók; letöltés előtt látszik a forrás, a méret és a licenc; jóváhagyás, hash, folytatás |
| Figyelmeztetés távoli erőforrásra | KDENLIVE:129 és 585 (K46) | A motor URL-t nem nyit meg |
| Online frissítés csak kézi paranccsal | docs/plans/CFA4-CINECERT-001-v2.3.md:298 és 303 | Nincs háttérkérés |
| Titok csak a keyringben | crates/project/src/studio.rs:13; docs/plans/CFA4-LIVE-STUDIO-001.md:104–106; CINECERT:189, 311, 316 | A v1.0-ban nincs titok (V-3, V-4) |
| Online táblázatok „a cfa4-fetch-re várnak” | docs/credits-plan.md:330–331; docs/plans/CFA4-TITLE-CREDITS-001-1.0-CFA4-ILLESZTES.md:246 | Más tervek is a `cfa4-fetch`-re várnak |
| Alkalmazáson belüli letöltés jóváhagyással megengedett, automatikus telepítés nem | TITLE-CREDITS-ILLESZTES:275 és 292 | Ugyanaz |
| Hálózati URL automatikusan nem futhat; az első kör helyi fájl | docs/plans/CFA4-FFMPEG-BACKEND-R1.md:109 és 181 | A motor és a médiaháttér csak helyi fájlt kap |
| Külső folyamat mintája (tulajdonos által jóváhagyva) | docs/plans/CFA4-DCPOMATIC-001-FINAL-1.0.md:4 (végleges, jóváhagyva 2026-11-13), 42 (D2), 71 és 74; megvalósítva: crates/dcpomatic/src/process.rs:1–3, 28–68, 99–132, crates/dcpomatic/README.md:44–56; ugyanez a minta: crates/dcp-inspect/README.md:51–61; az ADR-0011:37–43 és 79–80 ugyanezt írja le, de az ADR státusza még „javasolt” (3–5) | Argumentumtömb, shell nélkül, saját folyamatcsoport, kiürített környezet, privát 0700-s HOME/TMPDIR/munkakönyvtár, SIGINT, majd 5 s után SIGKILL, időtúllépés, ellenőrzés indulás előtt, a program keresése sosem a PATH-on. Eltérés: 5.1 |
| Helyi ellenőrzés internet nélkül; külső folyamat argumentumai biztonságosan; ideiglenes fájlok törlése | docs/plans/DCP-VALIDATION-2026-10-27.md:53, 526, 528 | Ugyanaz az elv: S0 hálózat nélkül, 5.1 |
| MI: csak a K18-on át, jóváhagyással; felhős MI csak külön ADR-rel | AI_modules ADR-0008:117–119 és 331–332 (@`fc8b7335`) | V-4: kulcsos API csak külön ADR-rel |
| Az igazságforrás a `.fcproj`; a mi származtatott | DATA-PG terv:48, 400, 561 (P8, I5) | V-6, V-7 |
| Offline-first; az adat nem hagyja el a gépet | DATA-PG terv:78–79 és 435 (R1, R2) | S0 hálózat nélkül; S1 kérésenként |
| A terv a `main`-en él | VISION.md:56–57 | F0: a jóváhagyott terv a `main`-re kerül |

## 3. Architektúra

### 3.1 Egy összevont út

```text
Qt6 GUI ── importpanel / jóváhagyó ablak / napló ──▶ cfa4-cli serve (motor, L4, csak loopback)
                                                     │  external.* parancsok
                                                     │  cfa4-external (L2): leíró, terv, parser
                                                     │  cfa4-project (L1): ExternalRecord, .fcproj v17
                                                     │
                      (S1 és letöltés, egy jóváhagyott kérés)
                                                     ▼
                                     cfa4-fetch (L6, külön folyamat, Fetch-profil)
                                     host-engedélylista, 443, DNS, csak a célkönyvtár
                                                     │ HTTPS
                                                     ▼
                                      leíróban rögzített hostok (F2 után)
```

- **S0:** a felhasználó fájlja a motorba kerül. A motor a meglévő parancsokkal importál, és `ExternalRecord`-ot ír `Retrieval::UserFile` értékkel. Hálózat nincs.
- **S1:** a motor a leíró alapján `FetchPlan`-t készít. A GUI megmutatja a jóváhagyó ablakot. Csak jóváhagyás után indul a `cfa4-fetch`. Az oEmbed-választ már a `cfa4-fetch` normalizálja, a motor `repair()`-rel újra ellenőrzi. Rekord `Retrieval::OEmbed` értékkel.
- **ÁSZF-engedélyes letöltés:** két külön jóváhagyott kérés.
  1. Méretlekérdezés (`HEAD`), hogy a méret ismert legyen.
  2. A letöltés maga a felhasználó által választott célkönyvtárba: `.part`, SHA-256, átnevezés (crates/speech/src/models.rs:264–312 mintája).
  A motor a helyi fájlt újrahash-eli, és csak egyezés esetén importálja a szokásos médiaimporttal. URL a motorhoz és a médiaháttérhez nem jut (FFMPEG-R1:109 és 181).
- **S2:** kizárva (V-2, 6. fejezet).

### 3.2 Rétegek és csomagok (xtask/src/main.rs:97–143)

| Csomag | Réteg | Tartalom | Függőség |
|---|---|---|---|
| `cfa4-project` (meglévő, L1, main.rs:100) | L1 | `crates/project/src/external.rs`: `ExternalRecord`, `Retrieval`, `Produced`, `TosRef`, `repair_external()`; `Project::external` | változatlan |
| `cfa4-external` (új) | L2, ahogy a `captions` és a `speech` (main.rs:116, 118) | leírósémák és -validátor, `ExternalProvider` trait, oEmbed-parser, `FetchPlan`/`FetchRequest`/`FetchResult` IPC-típusok, tiltólista | `cfa4-project`, serde, `toml`; **hálózat és fájlrendszer nélkül** |
| `cfa4-format` (meglévő, L3, main.rs:133) | L3 | `v16_to_v17` | – |
| `cfa4-engine` (meglévő, L4, main.rs:134) | L4 | `external.*` parancsok, a `cfa4-fetch` indítása (`helper_exes`, profile.rs:123–125), hash-ellenőrzés, napló | `cfa4-external` |
| `cfa4-fetch` (új, `apps/cfa4-fetch`) | L6, ahogy a `studio-host` (main.rs:143) | az egyetlen hálózati program: rustls, ureq (a speech-ből áthozva), sandbox önlezárás | `cfa4-external`, `cfa4-sandbox` (L1, main.rs:110) |
| `cfa4-speech` (meglévő, L2) | L2 | a `download` feature és az `ureq` kikerül (models.rs:248, 315–328); a modellfájl-lista és a SHA marad | – |
| `cfa4-store` (DATA-PG) | a DATA-PG szerint | `mi.external_ref` migráció a PG-4-ben | DATA-PG terv:185 és 442 |
| Qt6 (`python/cfa4_qt/panels/`) | GUI | importpanel, jóváhagyó ablak, beállítások, napló | engine-parancsok (docs/qt-gui.md:64–70) |

Az `xtask` `LAYERS` táblájába két új sor kerül: `("external", 2)` és `("fetch", 6)`.

## 4. Adatmodell és típusok

### 4.1 Rust-típusok (`cfa4-project`, L1)

```rust
// crates/project/src/external.rs
pub const MAX_RECORDS_PER_ITEM: usize = 32;
pub const MAX_FIELDS: usize = 32;
pub const MAX_FIELD_CHARS: usize = 10_000; // = studio MAX_TEXT_CHARS (studio.rs:24)
pub const MAX_URL_BYTES: usize = 4096;     // = studio MAX_PATH_BYTES (studio.rs:26)
pub const MAX_ID_CHARS: usize = 256;       // = a store azonosítóhatára (DATA-PG terv:61)

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExternalRecord {
    /// Leíró-azonosító: "file:srt", "file:vtt", "file:transcript-json", "file:oembed-json", "oembed:<id>", "download:<id>".
    pub provider: String,
    /// A szolgáltató saját videóazonosítója, ha ismert.
    pub external_id: Option<String>,
    /// Csak https://, legfeljebb MAX_URL_BYTES.
    pub url: Option<String>,
    /// Az oEmbed `version` mezője vagy a fájlformátum verziója.
    pub api_version: Option<String>,
    pub retrieval: Retrieval,
    /// A beolvasott bájtok SHA-256-ja (fájl, válaszjson vagy letöltött fájl), 64 kisbetűs hex.
    pub content_sha256: String,
    pub licence: Option<String>,
    /// A leíró ÁSZF-hivatkozása a lekérés pillanatában (S0-nál None).
    pub tos: Option<TosRef>,
    /// Engedélyezett mezők (4.4), szöveges értékkel.
    pub fields: BTreeMap<String, String>,
    /// Mit hozott létre a projektben.
    pub produced: Vec<Produced>,
    /// 1-től nő ugyanarra a (provider, external_id) párra; a régi rekord megmarad.
    pub revision: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Retrieval {
    UserFile { file_name: String },
    OEmbed { request_id: String, approved_at_utc_ms: i64 },
    Download { request_id: String, approved_at_utc_ms: i64, bytes: u64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TosRef { pub source: String, pub date: String } // date: ÉÉÉÉ-HH-NN

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Produced { Transcript, Captions, Metadata { keys: Vec<String> }, MediaFile }
```

- A `request_id` UUIDv7, ahogy a File Services azonosítója (CFA4-FILE-SERVICES-001-1.0.md:60–67).
- A `Transcript::source` értékkészlete nem változik: S0- és S1-eredetű átiratnál `"imported"` (crates/project/src/transcript.rs:68–76). Hogy honnan jött, azt az `ExternalRecord.produced` mutatja meg.

### 4.2 `.fcproj` v17 és migráció

```rust
// crates/project/src/lib.rs, Project (a 1277–1317. sor mintájára)
/// External sources (CFA4-VIDEO-EXTERNAL-SOURCES-001): where imported data came from, keyed by
/// the project item it belongs to. Schema v17.
#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
pub external: BTreeMap<ItemId, Vec<ExternalRecord>>,
```

```rust
// crates/format/src/lib.rs (a MIGRATIONS a 64–66. sorban, a SCHEMA_VERSION a 69–70. sorban)
fn v16_to_v17(doc: Value) -> Result<Value, String> {
    Ok(doc) // no-op bump: egy régebbi build mentéskor elhagyná az `external` mezőt
}
```

- **Miért no-op lépés:** a docs/project-files.md:69–73 szabálya szerint akkor kell sémaverziót emelni, ha egy régebbi build csendben eldobná az új mezőt. A minta a `v15_to_v16` (format/lib.rs:401–403).
- **Tesztek:**
  - kézi v16-os fixture a `crates/format/tests/fixtures/` alá (project-files.md:63–66)
  - `schema_version_matches_table` → 17 (format/lib.rs:410–412)
- **V-14:** a docs/project-files.md:55 táblázata csak v13-ig tart, a kód v16-on áll. Ezt külön dokumentációs javítás rendezi, ennek a tervnek nem része.

### 4.3 Repair-határok (a studio.rs:236–275 mintájára)

`repair_external(&mut BTreeMap<ItemId, Vec<ExternalRecord>>, &BTreeMap<ItemId, ProjectItem>) -> usize` betöltéskor fut, és visszaadja az eldobott elemek számát.

1. Eldobja a nem létező `ItemId`-hez tartozó rekordokat.
2. Eldobja azt a rekordot, amelynek a `provider`-e nem felel meg ennek: `^[a-z0-9][a-z0-9:._-]{0,63}$`.
3. Eldobja a nem `https://` kezdetű vagy `MAX_URL_BYTES`-nál hosszabb `url`-t. A rekord ilyenkor megmarad, csak az `url` lesz `None`.
4. Eldobja a rekordot, ha a `content_sha256` nem 64 kisbetűs hex.
5. `MAX_ID_CHARS`-ra vágja az `external_id`-t, `MAX_FIELD_CHARS`-ra a `fields` értékeit és a `licence`-t. Legfeljebb `MAX_FIELDS` kulcsot tart meg, és csak a 4.4 engedélyezett kulcsait. A `html` mindig kiesik.
6. Ugyanabból a (provider, external_id, revision) hármasból csak az elsőt tartja meg.
7. Tételenként legfeljebb `MAX_RECORDS_PER_ITEM` rekordot tart meg, a legnagyobb `revision`-öket.
8. Ha a `tos.date` nem ÉÉÉÉ-HH-NN alakú, a `tos` értéke `None` lesz.

### 4.4 Engedélyezett mezők

- **oEmbed** (oembed.com §2.3.4): `type`, `version`, `title`, `author_name`, `author_url`, `provider_name`, `provider_url`, `cache_age`, `thumbnail_url`, `thumbnail_width`, `thumbnail_height`, `width`, `height`.
- **A `html` mezőt soha nem tároljuk és nem rendereljük.** Az oembed.com §3 XSS-kockázatot ír, és ha szükséges, idegen domainről töltött iframe-et ajánl. A CFA4-ben nincs beágyazott lejátszó.
- **URL-mezők** (`author_url`, `provider_url`, `thumbnail_url`) csak `https://` sémával maradnak meg (oembed.com §3). Szövegként tárolódnak, a CFA4 nem nyitja meg őket.
- **S0** fájlnév, nyelv és formátum mezőket tárol.

### 4.5 A szolgáltatóleíró adatfájl (K18, V-11)

- Hely: `data/external-providers/<id>.toml`.
- A csomag a validátoron átment, F2-ben jóváhagyott leírókat viszi.
- A felhasználó a Beállításokban nem vehet fel újat, csak a meglévőket kapcsolhatja be vagy ki.

```toml
schema = 1
id = "oembed:pelda"                 # ^(oembed|download):[a-z0-9._-]{1,48}$
name = "Példa videószolgáltató"
kinds = ["oembed"]                  # "oembed" | "download"
allowed_hosts = ["www.pelda.hu"]    # pontos hostnevek; nincs * és nincs IP-literál
url_schemes = ["https://www.pelda.hu/video/*"]   # oembed.com §2.1
endpoint = "https://www.pelda.hu/oembed"         # host ∈ allowed_hosts
tos_source = "https://www.pelda.hu/aszf"         # az ÁSZF pontos URL-je
tos_date = "2026-11-15"                          # az ellenőrzött ÁSZF-változat dátuma
tos_permits = ["oembed"]            # amit az ÁSZF kifejezetten megenged; a "download" csak szó szerinti engedéllyel
tos_quote = "…"                     # az engedélyt adó ÁSZF-mondat szó szerint
legal_gate = "F2-pelda-2026-11-15"  # az F2 döntési bejegyzés azonosítója
licence_note = "A tartalom a feltöltőé; a CFA4 csak a metaadatot tárolja."
```

**A validátor elutasítja a leírót, ha:**
- hiányzik a `tos_source`, a `tos_date`, a `tos_quote` vagy a `legal_gate`;
- a `kinds` nem részhalmaza a `tos_permits`-nek;
- van benne `http://`, csillag, IP-literál, userinfo vagy 443-tól eltérő port;
- az `endpoint` vagy egy `url_schemes`-elem hostja nincs az `allowed_hosts`-ban;
- van benne hitelesítési mező (`api_key`, `token`, `auth`, `cookie`, `header`);
- van benne alapértelmezett bekapcsolást jelző mező;
- az `id` vagy a `name` illeszkedik a 6. fejezet tiltólistájára.

**Elévülés:** ha a `tos_date` 365 napnál régebbi, a leíró futásidőben „ÁSZF-ellenőrzés esedékes” állapotba kerül, és nem használható, amíg az F2 újra nem ellenőrzi.

**A v1.0 csomagban nincs előre felvett szolgáltató.** Leíró csak F2-döntéssel kerülhet be. YouTube-letöltési leíró nem készülhet, mert az ÁSZF tiltja (1.3).

### 4.6 `ExternalProvider` trait (`cfa4-external`, L2)

```rust
pub trait ExternalProvider {
    fn descriptor(&self) -> &ProviderDescriptor;
    /// Tiszta függvény: I/O nincs. A jóváhagyó ablak mindent innen mutat.
    fn plan(&self, req: &ImportRequest) -> Result<FetchPlan, ProviderError>;
    /// Tiszta függvény: a cfa4-fetch már korlátozott, ellenőrzött válaszát normalizálja.
    fn parse(&self, plan: &FetchPlan, resp: &FetchResult) -> Result<ExternalRecord, ProviderError>;
}

pub trait LocalImporter { // S0
    fn id(&self) -> &str; // "file:srt" …
    fn import(&self, file_name: &str, bytes: &[u8]) -> Result<(ExternalRecord, ImportPayload), ProviderError>;
}

pub struct FetchPlan {
    pub request_id: String,            // UUIDv7
    pub provider: String,
    pub purpose: Purpose,              // OEmbed | Probe | Download
    pub url: String,                   // a ténylegesen hívott URL (az endpoint + a kódolt videó-URL)
    pub host: String,
    pub max_bytes: u64,                // 5.5
    pub target: Option<PathBuf>,       // csak Download: a felhasználó választotta könyvtár
    pub approval: ApprovalInfo,        // 5.3
}
```

A szolgáltató nem lát hálózatot, ezért minden szolgáltató offline, fixture-ökkel tesztelhető.

### 4.7 `mi.external_ref` DDL (V-7, csak a PG-4 után)

```sql
-- cfa4-store migráció, SET ROLE cfa4_owner_mi (DATA-PG terv:386); a PG-4 része (DATA-PG terv:185, 442)
CREATE TABLE mi.external_ref (
    id               uuid        PRIMARY KEY,
    asset_id         uuid        NOT NULL REFERENCES mi.asset(id) ON DELETE CASCADE,
    media_item_ref   text        NOT NULL CHECK (char_length(media_item_ref) BETWEEN 1 AND 256),
    provider         text        NOT NULL CHECK (provider ~ '^[a-z0-9][a-z0-9:._-]{0,63}$'),
    external_id      text        NULL     CHECK (char_length(external_id) <= 256),
    url              text        NULL     CHECK (url LIKE 'https://%' AND octet_length(url) <= 4096),
    api_version      text        NULL     CHECK (char_length(api_version) <= 32),
    retrieval        text        NOT NULL CHECK (retrieval IN ('user_file', 'oembed', 'download')),
    content_sha256   bytea       NOT NULL CHECK (octet_length(content_sha256) = 32),
    licence          text        NULL     CHECK (char_length(licence) <= 10000),
    tos_source       text        NULL,
    tos_date         date        NULL,
    record_revision  integer     NOT NULL CHECK (record_revision >= 1),
    retrieved_at     timestamptz NULL,
    source_rev       text        NOT NULL,
    UNIQUE (asset_id, media_item_ref, provider, record_revision)
);
CREATE INDEX external_ref_provider_idx ON mi.external_ref (provider, external_id);
```

- **Csak sémán belüli FK** (`mi.asset`, DATA-PG terv:240). Sémák közötti FK nincs.
- **A tábla származtatott.** A `.fcproj`-ból bármikor újraépíthető a `mi.project_link`-en át (DATA-PG terv:241 és 264–272), a `source_rev` ugyanazt jelenti, mint a `mi.transcript`-ben (DATA-PG terv:243).
- **Mező, amely a `.fcproj`-ban nincs, a táblában sincs.** A `fields` nem kerül a PG-be: a keresés a projektből olvassa.

## 5. `cfa4-fetch`, kapcsolók, titkok, korlátok, jóváhagyás, napló

### 5.1 Folyamatmodell (a CFA4-DCPOMATIC-001 jóváhagyott mintája)

A minta a jóváhagyott CFA4-DCPOMATIC-001 (4., 42., 71. és 74. sor) és annak megvalósítása (crates/dcpomatic/src/process.rs). Ugyanezt írja le az ADR-0011:37–43, és ugyanígy működik a `cfa4-dcp-inspect` is (README.md:51–61).

1. **Keresés:** a motor a `cfa4-fetch`-et a DCP-o-matic kétlépcsős mintájára keresi, **a `PATH`-on soha** (lib.rs:174–181; az üres környezeti változó sem számít felülírásnak, lib.rs:181).
   - **Fordítási idejű előtag:** a DCP-o-matic `PRIVATE_DIR` alapértéke `/usr/lib/cfa4/dcpomatic` (lib.rs:44), más előtagú csomagban a `CFA4_DCPOMATIC_DIR` fordítási idejű értéke (lib.rs:41–43, `option_env!`). A `cfa4-fetch` ugyanígy: a bináris neve `cfa4-fetch`, az alapértelmezett könyvtár `/usr/lib/cfa4`, más előtagú csomagban a `CFA4_FETCH_DIR` (fordítási idő). A teljes alapértelmezett út tehát `/usr/lib/cfa4/cfa4-fetch` (CFA4-DCPOMATIC-001:49 D9).
   - **Fejlesztői felülírás:** a DCP-o-matic a `CFA4_DCPOMATIC_BIN_DIR` könyvtárat használja a `PRIVATE_DIR` helyett (lib.rs:46–48, 173–181). A `cfa4-fetch` egyetlen bináris, ezért a `CFA4_FETCH_BIN` a futtatható fájl útját adja meg, nem könyvtárat.
   - **Elérhetőség:** ha a program hiányzik vagy nem indul, az `external.status` a `Dcpomatic::locate()` `Unavailable` mintájára (lib.rs:85–87, 186–197; CFA4-DCPOMATIC-001:72) „nem elérhető” állapotot ad, okkal, és a CFA4 többi része változatlanul működik.
   - **Nem átvett, indokkal:** a harmadik féltől származó, rögzített DCP-o-matic kiadás alsó verzióhatára (`MIN_VERSION` 2.18.51, lib.rs:50, 216–221; real_dcpomatic.rs:38). A `cfa4-fetch` a CFA4 saját programja, nem külső, rögzített eszköz.
2. **Indítás:** kérésenként egy példány, argumentumtömbbel, shell nélkül, saját folyamatcsoportban (`process_group(0)`, process.rs:66). A futtatható fájl frissen írt állapotából adódó átmeneti `ETXTBSY` hibánál rövid ismétlés (process.rs:70–84). A `cfa4-fetch` összeomlása hiba a motornál, soha nem a CFA4 összeomlása (lib.rs:9 mintája).
3. **Környezet:** kiürítve (`env_clear`, process.rs:53–68). Csak a `LANG`/`LC_ALL` megy át, és ha egyik sincs, `LANG=C.UTF-8` (process.rs:57–64).
   - **Eltérés:** a `PATH` és az `LD_LIBRARY_PATH` nem megy át, mert a `cfa4-fetch` programot nem indít (tests.rs:54), és nem prefixbe fordított könyvtárakkal fut.
   - A proxy-változók nem mennek át (6. fejezet, 4. pont).
   - `HOME`, `TMPDIR` és a munkakönyvtár egy privát, 0700-s könyvtár. A motor hozza létre, és a futás után törli (process.rs:28–49, 65–66). A Fetch-profil ezt nem teszi írhatóvá, csak semleges munkakönyvtár.
4. **Csatorna – eltérés a mintától, indokkal:** a DCP-o-matic és a dcp_inspect fájlba írja a kimenetet, hogy ne teljen meg a pipe, és a stdin-jük zárva van (process.rs:1–3; dcpomatic README.md:44 és 47; dcp-inspect README.md:51–57). A `cfa4-fetch` viszont a Fetch-profil miatt semmit nem olvashat, és csak a letöltés célját írhatja (tests.rs:49–54). Fájlos csatornához a profilt bővíteni kellene. Ezért marad a pipe:
   - a kérés egyetlen JSON-sor (`FetchRequest`) a stdin-en, legfeljebb 64 KiB, utána a motor zárja a stdin-t;
   - a válasz a stdout-on érkezik (haladássorok és egy záró `FetchResult`, legfeljebb 1 MiB + haladás). **A motor saját szálon folyamatosan olvassa**, így a pipe nem telik meg;
   - a stderr-ből csak az utolsó 4 KiB marad meg hibaüzenetnek (a process.rs:26 és 168–173 `TAIL` és `last_message` mintájára).
5. **Önlezárás az első lépésben:** a `cfa4-sandbox` Fetch-profilja (profile.rs:234–238). Csak a letöltés célja írható, kifelé csak 443-ra és DNS-re lehet kapcsolódni, programindítás nincs (tests.rs:49–54). Ha a lezárás nem sikerül, a program kilép, és nem kapcsolódik (D4, SECURITY-001:121).
   - **Különbség a DCP-o-matictól:** annak a hálózati tilalma ma csak a `--no-remote` kapcsolón múlik (crates/dcpomatic/src/request.rs:383; CFA4-DCPOMATIC-001:130), saját sandbox-profilja nincs (profile.rs folyamatfajtái, sandbox/README.md:22–31). A `cfa4-fetch` nem kapcsolóra, hanem kernelszintű profilra és programszintű hostlistára (5.2) támaszkodik.
6. **Ellenőrzés indulás előtt:** a motor indítás előtt ellenőrzi a kérést (leíró, kapcsolók, host, cél). Hibás kérésre nem indul folyamat (CFA4-DCPOMATIC-001:74; dcpomatic README.md:51–53).
7. **Megszakítás:** SIGINT a csoportnak, 5 s után SIGKILL (process.rs:23–24 és 99–120; ADR-0011:42–43).
   - A DCP-o-matic a félkész filmkönyvtárat megszakításkor és időtúllépéskor törli, motorhiba után viszont megőrzi, mert a naplója abban van (lib.rs:225–241; dcpomatic README.md:48–50).
   - **Eltérés, indokkal:** a `.part` nem diagnosztikai napló, hanem félkész letöltés. Megszakításkor, időtúllépéskor, motorhibánál és akkor is törlődik, ha a folyamatot SIGKILL állította le (a `cfa4-fetch` töröl, ha tud; SIGKILL esetén a motor). A diagnosztika a JSONL-naplóba és a stderr utolsó 4 KiB-jába kerül (5.7, 5.1 4. pont).
8. **Időtúllépések:**
   - kapcsolódás: 30 s (a models.rs:315–328 ügynökének értéke)
   - oEmbed és HEAD összesen: 60 s
   - letöltés: 120 s haladás nélkül, ami a vezérlőcsatorna 2 perces socket-határával egyezik (docs/control-protocol.md:21)
   Lejáratkor SIGKILL a csoportnak (process.rs:121–125).
   **Eltérés, indokkal:** a DCP-o-matic egy teljes futtatásra 48 órát ad (lib.rs:170, 199–204), mert egy 4K nagyjáték kis gépen lassú; a `--version` lekérdezés 60 s (lib.rs:217). A `cfa4-fetch` hálózati kérés, ezért rövidebb, tevékenységhez kötött korlátokat kap.
9. **Kimenet:**
   - oEmbed esetén normalizált mezők (4.4)
   - Probe esetén `Content-Length` és `Content-Type`
   - Download esetén a cél útja, a mérete és a SHA-256-ja
   A válasz nyers HTML-je és törzse soha nem kerül a motorhoz.
10. **A motor oldalán:**
    - a `FetchResult`-ot a `repair_external()` újra ellenőrzi;
    - letöltés után a motor újrahash-eli a fájlt, és csak egyezés esetén importálja;
    - a médiaháttér csak helyi utat kap.

### 5.2 Host-engedélylista (V-13, programszinten)

A sandbox-profil csak portot szűr (profile.rs:236–237). A hostellenőrzést ezért a `cfa4-fetch` maga végzi, **minden kapcsolódás előtt**:

1. A séma `https`, a port 443, nincs userinfo. A host kisbetűsítve, IDNA-normalizálva **pontosan** szerepel a leíró `allowed_hosts` listájában.
2. IP-literál tilos. A feloldott címek közül a loopback, a privát, a link-local, a CGNAT és a multicast címet elutasítja. Ez SSRF-védelem.
3. Átirányítás legfeljebb 3, és mindegyik célt ugyanez a szabály ellenőrzi. Engedélyezett hostról nem engedélyezettre átirányítani hiba.
4. **TLS:** rustls és a platform-hitelesítő (models.rs:315–320).
5. **Fejlécek:** rögzített `User-Agent: CFA4/<verzió>`. Nincs cookie-tár, `Authorization`, `Referer`, sem egyéni fejléc.

A speech-letöltés (V-12) saját leírót kap: `download:hf-openai-whisper`, a `huggingface.co` hosttal (models.rs:340). Hogy a Hugging Face mely CDN-hostjaira irányít át, az **[nyitott]**. Az F3-ban a valódi szolgáltatással futó teszt állapítja meg, és a leíróba kerül.

### 5.3 Jóváhagyó ablak (Qt6, minden kérés előtt)

**Az ablak mutatja:**
- a szolgáltató nevét és azonosítóját;
- a kérés célját: metaadat, méretlekérdezés vagy letöltés;
- a pontosan hívott URL-t és hostot;
- **mi hagyja el a gépet:** a hívott URL a benne kódolt videó-URL-lel, a gép IP-címe, a `User-Agent`;
- a méretet: oEmbednél a felső határt (5.5), letöltésnél a méretlekérdezés `Content-Length` értékét;
- a licencet: a leíró `licence_note` mezőjét, és ha van, a válasz adatát;
- az ÁSZF-forrást és -dátumot (`tos_source`, `tos_date`);
- a célt: csak projektmetaadat, vagy fájlút.

**Gombok:** „Ezt az egy kérést engedélyezem” és „Mégse”. Az alapértelmezett fókusz a „Mégse”. Nincs „mindig engedélyezem” lehetőség.

**A jóváhagyás csak a GUI ablakából jöhet.** A `docs/agents.md` két MCP-módot különböztet meg (10–16): a headless (`cfa4-cli mcp --demo` vagy `--project`, nincs ablak) és a híd (`cfa4-cli mcp --bridge`, a futó asztali alkalmazás). A headless MCP-nek nincs mappakorlátozása, oda ír, ahová a felhasználó (agents.md:17–19).
- **Headless MCP és headless CLI:** az `external.fetch.run` elutasítva (`cfa4-cli mcp` headless, control-protocol.md:200). Ablak nincs, jóváhagyás nincs.
- **Híd / vezérlőcsatorna:** a kérés elindulhat, de a jóváhagyó ablaknak az asztali alkalmazásban meg kell jelennie, alapértelmezett fókusz a „Mégse”. A `ui_click` / `ui.key` kattintásának maradék kockázatát a 9. fejezet tárgyalja (agents.md:85–88, 156–160).
- **MCP-eszközjelzések:** ha ehhez a funkcióhoz külön MCP-eszköz készül, a négy hintet be kell állítani (agents.md:104–107), köztük az `openWorldHint` (hálózat) értékét; az eszköz nem lehet csak olvasható. A v1.0-ban a parancs a meglévő `command_run`-on át megy.
- **Hosszú futás:** a letöltés haladása a GUI-ban jelenik meg. Az export MCP-haladás/lemondás mintáját (agents.md:119–139) a v1.0 nem veszi át, mert headlessből a letöltés nem indítható; a lemondás a `.part` fájlt ettől függetlenül törli (5.1, 7. pont).

### 5.4 Kill-switch három szinten

1. **Build:** a `cfa4-fetch` külön bináris a privát programmappában (5.1, 1. pont). Ha nincs telepítve, az `external.status` a `Dcpomatic::locate()` `Unavailable` mintájára (lib.rs:85–87, 186–197) „nem elérhető” állapotot ad, az S1 és a letöltés nem indul, minden más működik (ADR-F8:17–19; CFA4-DCPOMATIC-001:78). A `cfa4-speech` `download` feature-e megszűnik, a motorban nem marad hálózati kód.
2. **Globális beállítás** a `preferences.json`-ban, amely nem projektadat (docs/project-files.md:226–231):
   - `external.network.enabled = false` (alapértelmezés)
   - `external.download.enabled = false` (alapértelmezés; csak bekapcsolt hálózat mellett állítható)
3. **Szolgáltatónként:** `external.providers.<id>.enabled = false` (alapértelmezés).

A kapcsolók mellett minden egyes kérés előtt ott van a jóváhagyó ablak is (5.3). Ha bármelyik szint ki van kapcsolva, a motor `FetchPlan`-t sem készít. Ilyenkor nem jelenik meg ablak, és nem indul folyamat.

### 5.5 Korlátok (nem költség, licensing.md:113–132)

| Korlát | Érték |
|---|---|
| Kérés jóváhagyásonként | 1. Újrapróbálás csak új jóváhagyással. Háttérkérés, ütemezett kérés és előbetöltés nincs (CINECERT:298, 303) |
| Egyidejű `cfa4-fetch` | legfeljebb 2 (1 metaadat + 1 letöltés) |
| Ugyanarra a hostra | legalább 2 s két kérés között |
| oEmbed-válasz | legfeljebb 1 MiB, `Content-Type: application/json` (oembed.com §2.3.1), UTF-8 |
| HEAD | törzs nélkül; `Content-Length` kötelező |
| Letöltés | `Content-Length` kötelező, a fájl legfeljebb ekkora lehet. Ha nincs ilyen fejléc, nincs letöltés |
| Átirányítás | legfeljebb 3 |
| Fizetős vagy kulcsos szolgáltatás | nincs: a validátor elutasítja (4.5) |

### 5.6 Titokkezelés

- **A v1.0-ban nincs titok:** se kulcs, se jelszó, se cookie (V-3, V-4). A validátor elutasítja a hitelesítési mezőt.
- **Ha később külön ADR kulcsos API-t enged (V-4):**
  - a kulcs csak a Secret Service-ben lehet (CINECERT:316; a tiszta Rust D-Bus kliens, a `zbus`, már része a kódnak: apps/cfa4/src/appearance.rs:16–18);
  - nem kerülhet a `.fcproj`-ba (studio.rs:13), naplóba, argumentumba, és sem a bridge, sem az MCP nem adhatja vissza (CINECERT:189 és 311; control-protocol.md:15–16).

### 5.7 Naplózás

- **Hely:** `${XDG_STATE_HOME:-~/.local/state}/cfa4/external-requests.log`, helyi, JSON Lines formátumban, csak hozzáfűzéssel. 10 MiB után forgatás, 3 fájl marad.
- **Mezők:** `ts_utc`, `request_id`, `provider`, `purpose`, `host`, `url`, `decision` (`approved`, `cancelled` vagy `refused_by_policy` a kapcsoló, a validátor vagy a hostlista miatt), `status`, `bytes`, `sha256`, `error`.
- **Soha nem kerül bele:** választörzs, titok vagy fejlécérték.
- **Megtekintés és törlés:** Beállítások ▸ Külső források ▸ Napló. Az `ExternalRecord` `request_id`-je a naplósorra mutat.
- **A napló a gépen marad** (R2, DATA-PG terv:79). Tartalmazhat személyes adatot (`author_name`) és a felhasználó érdeklődését jelző URL-eket.

## 6. Tiltólista (V-10) és S2-kizárás

A tiltott kategóriákra **nincs** szolgáltatóleíró, parancs vagy kód. A validátor és a `cfa4-fetch` technikailag is kizárja őket. A számok csak jelzések: a donor VIDEOS-szakaszának teljes gépi bejárásából származnak (donor `settings/APIFY_ACTORS.md:29076–30361` @`25f6cb31`, 979 tétel).

| # | Kategória | Kikényszerítés | Jelzés a donorból |
|---|---|---|---|
| 1 | Vízjel-eltávolítás | Nincs ilyen funkció; a leíró nem deklarálhatja | 36 tétel |
| 2 | Nézettség-generálás | Egy kérés jóváhagyásonként, hostonként 2 s, nincs ismétlés | 2 tétel („view generator”) |
| 3 | Cookie-alapú hozzáférés | Nincs cookie-tár és nincs hitelesítési mező | 12 tétel |
| 4 | Residential proxy | Kiürített környezet, nincs proxy-beállítás, csak közvetlen kapcsolat az engedélyezett hosttal | 61 tétel („proxy”) |
| 5 | Email- és telefonszám-gyűjtés | Csak a 4.4 mezői maradnak meg | 37 tétel |
| 6 | Felnőtt tartalom | Az F2 ilyen platformra nem ad leírót; a validátor a tiltólistás nevet elutasítja | 12 tétel |
| 7 | Kommentek tömeges gyűjtése | Nincs kommentmező és nincs kommentvégpont | 166 tétel („comment”) |

**S2:** harmadik fél scraperei, aggregátorai és API-piactere (Apify, APIMesa) nem lehetnek szolgáltatók, mert a host csak a tartalom saját platformja lehet, és csak F2-döntéssel (4.5).

**A donor helyzete:**
- nincs LICENSE;
- minden linken affiliate-paraméter van (donor `fetch_apify_actors.js:11`);
- a donor maga is figyelmeztet a platform-ÁSZF-re és a GDPR-ra (donor `README.md:167` és `170`).
Ezért csak piaci jelzés (V-1).

## 7. Tesztek

| Szint | Teszt |
|---|---|
| `cfa4-project` | A `repair_external()` minden 4.3-szabályra kap ellenséges bemenetet; oda-vissza szerializálás; üres `external` esetén nincs kimenet (`skip_serializing_if`) |
| `cfa4-format` | v16-os fixture betöltése; `SCHEMA_VERSION == 17`; egy régebbi build v17-et olvasva verzióhibát ad (project-files.md:69–73) |
| `cfa4-external` | Validátor: minden 4.5-elutasítás, a 365 napos elévülés, a tiltólista. oEmbed-parser fixture-ökkel: `html` eldobva, nem https URL eldobva, ismeretlen kulcs eldobva, 1 MiB fölött hiba, nem JSON típus hiba. A `kinds ⊄ tos_permits` eset hiba |
| Folyamatvezérlés (motor) | Hamis `cfa4-fetch` shell-szkripttel, a dcpomatic és dcp-inspect tesztjeinek mintájára (CFA4-DCPOMATIC-001:76; dcpomatic README.md:60–63; dcp-inspect README.md:108–116): pontos argumentumtömb; kiürített környezet; privát könyvtár törlése; a `PATH` soha nem keresett; üres `CFA4_FETCH_BIN` a privát útra esik vissza (lib.rs:181); hiányzó program esetén `external.status` „nem elérhető”; megszakítás, amely a csoport háttérben indított gyermekét is leállítja; időtúllépés; folyamatosan olvasott stdout nagy kimenetnél sem akad el |
| `cfa4-fetch` | Helyi TLS-tesztszerverrel (hálózat nélküli CI): host nincs a listán; IP-literál; privát cím a DNS-ben; átirányítás nem engedélyezett hostra; 4. átirányítás; nincs `Content-Length`; túlméret; időtúllépés; SIGINT/SIGKILL; `.part` takarítása; SHA-egyezés. Az önlezárás után a `security.status` a `fetch` profilt mutatja (control-protocol.md:65) |
| Sandbox | A motor profilja nem kap kimenő hálózatot. A `fetch` csak a célt írhatja (tests.rs:49–54 bővítése) |
| Kapcsolók | Alapállapotban egyik `external.*` parancs sem indít folyamatot, és nem nyit ablakot. Mindhárom szint önállóan tilt |
| Jóváhagyás | Headless MCP/CLI-ből az `external.fetch.run` hiba (agents.md:10–16). Hídból a kérés csak az asztali jóváhagyó ablak után fut le. A „Mégse” után nincs folyamat. A naplóban `cancelled` sor jelenik meg |
| Napló | Minden ágról pontosan egy sor; nincs törzs és nincs fejléc; forgatás |
| Engine | Letöltés után a motor újrahash-el; eltérésnél nincs import; URL nem jut a médiaháttérhez (FFMPEG-R1:109 és 181) |
| Speech (V-12) | A `cfa4-speech` `ureq` és `download` nélkül fordul. A modellletöltés a `cfa4-fetch`-en át megy, SHA-val. A models.rs:340-es URL-teszt megmarad |
| Qt6 | Az importpanel és a jóváhagyó ablak automatizálási azonosítókkal; valódi motor elleni teszt a `python/tests/test_qt_gui.py` mintájára (qt-gui.md:187) |
| mi (F6) | Újraépítés a `.fcproj`-ból; a CHECK-ek; nincs sémák közötti FK |
| Agents DoD (docs/agents.md:360–372) | Az `external.*` parancsok címkével, menüúttal és paraméterleírással regisztrálva; a tiltott (disabled) eset teszttel fedett (5.4 kapcsolók); az S0/import undo/redo a meglévő importmintát követi; minden új interaktív vezérlőnek van automatizálási azonosítója; vezérlőcsatornás végigvitel és képernyőkép; a crate README és a progress dokumentumok frissülnek |
| Valódi szolgáltatás | Csak kézi, jelölt futtatás, egy F2-ben jóváhagyott leíróval. A CI-ben nincs külső hálózat. Mint a `real_dcpomatic.rs` (51–58): ha a szükséges környezet nincs beállítva, a teszt kihagy és átmegy, nem bukik |

Kötelező CI-kapuk:
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- `cargo xtask layers`, `cargo deny check licenses sources advisories` (CFA4-FFMPEG-BACKEND-R1.md:250–251)

## 8. Fázisok

| Fázis | Tartalom | Belépési kapu | Kilépési kapu | Függ |
|---|---|---|---|---|
| **F0** Szabály és dokumentum | A terv a `main`-re kerül (VISION.md:56–57). VISION §8 bejegyzés: „Külső videóforrás: oEmbed-metaadat és ÁSZF-engedélyes letöltés kapcsolhatóan, külön folyamatban (`cfa4-fetch`), alapból kikapcsolva, kérésenkénti jóváhagyással és naplózással (CFA4-VIDEO-EXTERNAL-SOURCES-001)”. Ez a VISION.md:186 kérdést erre a területre zárja le, a SUBTITLE D3 mintájára (SUBTITLE-1.2:253). SECURITY D5 kiegészítés: a `cfa4-fetch` céljai közé felkerül az „F2-ben jóváhagyott oEmbed- és letöltési szolgáltatók, a leíró host-engedélylistájával” | A tulajdonos jóváhagyja ezt a tervet | A VISION és a SECURITY-001 módosítása egyesítve, a tulajdonos jóváhagyásával | – |
| **F1** Adatmodell | 4.1–4.3: `ExternalRecord`, v17, `v16_to_v17`, `repair_external()`; a `cfa4-external` váza a validátorral és a tiltólistával (4.5, 6.), még szolgáltató nélkül | F0 | 7. fejezet project-, format- és external-tesztjei zöldek; `cargo xtask layers` PASS | F0 |
| **F2** Jogi kapu | Szolgáltatónként ÁSZF-ellenőrzés (`tos_source`, `tos_date`, `tos_quote`, `tos_permits`), GDPR-értékelés (`author_name`, napló), licencszöveg; döntési bejegyzés (`legal_gate`); YouTube-letöltés nincs | F0 | Minden felvett leíró validátoron átment, és tulajdonosi bejegyzés tartozik hozzá. Nulla leíró is érvényes kimenet | F0 (az F1-gyel párhuzamos) |
| **F3** `cfa4-fetch` és a K18-mag | 5.1–5.7: folyamat, önlezárás, host-engedélylista, három kapcsolószint, jóváhagyó ablak, napló; a speech-letöltés átköltöztetése (V-12) | F1; a `cfa4-sandbox` önlezárása használható a `fetch` profilhoz (sandbox/README.md:6: ma még egyik folyamat sem alkalmazza) | 7. fejezet fetch-, sandbox-, kapcsoló-, napló- és speech-tesztjei zöldek; a motorban nincs `ureq`; threat-model P6 lezárva (threat-model.md:50, 57) | F1, SECURITY-001 B-fázisai (CFA4-SECURITY-001.md:125, D8) |
| **F4** S0 Qt6-import | Importpanel: SRT/VTT/átirat-JSON/oEmbed-JSON fájlból, `ExternalRecord` `UserFile`-lal; a forrás látszik a Project panel metaadatában | F1 | Qt6 valódi motoros teszt; hálózat nélkül teljes | F1 (az F2-től és F3-tól független, V-8) |
| **F5** S1 és engedélyes letöltés | Az F2 leírói élesben, mindhárom kapcsolóval és jóváhagyással; letöltés a K18-on át, hash-ellenőrzéssel, helyi importtal | F2 legalább egy leíróval; F3; F4 | 10. fejezet AC-1–AC-16 PASS; kézi futás valódi szolgáltatással | F2, F3, F4 |
| **F6** `mi.external_ref` | 4.7 DDL, újraépítés, keresés | A DATA-PG PG-4 kész (DATA-PG terv:442) | mi-tesztek zöldek; újraépítés bizonyítva | PG-4, F1 |

## 9. Kockázatok

| Kockázat | Kezelés |
|---|---|
| Az S1 kiküldi a videó URL-jét és az IP-címet (R2) | A jóváhagyó ablak megmutatja (5.3); kérésenkénti engedély; napló |
| Az ÁSZF megváltozik a leíró rögzítése után | `tos_date` és 365 napos elévülés; újraellenőrzés az F2-ben (4.5) |
| Engedélyezett hostról nem engedélyezettre irányít át | Minden ugrás hostellenőrzése; legfeljebb 3 ugrás (5.2) |
| Rosszindulatú válasz (XSS, túlméret, JSON-bomba) | A `html` eldobva; 1 MiB; a parser a `cfa4-fetch`-ben; a motorban `repair()` |
| UI-automatizálás kattint a jóváhagyó ablakra | A vezérlőcsatorna csak 127.0.0.1-en hallgat, tokenhez kötött, és a felhasználó indítja (control-protocol.md:4–19; agents.md:156–160). A híd MCP `ui_click` / `ui.key` eszközei ugyanezt a tokent használják (agents.md:15, 85–88). A napló minden jóváhagyást rögzít. Maradék kockázat: a token birtokosa a GUI-t is vezérli |
| Megtelő pipe vagy elakadó csatorna | A stdout-ot saját szál olvassa folyamatosan, a kimenet korlátos (5.1, 4. pont), és erre teszt is van (7. fejezet) |
| A sandbox még egyik folyamatra sincs alkalmazva | Az F3 belépési kapuja; D4: lezárás nélkül a `cfa4-fetch` nem indul |
| A céges proxy nem használható | Szándékos (6. fejezet, 4. pont); a „nem elérhető” állapotot a GUI kiírja |
| Szerzői jog a letöltött és importált tartalomra | `licence` és `tos` a rekordban; licenc nélkül a felhasználónak kell döntenie (licensing.md:12–13) |
| Személyes adat a metaadatban (`author_name`) | Csak helyben; nem kerül a PG-be (4.7); a kommentek kizárva |
| A K18-at a KDENLIVE P2 prioritással tervezi (KDENLIVE:288); ma nincs kódja | A K18-magot ennek a tervnek az F3-a készíti el; a további K18-szolgáltatók (Hugging Face, Freesound, Pixabay) erre épülnek |
| Az ADR-0011 státusza még „javasolt” (ADR-0011:3–5), pedig a CFA4-DCPOMATIC-001 szerint az 1. fázis PR-jének kellett volna hozzáigazítania (CFA4-DCPOMATIC-001:6 és 65) | A folyamatminta tulajdonosi jóváhagyása a CFA4-DCPOMATIC-001-ből jön (4. és 42. sor), és a minta megvalósítva is van (process.rs). Ez a terv az ADR-0011-et csak leíró forrásként használja |

## 10. Elfogadási kritériumok

- **AC-1** Alapállapotban egyik `external.*` parancs sem nyit hálózati kapcsolatot, nem indít folyamatot és nem nyit ablakot.
- **AC-2** A motor folyamatában nincs hálózati kód. Az `ureq` csak a `cfa4-fetch` függősége.
- **AC-3** A `cfa4-fetch` önlezárás nélkül nem kapcsolódik. Lezárt állapotban csak 443-ra és DNS-re kapcsolódik, és csak a célt írja.
- **AC-4** A `cfa4-fetch` minden kapcsolódás és átirányítás előtt kikényszeríti a host-engedélylistát. A privát és loopback cím tiltott.
- **AC-5** Minden kérés előtt jóváhagyó ablak jelenik meg az 5.3 minden mezőjével. Headless MCP/CLI-ből a jóváhagyás nem lehetséges. Hídból a kérés csak akkor fut le, ha az asztali ablakban jóváhagyják.
- **AC-6** Minden kérésről pontosan egy naplósor keletkezik, választörzs és titok nélkül.
- **AC-7** A leíró validátora minden 4.5-elutasítást kikényszerít. A v1.0 nem szállít F2-döntés nélküli leírót.
- **AC-8** Letöltés csak `download` ∈ `tos_permits`, bekapcsolt `external.download.enabled` és két jóváhagyott kérés után történik. YouTube-letöltés nincs.
- **AC-9** A motor csak újrahash-elt, egyező helyi fájlt importál. URL nem jut a médiaháttérhez.
- **AC-10** `.fcproj` v17: a v16 betöltődik; egy régebbi build a v17-et verzióhibával utasítja el; a `repair_external()` minden határa teszttel igazolt.
- **AC-11** Az S0 hálózat és PG nélkül teljesen működik.
- **AC-12** A `html` és a nem https URL soha nem tárolódik.
- **AC-13** A 6. fejezet hét kategóriájára nincs kód, parancs vagy leíró, és a kikényszerítésük teszttel igazolt.
- **AC-14** A speech modellletöltése a `cfa4-fetch`-en át megy, a SHA-ellenőrzés megmarad.
- **AC-15** A `mi.external_ref` a PG-4 után a `.fcproj`-ból újraépíthető, és nincs benne sémák közötti FK.
- **AC-16** `cargo fmt`, `clippy -D warnings`, `test`, `xtask layers`, `cargo deny` PASS. A Qt6 valódi motoros tesztjei PASS.

## 11. Döntésnapló (István, 2026-11-15)

| V | Döntés |
|---|---|
| V-1 | A donort nem vesszük át, csak jelzésként használjuk. |
| V-2 | Harmadik féltől származó scraperek teljesen kizárva. |
| V-3 | Saját fizetős kulcs most nem. |
| V-4 | A hivatalos kulcsos API-k elhalasztva, csak külön ADR-rel jöhetnek vissza. |
| V-5 | Az S1 oEmbed a `cfa4-fetch`-en át fut, alapból kikapcsolva, a VISION §8 és a SECURITY D5 módosításával; minden kérés előtt jóváhagyó ablak és naplózás. |
| V-6 | Az igazságforrás a `.fcproj` v17: külön `external` mező, `v16_to_v17` migráció. |
| V-7 | A `mi.external_ref` származtatott tábla a mi sémában, csak a PG-4 után. |
| V-8 | Az S0 a PG-től függetlenül indulhat. |
| V-9 | Videóletöltés URL-ről csak ott, ahol a platform ÁSZF-je kifejezetten engedi. Szolgáltatónkénti adatfájlban az ÁSZF-forrással és dátummal; ellenőrzés az F2 jogi kapuban; csak a `cfa4-fetch`-en és a K18-on át; alapból kikapcsolva; jóváhagyó ablakkal (forrás, méret, licenc); hash-ellenőrzéssel; soha nem automatikusan (FFMPEG-R1:109, 181). A motor csak a helyi fájlt kapja meg. ÁSZF-engedély nélkül (pl. YouTube) nincs letöltés. |
| V-10 | A hét tiltott kategória bekerül a tervbe: vízjel-eltávolítás, nézettség-generálás, cookie-alapú hozzáférés, residential proxy, email- és telefonszám-gyűjtés, felnőtt tartalom, kommentek tömeges gyűjtése. |
| V-11 | A közös letöltő a K18-ra épül, szolgáltatóleíró adatfájllal. |
| V-12 | A speech `ureq`-letöltés átköltözése a `cfa4-fetch`-be az S1 előfeltétele. |
| V-13 | A `cfa4-fetch` programszinten kikényszerített host-engedélylistát kap. |
| V-14 | A project-files.md csúszása (v13 a v16 helyett) külön dokumentációs javítás; a terv csak említi. |

## 12. Nyitott pontok (tények, nem döntések)

- **[nyitott]** A CFA4-MEDIA-INTELLIGENCE-001 FINAL 2.0 szövege nincs a tárolókban (AI-MODEL-TRAINING-001.md:22).
- **[nyitott]** Mely platformok ÁSZF-je engedi kifejezetten az oEmbed-lekérést vagy a letöltést. Ez az F2 kimenete.
- **[nyitott]** A Hugging Face letöltési átirányításainak pontos hostjai. Az F3 állapítja meg (5.2).
- **[nyitott]** Van-e a `.fcproj`-nak stabil projektazonosítója a `mi.project_link`-hez (DATA-PG terv:241).
- **Lezárva (2026-11-15):** a CFA4-DCPOMATIC-001-FINAL-1.0 terv (134 sor) és a hozzá kapcsolódó, folyamatot, sandboxot, hálózatot vagy letöltést érintő dokumentumok teljesen elolvasva (13. fejezet). Ütközés nincs. A minta átvéve (5.1), az eltérések indokkal szerepelnek (5.1, 3., 4., 7. és 8. pont).
- **Lezárva (2026-11-15, későbbi olvasás):** a korábban csak részben olvasott négy fájl teljesen elolvasva a `ce8057bb` `main`-en, változtatás nélkül: `crates/dcpomatic/src/lib.rs` (285 sor), `crates/dcpomatic/src/request.rs` (416 sor), `crates/dcpomatic/tests/real_dcpomatic.rs` (66 sor), `docs/agents.md` (383 sor, nemcsak a 7. szakasz). A 5.1, 5.3, 5.4, 7., 9. és 13. fejezet ennek megfelelően pontosítva. Ütközés nincs.
- **[nyitott]** A donor „production-ready” állítása (donor README.md:92, vö. 160) nem ellenőrizhető. A terv nem használja.

## 13. Forrásjegyzék

**Tárak:**
- CFA4 `origin/main` `ce8057bb53fdee862a91633608af58d97a5a71c4` (a `git fetch` után változatlan)
- CFA4-AI_modules `fc8b7335033e3717e1793b9e3537ee19704b6e6c`
- donor `25f6cb311249d133bbd4dca99568aac457349317`
- DATA-PG-001 terv: #1386, head `553cfbd` (NYITOTT, 643 sor)

**CFA4 @`ce8057bb`:**
- VISION.md:37–39, 47–49, 56–57, 154, 184, 186
- docs/adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md:17–19, 36, 90
- docs/adr/ADR-0011-dcpomatic-external-mastering.md:3–5, 37–43, 44–45, 79–80
- docs/plans/CFA4-SECURITY-001.md:82, 120–122, 125
- docs/security/threat-model.md:50, 57
- crates/sandbox/README.md:6, 14–15, 22–31
- crates/sandbox/src/profile.rs:21–22, 123–125, 234–238
- crates/sandbox/src/tests.rs:49–54
- crates/speech/src/models.rs:3–6, 248, 264–312, 315–328, 340
- crates/project/src/lib.rs:50–62, 304–320, 1277–1317
- crates/project/src/studio.rs:13, 18–36, 236–275
- crates/project/src/transcript.rs:68–76
- crates/format/src/lib.rs:64–70, 401–403, 410–412
- docs/project-files.md:55, 57–73, 226–231
- docs/captions.md:24–25, 65
- docs/transcripts.md:26
- docs/licensing.md:12–13, 15–20, 102, 108–111, 113–132
- docs/plans/CFA4-FILE-SERVICES-001-1.0.md:27, 60–67, 116
- docs/plans/CFA4-AI-MODEL-TRAINING-001.md:22, 196, 334, 388–389
- docs/plans/SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md:220–222, 253
- docs/plans/CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md:129, 288, 324–326, 585, 631
- docs/plans/CFA4-LIVE-STUDIO-001.md:104–106
- docs/plans/CFA4-CINECERT-001-v2.3.md:189, 298, 303, 311, 316
- docs/credits-plan.md:330–331
- docs/plans/CFA4-TITLE-CREDITS-001-1.0-CFA4-ILLESZTES.md:246, 275, 292
- docs/plans/CFA4-FFMPEG-BACKEND-R1.md:109, 181, 250–251
- docs/control-protocol.md:4–19, 21, 65, 200
- docs/qt-gui.md:64–70, 77, 187
- xtask/src/main.rs:97–143
- apps/cfa4/src/appearance.rs:16–18
- docs/plans/CFA4-DCPOMATIC-001-FINAL-1.0.md:4, 6, 42, 49, 65, 71–78, 130 (teljesen olvasva, 134 sor)
- crates/dcpomatic/src/process.rs:1–3, 23–26, 28–68, 70–84, 99–132, 168–173 (teljesen olvasva, 236 sor)
- crates/dcpomatic/src/lib.rs:9, 41–50, 85–87, 170, 173–197, 199–204, 216–221, 225–241 (teljesen olvasva, 285 sor)
- crates/dcpomatic/src/request.rs:382–383 (teljesen olvasva, 416 sor)
- crates/dcpomatic/tests/real_dcpomatic.rs:38, 51–58 (teljesen olvasva, 66 sor)
- docs/agents.md:10–19, 85–88, 104–107, 119–139, 156–160, 360–372, 373–383 (teljesen olvasva, 383 sor)
- crates/dcpomatic/README.md:44–56, 60–63 (teljesen olvasva, 65 sor)
- crates/dcp-inspect/README.md:51–61, 108–116 (teljesen olvasva, 135 sor)
- docs/audit/DCPOM-01-DCPOMATIC-SURVEY-2026-11-13.md:99, 110 (teljesen olvasva, 121 sor)
- docs/development/dcpomatic.md:10–15 (teljesen olvasva, 66 sor)
- docs/plans/DCP-VALIDATION-2026-10-27.md:53, 443–457, 477–482, 526–532 (teljesen olvasva, 715 sor)
- docs/audit/DCP-00-CAPABILITY-AUDIT-2026-10-27.md:138–139 (teljesen olvasva, 272 sor; a fixture-ök rögzített commit és SHA-256 szerint töltődnek le, ez fejlesztői, nem termékbeli letöltés)

**AI_modules @`fc8b7335`:** docs ADR-0008:117–119, 331–332.

**DATA-PG terv (#1386 @`553cfbd`):** 48, 61, 78–79, 185, 232, 240–243, 264–272, 386, 400, 435, 442, 561.

**Donor @`25f6cb31`:**
- settings/APIFY_ACTORS.md:29076–30361 (a VIDEOS szakasz teljesen bejárva, 979 tétel). A többi kategóriát nem néztem át, mert a terv csak videós forrásokról szól.
- fetch_apify_actors.js:11
- README.md:92, 160, 167, 170

**Külső források (olvasva 2026-11-15):**
- oEmbed-specifikáció: https://oembed.com/ (§2.1, §2.3.1, §2.3.4, §3)
- YouTube Terms of Service: https://www.youtube.com/static?template=terms (hatályos 2023-12-15)
