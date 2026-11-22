# BacklotHub 7. lépcső – Search: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-11 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.24 · **Döntés:** #44 (saját PostgreSQL-keresés,
`SearchProvider` szerződés; a Universal Search újravizsgálata a 8. lépcső G2-jén) · **Előzmény:**
[G1 Search](CFA4-BACKLOTHUB-G1-07-SEARCH-2026-11-06.md) (funkcionális követelmények jóváhagyva 2026-11-10) ·
**Épül rá:** G2-01…G2-06 (jóváhagyva) · **Állapot:** G2 **jóváhagyva** (István, 2026-11-11) · implementáció nincs

Új döntési kérdés nincs. Új függőség nincs: `unaccent` és `pg_trgm` a PostgreSQL contrib része, ugyanúgy, mint
az `ltree` (G2-03).

## 1. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Modulok → kereső | regisztráció | Minden modul `SearchProvider`-t ad a saját objektumtípusaihoz: típus, mezőleírók (szűrhető mezők), szöveges keresés, találat-összefoglaló. Az alap lépcsőben: produkció, szervezet, szerkezeti elem, feladat, megjegyzés, személy. A 8., 14., 18–26. lépcső a sajátját hozza |
| Felület, CLI, MCP | → | `backlot.search.*` parancsok (5. pont) |
| Szűrőnyelv | közös | **Ugyanaz** a nyelv szolgálja ki a `backlot.task.list`-et és a Dashboard lefúrását (G2-05, G2-06): egy fordító, egy mezőregiszter |
| Jogosultság | ← | A produkciós tagság (#42) a lekérdezésben szűr, nem utólag |
| CFA4 Universal Search | később | Ha lesz, a `SearchProvider` szerződésen át csatlakozik; a BacklotHub kódja nem változik (#44) |
| Motor `media.search` | nincs | Más fogalom (offline média újrakapcsolása); a névtér `backlot.search.*`, ütközés nincs |

## 2. Szöveges keresés

- **Szövegkeresési konfigurációk** (migrációban):
  - `backlot_hu`: `unaccent` + `hungarian_stem`;
  - `backlot_en`: `unaccent` + `english_stem`;
  - `backlot_code`: `simple` + `unaccent`, kódokhoz (SQ010, SH0040).
- **Generált `tsvector` oszlop** minden kereshető táblán (`search_vector`, `GENERATED ALWAYS … STORED`), GIN
  indexszel. Mindig friss, mert az üzleti írással egy tranzakcióban számolódik. Súlyok: kód `A`, név/cím `B`,
  leírás/szöveg `C`. A magyar és az angol vektor összefűzve, mert a produkciók nyelve vegyes.
- **Elgépelés-tűrő keresés:** `pg_trgm` GIN-index a kódon és a néven, egy `IMMUTABLE` burkolófüggvénnyel
  (`backlot.f_norm(text)` = kisbetűs, ékezet nélküli), mert az `unaccent` önmagában nem használható indexkifejezésben.
- **Lekérdezés:** a felhasználó szövegéből `websearch_to_tsquery` (idézőjel, `-` kizárás működik), mindkét
  konfigurációval; ha a szöveg rövid (≤ 3 karakter) vagy kódnak látszik, trigram-/előtag-keresés. Rangsor:
  `ts_rank_cd` + trigram-hasonlóság, azonos rangnál `id` szerint (stabil lapozás).
- **Találat:** típus, `id`, produkció, cím, kód, kiemelt részlet (`ts_headline`), szülő-útvonal (elemeknél a
  `path` nevei), státusz.

```sql
-- példa: feladat
ALTER TABLE backlot.task ADD COLUMN search_vector tsvector GENERATED ALWAYS AS (
    setweight(to_tsvector('backlot_hu', coalesce(title,'')), 'B') ||
    setweight(to_tsvector('backlot_en', coalesce(title,'')), 'B') ||
    setweight(to_tsvector('backlot_hu', coalesce(description,'')), 'C') ||
    setweight(to_tsvector('backlot_en', coalesce(description,'')), 'C')
) STORED;
CREATE INDEX ON backlot.task USING gin (search_vector);
CREATE INDEX ON backlot.task USING gin (backlot.f_norm(title) gin_trgm_ops);
```

## 3. Szűrőnyelv (zárt, deklaratív)

```json
{"type": "task",
 "where": {"all": [
   {"field": "status_category", "op": "in", "value": ["active", "planned"]},
   {"field": "entity_path", "op": "under", "value": "<elem-id>"},
   {"any": [{"field": "due_date", "op": "lt", "value": "today"},
            {"field": "priority", "op": "eq", "value": "urgent"}]}]},
 "text": "kompozit",
 "sort": [{"field": "due_date", "dir": "asc"}],
 "page": {"after": null, "limit": 50}}
```

- **Mezőregiszter:** a provider adja mezőnként a nevet, a típust (`text`, `date`, `int`, `enum`, `uuid`,
  `path`), az engedett operátorokat és az SQL-kifejezést. A felhasználói bemenet **csak paraméterként** kerül
  az SQL-be; mezőnév és operátor csak a regiszterből.
- Operátorok: `eq`, `ne`, `in`, `lt`, `le`, `gt`, `ge`, `between`, `contains` (trigram), `is_null`, `under`
  (`ltree <@`), `linked_to` (`entity_link`), relatív dátumok (`today`, `today+7d`).
- Korlátok: legfeljebb 5 szint mélység és 50 feltétel (`SEARCH_FILTER_TOO_COMPLEX`); lekérdezési időkorlát
  (`statement_timeout`, alapból 5 s) → `SEARCH_TIMEOUT`, összeomlás nincs.

## 4. Adatmodell

```sql
CREATE TABLE backlot.saved_search (
    id            uuid PRIMARY KEY,
    person_id     uuid NOT NULL REFERENCES backlot.person(id),
    production_id uuid REFERENCES backlot.production(id),   -- NULL: minden produkción
    name          text NOT NULL,
    query         jsonb NOT NULL,             -- a 3. pont szerinti objektum, sémaverzióval
    shared        boolean NOT NULL DEFAULT false,
    ...közös oszlopok production_id nélkül,
    UNIQUE (person_id, production_id, name)
);
```

- Mentéskor a lekérdezés validálódik. Ha később egy mező megszűnik, a mentett keresés betöltéskor
  `SEARCH_FIELD_UNKNOWN` jelzést ad, és szerkeszthető marad (nem törlődik csendben).
- A mentett keresés nem üzleti adat, ezért nem auditált (mint a dashboard-nézet, G2-06); a megosztott keresés
  létrehozása és törlése viszont igen, mert mások is látják.

## 5. Parancsok

| Parancs | Mit csinál |
|---|---|
| `backlot.search.query` | Szabad szöveg és/vagy szűrő, egy vagy több típusra; típusonkénti darabszám (facet) is |
| `backlot.search.types` | Kereshető típusok és mezőik (a felület szűrőszerkesztőjéhez) |
| `backlot.search.related` | Kapcsolódó objektumok: szülő, gyerekek, linkek (`entity_link`), feladatok, megjegyzések |
| `backlot.search.saved.list` / `.save` / `.delete` / `.run` | Mentett keresések |

## 6. Jogosultság

A lekérdezés mindig tartalmazza: `production_id IN (SELECT production_id FROM backlot.production_member WHERE
actor = :actor)`. M1-ben a helyi felhasználó minden produkció tagja (#42), a feltétel így is ott van, hogy M2-ben
ne kelljen hozzányúlni. Produkción kívüli közös objektumok (szervezet, személy) M1-ben mindenkinek láthatók; M2-ben
a szerepkör szerint (Identity).

## 7. Méret és tesztek (G5-re előre)

- **L-profil:** ≈ 620 000 elem, 1,9 millió feladat, megjegyzések; szabad szöveges keresés, kódkeresés és
  összetett szűrő válaszideje a CFA4-MIN gépen (SC-02). Ha a közös keresés minden típuson lassú, típusonként
  párhuzamos lekérdezés, majd összefésülés.
- Magyar és angol szövegek, ékezettel és nélküle („kompozitalas” = „kompozitálás”), elgépelés („kompzit”).
- Szűrőnyelv-fuzz: véletlen és rosszindulatú bemenet (SQL-töredék, mély fa, ismeretlen mező) → hibakód, nincs
  nyers SQL, nincs összeomlás.
- A dashboard-csempe száma = a lefúrás találatszáma = a `backlot.task.list` elemszáma (egy fordító).

## 8. Hibakódok

`SEARCH_QUERY_INVALID`, `SEARCH_FIELD_UNKNOWN`, `SEARCH_OPERATOR_INVALID`, `SEARCH_FILTER_TOO_COMPLEX`,
`SEARCH_TIMEOUT`, `SEARCH_TYPE_UNKNOWN`.

## 9. Licenc (G3-ra előre)

Új függőség nincs (`unaccent`, `pg_trgm`: PostgreSQL License, contrib). Kitsu (AGPL): mentett szűrő mint
minta, kód nem; Meilisearch elvetve (G1).

## 10. G2-eredmény

**Javaslat: PASS.** A CFA4 Universal Search kérdése a 8. lépcső G2-jén kerül újra elő (#44).
