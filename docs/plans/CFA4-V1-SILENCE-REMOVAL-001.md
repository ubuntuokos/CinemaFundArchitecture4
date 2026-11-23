# CFA4-V1-SILENCE-REMOVAL-001: a csend-eltávolítás (V1, F6) megvalósítási terve

**Állapot:** **jóváhagyva (István, 2026-11-21)**, a D1–D5 döntéseknél a terv javaslatai érvényesek: **D1** az energiaalapú V1-A **nem MI-funkció**, ezért az MI-réteg előtt is indulhat; **D2** a nem romboló mód csak jelölés; **D3** kötegnél metszet-mód ripple-lel, klipenkénti csak lifttel; **D4** a régió marker (nincs sémaváltozás); **D5** az alap-paraméterek `minSilence` 0,5 s, `padding` 0,1 s, mérés alapján hangolva. A **V1-A megvalósítása indulhat** (külön ágon és PR-ban, Ready for review, Gazda issue-ban jelezve, merge nélkül); a **V1-B (Silero) az MI-réteg után** jön. A jóváhagyás a V1-A-ra szól, a V1-B külön indítási engedélyt kér az MI-réteg üzembe helyezése után. **A V1-B az MI-réteg zárási feltételei közé tartozik** (István, 2026-11-21; 12. pont). A dokumentum maga csak tervet tartalmaz, kódot nem.
**Dátum:** 2026-11-21 · **Alap:** `main` @ `fc310bd` · **Módszer:** csak olvasás: a `docs/venturi-parity.md` V1 szakasza, a `crates/speech/src/vad.rs`, a `crates/engine/src/transcript.rs`, a `crates/edit/src/transcript.rs`, a `crates/engine/src/beat_markers.rs` és a `crates/project/src/lib.rs` (`Marker`, `MarkerKind`) áttekintése. A kódot nem futtattam.
**Nyelv:** magyar.
**Forrás és előzmények:** [`docs/venturi-parity.md`](../venturi-parity.md) V1 (a Venturi „Silence removal” funkciója és a V1 5. pontja, az elfogadási feltételek bővítése), [`CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001`](CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001.md) 5.2 (a tulajdonos 2026-11-20-i döntései), a tulajdonos 2026-11-21-i jóváhagyása az elfogadási teszt nevéről (`v1_silence_removal_acceptance`).

---

## 0. Röviden

A cél: **átirat nélkül, bármilyen nyelven** megtalálni egy klip (vagy több klip) csendes szakaszait, **megmutatni** őket, hagyni a felhasználót **szerkeszteni a kijelölést**, és csak azután, **a felhasználó kérésére** törölni őket. Semmi nem törlődik automatikusan.

A tulajdonos döntései (2026-11-20, 2026-11-21), amelyek ennek a tervnek a követelményei:

1. **Régióelőnézet:** az észlelt régiók előnézete, markerként is megjelenítve, mielőtt bármi törlődne.
2. **Szerkeszthető kijelölés:** a régiók törlés előtt szerkeszthetők és kijelölhetők; nincs automatikus romboló törlés.
3. **Kötegelt elemzés:** több klip elemzése egy lépésben.
4. **Opcionális ripple-törlés.**
5. **Nem romboló mód:** a régiók törlés nélkül, jelölésként maradnak.
6. **Elfogadási teszt:** `v1_silence_removal_acceptance` (hangszinkron megmarad, undo/redo, render-ellenőrzés).

A terv két szállításra bontja a munkát, mert a tulajdonos 2026-10-27-i szabálya szerint **minden MI-funkció csak az MI-réteg üzembe helyezése után készül** (`docs/venturi-parity.md` 5. pont):

- **V1-A (nem MI, az MI-réteg nélkül is indítható):** a régiókezelés teljes munkafolyamata (előnézet, szerkesztés, köteg, ripple, nem romboló mód, teszt, Qt6 felület) a **már meglévő, energiaalapú** csendérzékelőre építve (`crates/speech/src/vad.rs`: 10 ms-os keretek, zajpadlóhoz és beszédszinthez igazított küszöb). Ez nem neurális modell. **A tulajdonosnak kell eldöntenie (D1), hogy ez MI-funkciónak számít-e**; ha igen, a V1-A is az MI-réteg után indul.
- **V1-B (MI, az MI-réteg után):** a Silero VAD neurális modell (`tract-onnx`) mint második észlelő; ugyanaz a parancsfelület, csak a `detector` paraméter új értéke.

A V1-B nélkül a V1 **nem teljes** a `docs/venturi-parity.md` szerint; ez a terv ezt nem leplezi: a V1-A az elfogadási feltételek nagy részét teljesíti, de a „beszéd elkülönítése a kattogástól és a teremzajtól” csak a V1-B-vel.

---

## 1. A CFA4 ma (a kód alapján)

| Mi | Hol | Mit tud | Mit nem |
|---|---|---|---|
| Energiaalapú csendérzékelés | `crates/speech/src/vad.rs` (`frame_db`, `threshold`, `tighten_words`) | 10 ms-os keretek szintje, küszöb a zajpadló (10. percentilis, −100 dBFS felett) és a beszédszint (95. percentilis) között, legalább 6 dB-lel a padló felett | Csak a Whisper szóhatárait **szűkíti**; önálló régiólistát nem ad, átirat nélkül nem hívható |
| Szünettörlés | `transcript.removePauses` (`crates/engine/src/transcript.rs`) → `remove_ranges` → `tx::ripple_delete_ranges` (`crates/edit/src/transcript.rs`) | A tartományokat **minden nem zárolt sávon** jobbról balra ripple-törli (a kötött kép és hang együtt marad), egy undo-lépés | **Átirat kell hozzá** (`has_transcript`); nincs előnézet, nincs szerkeszthető kijelölés, nincs nem romboló mód, nincs köteg |
| Markerek | `Marker { start, duration, name, comment, kind, color }`, `MarkerKind::{Comment, Chapter, Segmentation, …}` | Hosszúságú (tartomány-)marker is lehet | – |
| Marker-minta | `markers.addBeats` / `markers.clearBeats` (`crates/engine/src/beat_markers.rs`) | Egy klipből generált markerek **megjegyzés-előtaggal** megjelölve, újragenerálás cseréli őket, egy undo-lépés, más markerhez nem nyúl | – |
| Klip kikapcsolása | `clip.enable` | A klip kikapcsolható törlés nélkül | – |
| Vágás | `timeline.razor`, `sequence.lift`, `sequence.extract` | Meglévő szerkesztőparancsok | – |

**Következmény:** a régiókezelés **nem új szerkesztési alapművelet**, hanem új észlelő + a meglévő markerek és a meglévő `ripple_delete_ranges` újrahasznosítása. Ez adja a becslés zömét és a kockázat kis részét.

---

## 2. A követelmények (R1–R12)

| # | Követelmény | Honnan |
|---|---|---|
| R1 | Csendes régiók észlelése **átirat nélkül**, nyelvtől függetlenül | V1 2. pont |
| R2 | Paraméterek **`Tick`-ben** (nem lebegőpontos másodpercben a belső ábrázolásban): küszöb, minimális szünet, megtartott ráhagyás | V1 2. pont, AGENTS.md (pontos idő = `Tick`) |
| R3 | **Régióelőnézet:** az észlelés **nem módosít** idővonalat, csak **jelöl**: a régiók tartomány-markerek (és a Qt6 felület listája) | V1 5. pont |
| R4 | **Szerkeszthető kijelölés:** minden régió külön kijelölhető/kivehető, határai mozgathatók, a régió törölhető a listából; a törlés **csak a kijelölt, megtartásra nem jelölt** régiókra hat | V1 5. pont |
| R5 | **Nincs automatikus romboló törlés:** a törlés külön, kifejezett parancs; az észlelő parancs sosem töröl | V1 5. pont |
| R6 | **Kötegelt elemzés:** több klip egy lépésben (kijelölt klipek, egy sáv, a teljes sorozat hangsávjai), egy undo-lépéssel és megszakítható munkával | V1 5. pont |
| R7 | **Opcionális ripple-törlés:** `ripple` igaz (alap) → a hézag bezárul; hamis → a törölt rész üres marad (lift) | V1 5. pont |
| R8 | **Nem romboló mód:** a régiók **jelölésként maradnak**, nem törlődik semmi; a jelölések később ugyanazzal a paranccsal törölhetők | V1 5. pont |
| R9 | **Hangszinkron megmarad:** a kötött kép és hang együtt vágódik, a sorozat hossza a törölt régiók összegével rövidül | elfogadási teszt (a) |
| R10 | **Undo/redo:** a törlés (és a kötegelt futás) egy-egy lépés; undo után a sorozat azonos az eredetivel | elfogadási teszt (b) |
| R11 | **Render:** a törlés előtti és utáni kimenet a megmaradt részeken azonos; a törölt régiókban nincs hang | elfogadási teszt (c) |
| R12 | **Soha nem omlik össze** (AGENTS.md §0): hiányzó hang, üres eredmény, zárolt sáv, sebességváltott/visszafelé játszott klip, óriási klip, megszakítás: mind hibaüzenet vagy üres eredmény, nem pánik | AGENTS.md §0 |

---

## 3. Tervezett parancsfelület

Minden szerkesztés **parancs** a regiszterben (AGENTS.md: „minden parancs”), a Qt6 felület csak ezeket hívja. A nevek javaslatok; a végleges nevet a megvalósítás első lépése rögzíti a parancsregiszterben.

### 3.1 `audio.detectSpeech` (észlelés és előnézet; **nem töröl**)

Paraméterek: `clips: [id]?` (alap: a kijelölt klipek, ha nincs, a célsáv klipjei), `track: id?`, `scope: "selection"|"track"|"sequence"`, `detector: "energy"|"silero"` (alap `energy`; `silero` csak V1-B), `thresholdDb: f?` (alap: az észlelő automatikus küszöbe), `minSilence: ticks?`, `padding: ticks?` (a beszéd két oldalán megtartott ráhagyás), `mode: "preview"|"query"` (alap `preview`).

- `query`: csak visszaadja a régiókat, nem változtat semmit (nincs undo-lépés).
- `preview`: a régiókat **tartomány-markerként** felveszi (R3), megjegyzés-előtaggal (`SILENCE_COMMENT`, a `BEAT_COMMENT` mintájára), egy undo-lépésben; ugyanazon klipek újraelemzése **cseréli** a korábbi csend-markereket, más markerhez nem nyúl.
- Eredmény: `{regions: [{id, start, duration, clip?, keep: false}], clips: n, ticks: total}`.

### 3.2 A régiók szerkesztése (R4)

A régió a marker. A szerkesztés **a meglévő markerparancsokkal** megy (áthelyezés, hosszváltoztatás, törlés), kiegészítve egy **megtartás-jelzővel**:

- `silence.keep {ids: [id], keep: bool}`: a régió megtartásra jelölése (a törlés kihagyja). A jelző a marker megjegyzés-előtagjában él (`[silence:keep]`), tehát a projektbe mentődik, és a meglévő formátumot nem változtatja (nincs új mező).
- `silence.regions {}` (lekérdezés): a csend-markerek listája (id, kezdet, hossz, megtartva-e, melyik klipből).

### 3.3 `clip.removeSilence` (a törlés; **csak kifejezett hívásra**)

Paraméterek: `regions: "markers"|[id]` (alap: az összes, **nem** megtartásra jelölt csend-marker), `ripple: bool` (alap igaz, R7), `mode: "cut"|"mark"` (alap `cut`; `mark` = nem romboló, R8), `keepMarkers: bool` (alap igaz: törlés után a kész régiók markerei eltűnnek).

- `cut`: a kijelölt régiókat a meglévő `ripple_delete_ranges` törli (a kötött kép és hang együtt, R9), `ripple: false` esetén a hézag üres marad (lift). **Egy undo-lépés** (R10).
- `mark`: semmit nem töröl; a régiók markerként maradnak (R8). Ez az alapértelmezett viselkedés az **észlelőnél** (3.1), a törlést **mindig külön** kell kérni (R5).
- A parancs **nem fut**, ha nincs csend-marker (hibaüzenet: „először elemezz”), így sosem töröl „vakon”.

### 3.4 Kötegelt elemzés (R6)

A `scope`/`clips` paraméter több klipet vesz. Két kérdést kell eldönteni (D3):

- **Idővonali tartomány:** egy régió csak akkor törölhető, ha **minden elemzett, ott jelen lévő hangklipben csend van** (metszet). Ez védi a más sávon szóló beszédet. A javasolt alap.
- **Klipenként:** minden klip a saját csendjeit kapja; ripple nélkül (lift) biztonságos, ripple-lel a sávok elcsúszhatnak: ezért ripple-lel csak a metszet-mód engedélyezett.

A kötegelt futás **megszakítható munka** (a meglévő `engine.job` esemény), egyetlen undo-lépéssel, a klipek hangját darabokban olvassa (nincs teljes klip a memóriában).

---

## 4. Az észlelő algoritmusa

### V1-A (energiaalapú, meglévő alapra)

A `vad.rs` keretszint- és küszöbszámítását önálló, régiót visszaadó függvénnyel egészítjük ki (a `tighten_words` változatlan marad): 10 ms-os keretek → küszöb → egymást követő csendes keretek ≥ `minSilence` → régió; a két oldalon `padding` beszédhez tartozó ráhagyás levonva; a régióhatárok a sorozat képkockarácsához igazítva (ahogy a `transcript.removePauses` is a `s.sequence_rate()`-et használja). Az alapértékek (javaslat, mérésre hangolandó): `minSilence` 0,5 s, `padding` 0,1 s. A `transcript.removePauses` alapja (1,0 s / 0,15 s) szándékosan más: az szóközi szünetet töröl, ez csendet.

### V1-B (Silero, az MI-réteg után)

- `crates/speech`: `vad::silero` modul `tract-onnx`-szel a Venturi `vv-vad` alapján (fejléc + nyilvántartás a `licensing.md` §5-ben). **A modellfájl nem kerül a repóba és a binárisba** (tulajdonosi szabály: a CFA4 modellt nem terjeszt): a felhasználó az első használatkor, a licenc (MIT) megmutatása és egyszeri elfogadása után tölti le a közös modelltárba (CFA4-AI_modules ADR 0008, F5), a hash ellenőrzése után. Ha a modell hiányzik, a `detector: "silero"` érthető hibát ad (letöltési felajánlással), az `energy` változatlanul működik. A modellt igénylő tesztek kihagyják magukat, ha a modell nincs a modelltárban (mint a betűtípus-tesztek); a csővezeték (előfeldolgozás, ablakolás, régióképzés) egy generált kis ONNX-gráffal modell nélkül is tesztelhető.
- A modell kimenetét ugyanaz a régióképző lépés fogja össze, mint az energiaalapút.
- Whisper-előszűrő (V1 3. pont): külön tétel a V1-B-ben, **nem** része a csend-eltávolítás parancsainak.
- **Tűrés (V1 4. pont):** a VAD-határok eltérése a generált fixture-hez képest ±1 ablak (32 ms).

---

## 5. A Qt6 felület

A felület a parancsokat hívja, saját logikát nem tartalmaz. Automatizálási azonosítók a meglévő minta szerint.

- **Menü:** Sequence ▸ Remove Silence… (vagy Clip ▸ Remove Silence…, a megvalósításkor a menüszerkezethez igazítva).
- **Párbeszéd „Remove Silence” (nem modális, hogy az idővonal látható maradjon):**
  - Hatókör: Selected clips / Track / Sequence.
  - Észlelő: Energy (V1-B-ben: Speech model).
  - Küszöb (automatikus vagy dB), Minimum silence, Padding (időkód vagy másodperc, belül `Tick`).
  - **Analyze** gomb → `audio.detectSpeech` `preview` → a régiók a listába és markerként az idővonalra kerülnek.
  - **Régiólista:** soronként jelölőnégyzet (törlendő/megtartott), kezdet, hossz, klip; „Play region”, „Select all / none”; a sorra kattintás a lejátszófejet a régióhoz viszi; a határok az idővonali markerként húzhatók.
  - **Ripple** jelölőnégyzet (alap be), **Mode** (Delete / Mark only).
  - **Remove selected** gomb → `clip.removeSilence`; megszakítható folyamatjelző kötegnél.
- A régiómarkerek színe és előtagja egyedi, így elkülönülnek a felhasználó többi markerétől; „Clear silence markers” törli őket.
- Új felirat mind a katalógusnyelveken (az i18n kapu kötelező), a billentyűparancs-kötésből a törlő parancs **kimarad** (a `effects.toggleEnabled` mintájára: romboló parancs nincs egybillentyűs kötésen).

---

## 6. Tesztek és elfogadás

### 6.1 Az elfogadási teszt: `v1_silence_removal_acceptance`

(`crates/speech` + `crates/engine`, generált beszéd+csend fixture a `target/fixtures/`-ben, nem commitolva.) Három ellenőrzés:

- **(a) Hangszinkron megmarad (R9):** a megmaradó klipek kép- és hangoldala azonos időtartományú, a kötött hang nem csúszik el; ripple-törlés után a sorozat hossza a törölt régiók összegével rövidül (`Tick`-pontosan).
- **(b) Undo/redo (R10):** a törlés és a kötegelt futás egy-egy lépés; az undo után a sorozat **bájtra azonos** az eredetivel, a redo után a törlés utánival.
- **(c) Render (R11):** a törlés előtti és utáni kimenet a megmaradt részeken azonos (golden összevetés), a törölt régiókban nincs hang.

### 6.2 További kategóriák

| Kategória | Teszt |
|---|---|
| Funkcionális | régiók a fixture ismert csendjeihez ±1 ablak (32 ms); `minSilence`/`padding` hatása; `query` nem módosít |
| Előnézet (R3) | `preview` csak markert ad, idővonal változatlan; újraelemzés cseréli a markereket, mást nem bánt |
| Szerkeszthető kijelölés (R4) | `silence.keep` kihagyja a régiót a törlésből; áthelyezett/átméretezett marker a törlésben a módosított tartományt adja |
| Nincs automatikus törlés (R5) | `audio.detectSpeech` után a sorozat bájtra azonos; `clip.removeSilence` marker nélkül hibát ad |
| Köteg (R6) | több klip egy undo-lépésben; metszet-mód: a másik sávon szóló beszéd mellett nem töröl; megszakítás után a sorozat változatlan |
| Ripple / lift (R7) | ripple: a hézag bezárul; lift: a hossz változatlan, a hézag üres |
| Nem romboló mód (R8) | `mark`: nincs törlés, a markerek megmaradnak |
| Projektperzisztencia | a markerek és a megtartás-jelző mentés/újratöltés után megvannak |
| Idővonal-integritás | zárolt sáv érintetlen; sync-zár nélküli sáv a megadott módon |
| GUI | a párbeszéd automatizálási azonosítói; valódi motoros teszt: Analyze → lista → jelölés kivétele → Remove selected → undo |
| Hibakezelés (R12) | hang nélküli klip, csak csendes klip (nincs régió), csupa beszéd, sebességváltott / visszafelé játszott klip (a `markers.addBeats` mintájára elutasítva), zárolt sáv, óriási klip (darabolt olvasás), fogadott NaN/negatív paraméter |
| Teljesítmény | hosszú klip (óra) darabolt feldolgozása korlátos memóriával |
| Hardver | CPU-only |

**Nincs kész státusz** puszta fordulás vagy a GUI megjelenése alapján: kell a tesztkimenet (CI-napló) és a kimeneti viselkedés ellenőrzése.

---

## 7. Fázisok és becslés

A becslés **tájékoztató**, nem mért; a V1 eredeti becslése 20–35 óra volt (a neurális modellre és a parancsra), a három bővítés, a ripple/nem romboló mód és a Qt6 felület ezt növeli.

| Fázis | Tartalom | Becslés |
|---|---|---|
| **V1-A.0** | Parancsregiszter-nevek rögzítése, a `vad.rs` régiófüggvénye, fixture-generátor | 4–6 ó |
| **V1-A.1** | `audio.detectSpeech` (`query`/`preview`), csend-markerek, `silence.regions`, `silence.keep` | 6–10 ó |
| **V1-A.2** | `clip.removeSilence` (`cut`/`mark`, ripple/lift), egy undo-lépés | 5–8 ó |
| **V1-A.3** | Kötegelt elemzés (metszet-mód, megszakítható munka) | 5–8 ó |
| **V1-A.4** | Qt6 párbeszéd és régiólista, i18n | 8–14 ó |
| **V1-A.5** | `v1_silence_removal_acceptance` és a 6.2 tesztjei, dokumentáció (`qt-parity.md`, `venturi-parity.md` V1 állapot) | 6–10 ó |
| **V1-B** (MI-réteg után) | Silero modul, `detector: "silero"`, ±1 ablak teszt, eszköz-nyilvántartás, Whisper-előszűrő külön | 20–30 ó |

V1-A összesen kb. 34–56 óra, V1-B kb. 20–30 óra.

---

## 8. Licenc, függőség, kockázat

- **V1-A:** új függőség nincs (a meglévő `crates/speech`, `crates/edit`, `crates/engine`).
- **V1-B:** a Silero VAD modell MIT (a felhasználó tölti le, nem terjesztjük), a `tract-onnx` MIT OR Apache-2.0 (`docs/venturi-parity.md` 1. pont); a Venturi kódja GPL-3.0-or-later, átvehető, a fájlokban az eredet-fejlécet (repó + commit, szerző, licenc) **mi írjuk**, a nyilvántartásba a `licensing.md` §5-be ugyanabban a commitban. `cargo deny` ellenőrzés.
- **Kockázatok:** (1) az energiaalapú észlelő zajos felvételen téves régiót adhat, ezért az előnézet és a szerkeszthető kijelölés kötelező (R3–R5); (2) a metszet-mód többsávos köteg esetén a vártnál kevesebb régiót ad, ez szándékos és a felületen magyarázandó; (3) a régiómarkerek tömege hosszú anyagon lassíthatja az idővonalat, ezért a markerek száma korlátozandó (javaslat: legfeljebb néhány ezer, fölötte hibaüzenet és a paraméterek szigorítása).

---

## 9. A tulajdonosnak eldöntendő kérdések (eldöntve 2026-11-21: a javaslatok érvényesek)

| # | Kérdés | Javaslatom |
|---|---|---|
| **D1** | A V1-A (az energiaalapú észlelő) **MI-funkciónak** számít-e a 2026-10-27-i szabály szerint? Ha igen, a V1-A is az MI-réteg után indul. | Nem MI (nincs modell, nincs tanulás), ezért indulhat; a Silero a V1-B |
| **D2** | A „nem romboló mód” maradjon-e **csak jelölés**, vagy legyen egy **kikapcsolt darabok** változat is (a régiók kivágása a klipekből razor-ral, a darabok `clip.enable` kikapcsolásával, a hossz változatlan)? | Csak jelölés (amit a V1 5. pontja ír); a másik változat külön, utólagos tétel |
| **D3** | Kötegnél a régió legyen a **metszet** (csak ott töröl, ahol minden elemzett hang csendes), vagy **klipenkénti**? | Metszet ripple-lel, klipenkénti csak lifttel |
| **D4** | A régió legyen **marker** (a meglévő formátummal, új mező nélkül), vagy külön tároló (projektformátum-változás)? | Marker, mert nem kell sémaváltozás és az idővonalon már szerkeszthető |
| **D5** | Az észlelő alap-paraméterei: `minSilence` 0,5 s, `padding` 0,1 s elfogadható-e kiindulásnak? | Igen, a V1-A.5-ben mérés alapján hangolva |

---

## 10. Mit nem ellenőriztem (kötelező nyilvánosság)

1. **Kódot nem futtattam**, a parancsfelület és a becslés a kód olvasásából következik, nem mérésből.
2. A `crates/speech/src/vad.rs`-nek csak a fejlécét és a függvényneveit olvastam; hogy a keretszint-függvények közvetlenül hívhatók-e régióképzéshez, a V1-A.0 első lépése.
3. A **`plan/` mappát** (STATUS.md, execution-plan.md, architecture.md) nem láttam (gép-lokális, gitignore-olt); ha ott van a V1-et vagy az MI-réteget érintő jóváhagyott terv, azt nem vettem figyelembe. **Az MI-réteg tervét nem olvastam**: az D1 és a V1-B kapuja ezért a tulajdonos szavára épül.
4. A Venturi forráskódját nem vizsgáltam újra; a `vv-vad` szerepe a `docs/venturi-parity.md` leírásából való.
5. A menü pontos helye (Sequence vagy Clip) és a billentyűkötések a megvalósításkor igazodnak a meglévő menüszerkezethez.

---

## 11. Fázis-lezárási bizonyíték (a megvalósításra)

Minden fázis lezárásához kötelező: CI-napló (`cargo xtask ci` + tesztek), a golden összevetés kimenete (R11), az undo/redo és a mentés-újratöltés teszt kimenete, a GUI-automatizálási napló, regressziós összevetés (`transcript.removePauses` változatlan), `docs/venturi-parity.md` V1 sorának és a `docs/qt-parity.md`-nek a frissítése. A V1-A akkor **kész**, ha a `v1_silence_removal_acceptance` mindhárom ellenőrzése zöld, és a 6.2 táblázat minden sorára van teszt; a V1 **teljes**, ha a V1-B is kész.

---

## 12. A V1-B az MI-réteg zárási feltételei között (István, 2026-11-21)

**Döntés (István, 2026-11-21):** a V1-B (Silero VAD) az **MI-réteg zárásának feltételei közé** kerül. Az MI-réteg ezért addig nem tekinthető lezártnak, amíg a V1-B nincs kész. A V1-B továbbra is **az MI-réteg üzembe helyezése után** indul és külön indítási engedélyt kér (a V1-A jóváhagyása nem terjed ki rá); ez a pont csak azt rögzíti, hogy a lezárásnak a V1-B is feltétele.

A V1-B akkor **kész** (az MI-réteg lezárásához ennyit kell igazolni):

1. a `detector: "silero"` kiválasztható, az `audio.detectSpeech` a Silero-alapú régiókat adja ugyanazzal a parancsfelülettel és ugyanazzal a régióképző lépéssel, mint az energiaalapú (a V1-A-ban ez ma hibaüzenet);
2. a Silero modul a Venturi `vv-vad` alapján, az eredet-fejléc és a `licensing.md` §5 nyilvántartás ugyanabban a commitban, `cargo deny` zöld; a modellt a felhasználó tölti le a licenc elfogadása után (nincs modellfájl a repóban vagy a binárisban), hiányzó modellnél a `silero` érthető hibát ad;
3. a VAD-határok a generált fixture-hez képest ±1 ablakon (32 ms) belül vannak (a V1 4. pontja), CPU-n;
4. a `v1_silence_removal_acceptance` mindhárom ellenőrzése (a, b, c) a `silero` észlelővel is zöld, és a 6.2 táblázat sorai a `silero` esetre is le vannak fedve;
5. a Qt6 párbeszéd Detector listája a „Speech model” értéket is kínálja (a V1-A-ban csak az „Energy”);
6. a Whisper-előszűrő (V1 3. pont) külön tételként kész, vagy a tulajdonos kifejezetten kiveszi a V1-B-ből;
7. a `docs/venturi-parity.md` V1 sora és a `docs/qt-parity.md` 3.31 sora frissítve.

**Frissítés (2026-11-22):** a V1-B felvétele az MI-réteg tervébe megtörtént: CFA4-AI_modules ADR 0008 (F5 kapu és a „Lezárás” rész), a CFA4-AI_modules #57 PR-ben. A modell letöltési szabálya (nincs fagyasztott modell a repóban) ebben a tervben is érvényes.

**Mit nem tudtam elvégezni:** az MI-réteg saját zárási feltételeinek listája az MI-réteg tervében (a `CFA4-AI_modules` repó ADR 0006-ja) van, amely ebből a munkamenetből nem érhető el és nem is módosítható. Ezért a V1-B-t itt (a V1 tervében), a V1-A Gazda issue-ban (#1487) és a Gazda issue-ban (#1447) rögzítettem; **az MI-réteg saját zárási listájába a tulajdonosnak vagy az MI-réteg szálának kell felvennie**, hogy a feltétel ott is látszódjon.


## 13. V1-B: állapot (2026-11-23)

**Indítás:** István a V1-B-t kifejezetten felmentette az MI-réteg F0 kapuja alól, de **csak a GUI-független részre** (a koordinátor üzenete, 2026-11-23); az F0 kapu maga nem teljesült (az egui még a repóban van). A modellkezelő (CFA4-MODEL-MANAGER-001) külön szálon készül; ez a rész annak csatlakozási pontját adja, nem építi meg.

**Kész (a 12. pont feltételeihez képest):**

- 1. **igen:** `audio.detectSpeech {detector: "silero"}` a Silero-alapú régiókat adja, ugyanazzal a régióképző lépéssel (`vad::silence_regions_from_voiced`, az energiaalapú is ezt használja). Hiányzó modellnél, hiányzó `silero` featurénél és módosított modellnél hiba, **nincs csendes visszaesés** az `energy`-re (István, 2026-11-22).
- 2. **részben:** a `crates/speech` `vad::silero` modulja (feature `silero`, `tract-onnx` 0.23, MIT OR Apache-2.0). **Nem a Venturi kódja alapján készült**: a modell nyilvános be- és kimenetéből (`input`, `state`, `sr` → `output`, `stateN`) írtam, a Venturi forrását nem olvastam, ezért nincs átvett kód és nincs `licensing.md` §5 bejegyzés. A modell nincs a repóban és a binárisban. **`cargo deny` nem futott** (nincs telepítve ebben a környezetben).
- A modellt a felhasználó tölti le: `speech.vadModel` (forrás, licenc, méret, telepítve-e) → a gazda megmutatja a licencet → `speech.downloadVadModel {licenseAccepted: true}` (a meglévő, bezárt `cfa4-fetch`, rögzített URL és SHA-256; elfogadás nélkül elutasít). Az egyszeri, modellenkénti elfogadás tárolása a modellkezelő dolga.
- 3. **részben:** valódi modellel (CPU) futó tesztek (`crates/speech/tests/silero_model.rs`): csend és halk zaj nem beszéd, darabokban és egyben azonos valószínűségek, NaN/végtelen minta nem töri el, a beépített formáns hang a generált elrendezéshez képest az elején 4 ablakon (128 ms), a végén 0,4 s-on belül van. **A terv ±1 ablakos (32 ms) tűrése így nincs igazolva**: a robotszerű hang kifutása nem beszéd a modellnek, ezért a végén lazább a tűrés. Valódi emberi beszédes fixture kell hozzá (nincs ilyen a gépen).

**A modellről:** a tract az eredeti `silero_vad.onnx` és a `silero_vad_16k_op15.onnx` `If` csomópontján elakad (az 8 kHz-es ág alakja nem egyeztethető). A `silero_vad_op18_ifless.onnx` egyetlen legfelső szintű `If`-et tartalmaz (`sr == 16000`); a betöltő ennek a `then` (16 kHz) ágát illeszti a helyére, és eldobja az `sr` bemenetet. A rögzített URL (`snakers4/silero-vad`, `v6.2.3` címke) és SHA-256 a PyPI `silero-vad 6.2.3` kerekéből származik; **a GitHub-címkét és az ottani bájtazonosságot ebből a környezetből nem tudtam ellenőrizni** (a GitHub-letöltés tiltott), az első valódi letöltésnél kell megnézni.

**Még hátra (a 12. pontból):** 4. a `v1_silence_removal_acceptance` a `silero` észlelővel (modellfájl kell a CI-hoz); 5. a Qt6 párbeszéd „Speech model” értéke (GUI, az F0 felmentés nem terjed ki rá); 6. a Whisper-előszűrő; 7. a `docs/qt-parity.md` 3.31 sora.

### 13.1 V1-B, a további rész (2026-11-23, második szál)

- **12.5 kész:** a Qt6 párbeszéd Detector listája a „Speech model (Silero VAD)” értéket is kínálja (`python/cfa4_qt/silence_dialog.py`). A párbeszéd a `speech.vadModel` rekordot mutatja (forrás, licenc, méret); a modell nincs benne a binárisban, a **Letöltés** gomb csak a licenc elfogadásának bejelölése után él, és a `speech.downloadVadModel {licenseAccepted: true}` parancsot hívja. Hiányzó modellnél az **Analyze** érthető üzenetet ad, a motort nem hívja, és **nincs csendes visszaesés** az `energy`-re. Olyan fordításban, amelyben nincs `silero` feature, az érték nem választható. A küszöb-beállítások a modellnél tiltottak (a modell dönti el, mi a beszéd). A licenc **egyszeri, modellenkénti** elfogadásának tárolása továbbra is a modellkezelő dolga (itt csak a letöltés előtti bejelölés van, nem tárolódik).
- **12.6 kész (külön tételként):** a Whisper-előszűrő: `cfa4_speech::prefilter` (`speech_segments`, `transcribe_segments`) és `transcript.generate {vadFilter: "off"|"energy"|"silero"}` (alap `off`, így a meglévő viselkedés nem változik). Csak a beszédes szakaszok (0,2 s ráhagyással, 0,5 s alatti szüneteken át összevonva, 0,1 s alatti kattanások nélkül) kerülnek a felismerőhöz, a szavak a médiaidőben a helyükre kerülnek vissza, a nyelvet az első szakasz adja a többinek, a beszélő-címkézés egyszer fut a teljes hangon. Nem része a csend-eltávolítás parancsainak. Tesztek: `crates/speech/src/prefilter.rs`, `transcript_tests`.
- **12.4 és 12.3 előkészítve, modellel még nem futott:** `v1_silence_removal_acceptance_with_the_speech_model` (engine, a beépített hanggal, a (a), (b), (c) ellenőrzés a `silero` észlelővel) és `real_speech_bounds_are_within_one_window` (`crates/speech/tests/silero_model.rs`, valódi emberi beszéd + kézi címkék, ±1 ablak). Mindkettő **kihagyja magát** modell, illetve fixture nélkül (SKIPPED); a modellt teszt nem tölti le. Futtatás: tedd a modellt a `<adatkönyvtár>/models/silero-vad/` alá (az alkalmazás letöltése után), `CFA4_DATA_DIR=<adatkönyvtár> cargo test -p cfa4-engine --features silero silence_tests`; a valódi beszédes teszthez `CFA4_MODELS_DIR=<…/models>` és `CFA4_SPEECH_FIXTURE=<16 kHz WAV>` (mellette `<WAV>.speech.tsv`: soronként `kezdet<TAB>vég` másodpercben). **A ±32 ms tűrés igazolása ezért még hátra van**: valódi emberi beszédes minta nincs a felhő-környezetben, és a modellt nem én töltöm le.
