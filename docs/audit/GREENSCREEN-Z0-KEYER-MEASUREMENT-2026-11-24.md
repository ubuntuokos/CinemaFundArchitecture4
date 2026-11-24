# CFA4-GREENSCREEN-MATTING-INTEGRATION-001 – Z0: a kulcsolók mérése szintetikus zöldhátteren

**Dátum:** 2026-11-24 · **Terv:** [`CFA4-GREENSCREEN-MATTING-INTEGRATION-001-0.1`](../plans/CFA4-GREENSCREEN-MATTING-INTEGRATION-001-0.1.md) 7. pont, Z0 ·
**Forrás:** `main` @ `19ad4e24d` · **Jelleg:** csak mérés; terméket nem módosít (új teszt: `crates/render/src/key_measure_tests.rs`).

## 1. Mit mértünk és mit nem

Az Ultra Key (Default / Relaxed / Aggressive beállítás) és a Color Key ismert mattát állít-e vissza **szintetikus** zöldhátterű képeken.
A kép: zöld háttér, benne lágy szélű korong (6 px) vagy 1 px-es függőleges csíkok (haj), a korongon zöld szóródás (spill, a szélén erősebb).
Négy eset: `disc` (egyenletes háttér), `uneven` (a háttér fényereje ±25 %), `noisy` (2 % szenzorzaj), `hair`.
Reprodukálás: `cargo test -p cfa4-render --lib key_measure -- --nocapture`.

**Nem mértük:** valódi, nyílt licencű felvételeket (a terv Z0 szerint ezek is kellenek), idő- és sebességmérést, a Smart Render / stream copy megkerülését.
Ezek a [`deferred-physical-tests.md`](../development/deferred-physical-tests.md) új sorában vannak. A szintetikus kép nem helyettesíti a valódi felvételt: a szám a kulcsolók viszonyát mutatja, nem a gyakorlati minőséget.

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
| noisy | Ultra Default | 0,0782 | 0,4131 | 0,0000 | 0,2546 | 0,0275 |
| noisy | Ultra Relaxed | 0,0759 | 0,3926 | 0,0000 | 0,2499 | 0,0398 |
| noisy | Ultra Aggressive | 0,1059 | 0,4964 | 0,0000 | 0,3640 | 0,0107 |
| noisy | Color Key | 0,0142 | 0,2273 | 0,0000 | 0,0000 | 0,0920 |
| hair | Ultra Default | 0,0472 | 0,0000 | 0,0000 | 0,4531 | 0,0310 |
| hair | Ultra Relaxed | 0,1224 | 0,0000 | 0,0404 | 0,8280 | 0,0000 |
| hair | Ultra Aggressive | 0,1080 | 0,0000 | 0,0085 | 0,9633 | 0,0000 |
| hair | Color Key | 0,0000 | 0,0000 | 0,0000 | 0,0000 | 0,2426 |

## 3. Megfigyelések (a terv döntéseihez)

1. **A Color Key az erős zöld szóródású előtérben kisebb mattahibát ad, de nem szűri a szóródást** (spill 0,09–0,24). Az Ultra Key szűri (0,01–0,04), viszont az erősen szóródott előteret részben átlátszóvá teszi (hole 0,25–0,36). A kettő ellentétes irányba téved, ezért a kulcsolók közötti választás (S17) felhasználói döntés marad.
2. **Az Ultra Key haj-szerű, 1 px-es szerkezeten gyenge** (hole 0,45–0,96), a lágy szélen mindkét kulcsoló hibája nagy (edgeMAE 0,22–0,50). Ez a terv indoka az AI-matte (`ai.matting.*`) mellé, a mérés azonban szintetikus: a számot nem szabad a valódi hajra átvinni.
3. **Egyenetlen megvilágítás** az Ultra Keynél hátteret hagy az előtérben (leak ≈ 0,02), a Color Keynél kisebbet (0,007). A zaj (2 %) alig változtat.
4. **Az Ultra Key Setting (Default / Relaxed / Aggressive) felülírja a hozzá tartozó paraméterek kézi értékét**; csak a Custom hagyja meg őket (`ultra_key_setting`, `vfx/color.rs`). A felületen ezt jelezni kell (Z2 hiánylista-sor), különben a csúszkák hatástalannak látszanak.
5. A mérés a Z1a (Color Key Edge Thin, #1794) után készült; az Edge Thint itt nem használtuk (`thin` = 0).

## 4. Hátralévő a Z0-ból

Valódi, nyílt licencű zöldhátteres felvételek mérése, a kulcsolt klipek útvonalának ellenőrzése (megkerüli-e a Smart Rendert / stream copyt), saját sorok a G12/G18/G-xx mátrixba. Állapot: a `deferred-physical-tests.md` sora.
