# CFA4-GREENSCREEN-MATTING-INTEGRATION-001 – Z0: a kulcsolók mérése szintetikus zöldhátteren

**Dátum:** 2026-11-24 · **Terv:** [`CFA4-GREENSCREEN-MATTING-INTEGRATION-001-0.1`](../plans/CFA4-GREENSCREEN-MATTING-INTEGRATION-001-0.1.md) 7. pont, Z0 ·
**Forrás:** `main` @ `19ad4e24d` · **Jelleg:** csak mérés; terméket nem módosít (új teszt: `crates/render/src/key_measure_tests.rs`).

## 1. Mit mértünk és mit nem

Az Ultra Key (Default / Relaxed / Aggressive beállítás) és a Color Key mattahibája **saját, procedurálisan generált** képeken, amelyeknek a valódi mattája ismert.
A generátor a repóban van (`crates/render/src/key_synth.rs`, csak tesztfordításban): képletekből és rögzített magú generátorból dolgozik, **külső felvétel nincs, ezért licenckérdés sincs**; minden futás ugyanazokat a pixeleket adja.
Tulajdonosi döntés (2026-11-24): ha valamire nincs licenc, saját kóddal oldjuk meg.

Jelenetek: `disc` (lágy szélű korong, erős zöld szóródás a peremén), `uneven` (a háttér fényereje ±25 %), `noisy` (2 % szenzorzaj, képkockánként más), `hair` (36 hullámos, élsimított szál, 0,6–1,6 px széles, zöld szóródással), `smoke` (áttetsző folt, valódi alfa legfeljebb 0,6), `motion` (a korong és a szálak mozognak, zajjal, 12 képkocka 24 fps-en).
Reprodukálás: `cargo test -p cfa4-render --lib key_ -- --nocapture`.

**Minden szám SZINTETIKUS.** A képek a kulcsolók egymáshoz viszonyított viselkedését mutatják, nem a kamerás felvételen elérhető minőséget. A mérés nem idő- és sebességmérés, és nem vizsgálja a Smart Render / stream copy megkerülését (lásd 4. pont).

## 2. Eredmények

Mértékek: MAE = átlagos alfa-hiba az egész képen; edgeMAE = a lágy szélű sávban; leak = a valódi háttérben maradó alfa; hole = a valódi előtérben hiányzó alfa; spill = a zöld túlsúly az átlátszatlan előtérben (G − max(R,B), átlag).

| Jelenet | Kulcsoló | MAE | edgeMAE | leak | hole | spill |
|---|---|---|---|---|---|---|
| disc | Ultra Default | 0,0785 | 0,4179 | 0,0000 | 0,2548 | 0,0309 |
| disc | Ultra Relaxed | 0,0761 | 0,3958 | 0,0000 | 0,2499 | 0,0397 |
| disc | Ultra Aggressive | 0,1060 | 0,4966 | 0,0000 | 0,3647 | 0,0109 |
| disc | Color Key (tol. 40, feather 10) | 0,0141 | 0,2250 | 0,0000 | 0,0000 | 0,0916 |
| uneven | Ultra Default | 0,0932 | 0,4168 | 0,0203 | 0,2548 | 0,0309 |
| uneven | Ultra Relaxed | 0,0920 | 0,3939 | 0,0218 | 0,2499 | 0,0397 |
| uneven | Ultra Aggressive | 0,1060 | 0,4966 | 0,0000 | 0,3647 | 0,0109 |
| uneven | Color Key | 0,0190 | 0,2241 | 0,0068 | 0,0000 | 0,0916 |
| noisy | Ultra Default | 0,0781 | 0,4144 | 0,0000 | 0,2542 | 0,0277 |
| noisy | Ultra Relaxed | 0,0759 | 0,3939 | 0,0000 | 0,2497 | 0,0397 |
| noisy | Ultra Aggressive | 0,1058 | 0,4965 | 0,0000 | 0,3636 | 0,0109 |
| noisy | Color Key | 0,0141 | 0,2255 | 0,0000 | 0,0000 | 0,0919 |
| hair | Ultra Default | 0,0927 | 0,4550 | 0,0000 | 0,5549 | 0,0000 |
| hair | Ultra Relaxed | 0,1014 | 0,4396 | 0,0051 | 0,8600 | 0,0000 |
| hair | Ultra Aggressive | 0,1151 | 0,5225 | 0,0000 | 1,0000 | 0,0000 |
| hair | Color Key | 0,0430 | 0,2461 | 0,0000 | 0,0000 | 0,3224 |
| smoke | Ultra Default | 0,1080 | 0,1634 | 0,0000 | 0,0000 | 0,0000 |
| smoke | Ultra Relaxed | 0,0963 | 0,1454 | 0,0000 | 0,0000 | 0,0000 |
| smoke | Ultra Aggressive | 0,1170 | 0,1773 | 0,0000 | 0,0000 | 0,0000 |
| smoke | Color Key | 0,1139 | 0,1725 | 0,0000 | 0,0000 | 0,0000 |

Mozgás és villogás (`motion`, 12 képkocka; a villogás a matte átlagos abszolút változása két képkocka között, a valódi matte változása ugyanezen mérték szerint 0,1223):

| Kulcsoló | MAE | átlagos \|ΔAlfa\| |
|---|---|---|
| Ultra Default | 0,1217 | 0,0431 |
| Ultra Relaxed | 0,1325 | 0,0193 |
| Ultra Aggressive | 0,1703 | 0,0000 |
| Color Key | 0,0428 | 0,1109 |

## 3. Megfigyelések (a terv döntéseihez)

1. **A Color Key az erős zöld szóródású előtérben kisebb mattahibát ad, de nem szűri a szóródást** (spill 0,09–0,32). Az Ultra Key szűri (0,01–0,04), viszont az erősen szóródott előteret részben átlátszóvá teszi (hole 0,25–0,36 a koronggal). A kettő ellentétes irányba téved, ezért a módszerválasztás (S17) felhasználói döntés marad.
2. **Haj-szerű, élsimított szálakon az Ultra Key gyenge** (hole 0,55–1,00, az Aggressive mindet eltünteti); a Color Key itt is kisebb mattahibájú (0,043), de szóródással (0,32). A lágy szélen mindkettő hibája nagy (edgeMAE 0,22–0,52). Ez az AI-matte (`ai.matting.*`) indoka, szintetikus alapon.
3. **Áttetsző folton (smoke) mindkét kulcsoló hibája hasonló** (MAE 0,096–0,117): a kulcsolók az áttetszést nem becsülik jól.
4. **Egyenetlen megvilágítás** az Ultra Keynél hátteret hagy az előtérben (leak ≈ 0,02), a Color Keynél kisebbet (0,007). A zaj (2 %) alig változtat a matte hibáján.
5. **Villogás:** a Color Key mattája képkockánként a valódi változásnál kisebb mértékben mozog (0,111 a 0,122 ellenében), az Ultra Defaulté 0,043, a Relaxedé 0,019. Az Aggressive 0,000, mert a mozgó jelenetben a mattája teljesen eltűnik (MAE 0,170): ez a hiba, nem a stabilitás jele. Önálló időbeli simítást ez nem indokol; a Z2 „opcionális időbeli simítás” sora mért igény hiányában marad.
6. **Az Ultra Key Setting (Default / Relaxed / Aggressive) felülírja a hozzá tartozó paraméterek kézi értékét**; csak a Custom hagyja meg őket (`ultra_key_setting`, `vfx/color.rs`). A felületen ezt jelezni kell (Z2 hiánylista-sor), különben a csúszkák hatástalannak látszanak.
7. A mérés a Z1a (Color Key Edge Thin, #1794) után készült; az Edge Thint itt nem használtuk (`thin` = 0).

## 4. Hátralévő a Z0-ból

Kamerás felvételen nincs mérés: tulajdonosi döntés szerint licenc nélkül saját kóddal dolgozunk, a generátor ezt pótolja; a tulajdonos saját felvételei a referenciagépes méréshez később is használhatók (a `deferred-physical-tests.md` sora). Nyitva maradt: a kulcsolt klipek útvonala (megkerüli-e a Smart Rendert / stream copyt), saját sorok a G12/G18/G-xx mátrixba, idő- és sebességmérés az F0V-harnessen.
