# D1: döntési jegyzőkönyv sablon és pontozási módszer (VÁZLAT)

> **Állapot: vázlat a kapu-jóváhagyónak.** A terv D1 rubrikáját (§7, D1) nem módosítja: a kapuk és a súlyok (mért előny 30%, karbantartási költség 20%,
> integrációs felület 20%, upstream egészség 15%, host-lábnyom 15%) a tervből valók. Ez a sablon azt rögzíti, **hogyan** kell a "karbantartási költség"
> (kockázatnapló: "Vendorizálás karbantartási terhe → D1 rubrika karbantartási súlya") és a többi pontozott szempont számát előállítani, hogy a D1
> jegyzőkönyv jelöltenként összehasonlítható és utólag ellenőrizhető legyen. Mért adat még nincs; a sablon üres.

## 1. Kapuk (bármelyik FAIL megállít, pontozás nem váltja ki)

| Kapu | Bizonyíték | Eredmény |
|---|---|---|
| MQ-1–MQ-8 PASS | F2 mérés, `F2-poc-comparison.md` | PASS / FAIL / NEM MÉRT (a NEM MÉRT nem PASS) |
| Helyesség a baseline-hoz (AC-04, AC-05) | F0V harness riport | PASS / FAIL / NEM MÉRT |
| Licenc és provenance (A9) | F1 audit sor | PASS / BLOCKED |
| A11 Python | nyilvántartás | nincs új verzió / indoklás és jóváhagyás kell |

## 2. Pontozott szempontok (0–5, a súllyal szorozva)

| Szempont | Súly | A szám előállítása | Bizonyíték |
|---|---|---|---|
| Mért előny | 30% | az előregisztrált metrikák közül a legjobb, ahol a bootstrap 95% CI alsó határa a küszöb fölött van: 5 = több mint 2× a küszöb, 3 = a küszöböt éppen meghaladja, 0 = nincs előny | F2 |
| Karbantartási költség | 20% | éves becsült munkanap (lásd 3.); 5 = ≤ 2 nap/év, 4 = ≤ 5, 3 = ≤ 10, 2 = ≤ 20, 1 = ≤ 40, 0 = több | 3. pont |
| Integrációs felület | 20% | az érintett C0 szerződések és konverziós mátrix sorainak száma; 5 = nincs új szerződés, 0 = a Frame ABI törő változása kell | C0 spec |
| Upstream egészség | 15% | F1: kiadások 12 hónapban, aktív karbantartók (küszöb: ≥ 1 kiadás és ≥ 2 karbantartó); 5 = teljesíti és több mint kétszeresen, 0 = nem teljesíti | F1 audit |
| Host-lábnyom (A6) | 15% | rendszercsomagok száma és izolálhatóság (külön folyamat, sandbox); 5 = nincs új csomag, 0 = rendszerszintű módosítás | F1 host-footprint |

A súlyozott összeg nem dönt egyedül: a döntés a kapuk után az összeg és a tételes indoklás alapján a kapu-jóváhagyóé. Ha két jelölt között az összeg különbsége
kisebb, mint 0,5 pont (az 5 pontos skálán), a döntés nem vezethető le a számokból, és a jegyzőkönyv ezt kimondja.

## 3. Karbantartási költség becslése (éves munkanap)

A becslés tételei, mindegyik indoklással és forrással (nem tipp):

| Tétel | Mérés vagy forrás |
|---|---|
| Verziófrissítés (upstream kiadások) | kiadások száma az elmúlt 12 hónapban × az utolsó frissítés ismert vagy próbált munkaideje |
| Biztonsági javítások | a függőség RUSTSEC/CVE találatai 12 hónapban; a `cargo deny` riport |
| ABI/API törés | a `cargo semver-checks` vagy kézi összevetés két egymást követő kiadás közt |
| Build és csomagolás | a build igénye (pl. külön fordítás, rendszerkönyvtár); F1 host-footprint |
| Licenc-újraellenőrzés | kiadásonként a licencmátrix frissítése |
| Saját hibajavítás (fork, ha van) | a CFA4-be vendorozott kód sora × a nyitott hibák aránya |

A becslés tartománya (min, várható, max) szerepel, az 5 pontos skálán a **várható** érték számít, a max a kockázati megjegyzésbe kerül. Ha egy tétel nem becsülhető, `NEM BECSÜLHETŐ`, és a szempont nem kap pontot (nem 0).

## 4. Jegyzőkönyv jelöltenként (kitöltendő)

```
Jelölt:
Döntés: INTEGRATE_COMPONENT | ADAPT_PATTERN_ONLY | EXTERNAL_ADAPTER | RESEARCH_ONLY | REJECT
Kapuk: MQ ... | Helyesség ... | Licenc ... | A11 ...
Pontok: mért előny _ | karbantartás _ (várható _ nap/év, tartomány _–_) | felület _ | egészség _ | lábnyom _
Súlyozott összeg:
Indoklás (3 mondat):
Hivatkozott bizonyíték (fájl, commit):
Aláírás (kapu-jóváhagyó), dátum:
```

## 5. Nyitott

1. A pontozási sávok (2. és 3. fejezet) a jóváhagyóé; ez javaslat.
2. A `cargo semver-checks` használata új eszköz a repóban; ha nem kívánatos, kézi összevetés marad.
