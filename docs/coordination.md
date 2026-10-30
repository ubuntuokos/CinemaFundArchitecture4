# CFA4 – beolvasztási gazda és PR-szabályok

**Tulajdonosi döntés, 2026-10-27.** Ez a dokumentum minden munkamenetre (Claude, ChatGPT/Codex,
más MI-ügynök vagy ember) kötelező. A cél: ne keletkezzenek egymásra épülő, egymással ütköző,
soha be nem olvasztott PR-ok és ágak.

## 1. Kinek mi a dolga

| Szerep | Ki | Mit tehet |
|---|---|---|
| **Tulajdonos** | ubuntuokos | Dönt a sorrendről, jóváhagyja a beolvasztásokat, ágat töröl a GitHubon. |
| **Beolvasztási gazda** | egyetlen, erre kijelölt munkamenet | Egyedül ő olvaszt be a `main`-be. Sorrendet javasol, ütközést old fel, lezár, ha valamit egy másik PR kivált. |
| **Fejlesztő munkamenet** | minden más | Egy témán dolgozik, egy PR-ban. **Nem olvaszt be**, más PR-jához és ágához nem nyúl. |

Egyszerre **egy** gazda van. Az aktuális gazdát a [`docs/coordination-log.md`](coordination-log.md)
első sora mondja meg. Gazdát csak a tulajdonos jelölhet ki vagy válthat le.

## 2. Szabályok minden fejlesztő munkamenetnek

1. **Friss `main`-ből** indíts ágat. Ne építs másik, még nyitott PR ágára. Ha a munkád egy nyitott
   PR-tól függ, várd meg, amíg az bekerül, vagy szólj a gazdának.
2. **Egy PR = egy téma.** Ne csomagolj össze független változtatásokat.
3. **Kész PR-t ne olvassz be magad.** Jelöld „Ready for review”-nak, és írd a leírásába, hogy kész.
   A beolvasztás a gazda dolga.
4. **Más PR-jához, ágához ne nyúlj** (push, rebase, lezárás, gyűjtő-PR), akkor sem, ha ütközik
   a tieddel. Az ütközést a gazda oldja fel.
5. **Közös fájlok** (kodek-regiszter, `crates/codecs`, `crates/export/src/lib.rs`, effektlisták,
   `i18n/*.tsv`, `docs/*-parity.md`, `Cargo.toml`, `Cargo.lock`): csak a saját sorodat add hozzá,
   ne rendezd át és ne formázd újra a fájlt.
6. **Új téma indítása előtt** nézd meg a nyitott PR-okat. Ha 20-nál több van, új munkát csak
   a tulajdonos kérésére kezdj.
7. Munka végén **hagyd tisztán**: ne maradjon PR nélküli ág, ideiglenes (`tmp/`, `ci-debug/`) ág.
8. **Projekthatár** (tulajdonosi döntés, 2026-10-31). A saját projekteden (a tulajdonos által
   jóváhagyott terv, ADR vagy issue, amellyel a munkamenet indult) belül logikus sorrendben magad
   választhatod a következő feladatot. **Másik projektből nem vehetsz át feladatot.** Ha átfedést
   vagy ütközést találsz egy másik projekttel, jelezd a gazdának (PR-megjegyzésben vagy issue-ban),
   és az érintett részen ne dolgozz tovább. A tulajdonos és a gazda együtt dönt; **a döntés
   kötelező**, a munkamenetek nem nyitják újra.

## 3. A gazda munkamódja

1. **Állapotfelmérés** minden kör elején (csak olvasás): nyitott PR-ok, az alapáguk, utolsó
   commit ideje, beolvaszthatóság a mostani `main`-re, teljes CI állapota.
2. **Aktív PR-hoz nem nyúl:** ha az utolsó commit 3 óránál frissebb, vagy a leírás szerint még
   folyik a munka, kihagyja.
3. **Sorrendet javasol** a tulajdonosnak (`VISION.md` prioritása szerint: Qt6 először), és csak
   a jóváhagyott PR-okat olvasztja be, **egyenként**. Gyűjtő-PR-t nem nyit.
4. Beolvasztás előtt: a PR ága friss a `main`-nel (`update-branch`), a teljes CI
   (`cfa4-full-validation`) zöld **azon a fejen**.
5. Ha egy PR-t egy másik kivált, a gazda lezárja, és a megjegyzésben megnevezi, melyik váltotta ki.
6. Minden kör eredményét egy sorban beírja a [`docs/coordination-log.md`](coordination-log.md)-be.
7. Ágat nem töröl (a munkamenetek nem kapnak rá engedélyt): a törölhető ágak listáját átadja
   a tulajdonosnak.
8. **Átfedés vagy ütközés két projekt között** (§2.8): a tulajdonossal együtt dönt, a döntést
   egy sorban a [`docs/coordination-log.md`](coordination-log.md)-be írja, és az érintett
   munkameneteknek a PR-jukban vagy issue-jukban jelzi. A döntés ezután kötelező.

## 4. Ágak

A GitHubon be van kapcsolva az „Automatically delete head branches”. 2026-10-27-én mégis
54 olyan ág volt, ami már teljesen benne van a `main`-ben (42-nek a PR-ja ugyanazzal a fejjel
lett beolvasztva). Ennek oka még nincs kiderítve; a gazda első feladata megnézni, a következő
beolvasztás után törlődik-e az ág.

## 5. A gazda indítása

A tulajdonos új beszélgetést nyit, és ezt másolja be nyitó üzenetként (változtatás nélkül):

````text
Te vagy a CFA4 (https://github.com/ubuntuokos/CinemaFundArchitecture4) **beolvasztási gazdája**. Magyarul válaszolj.

Mielőtt bármit csinálsz, olvasd el a tárolóból: `VISION.md`, `docs/coordination.md`, `docs/coordination-log.md`, `AGENTS.md`.

A feladatod: a nyitott PR-ok rendbetétele úgy, hogy a többi, párhuzamosan dolgozó munkamenetet ne zavard.

1. Írd be magad gazdának a `docs/coordination-log.md` első sorába (PR-ral, mert a `main` védett).
2. Csak olvasva mérd fel az állapotot: minden nyitott PR-hoz az alapág, az utolsó commit ideje, beolvasztható-e a mostani `main`-re, mivel ütközik, és van-e zöld teljes CI-je (`cfa4-full-validation`) a mostani fején.
3. Sorold őket csoportokba: (a) aktív, 3 órán belüli commit – nem nyúlsz hozzá; (b) beolvasztható; (c) ütközik vagy újraalapozás kell; (d) egy másik PR kiváltotta; (e) tulajdonosi döntés kell.
4. Javasolj sorrendet a `VISION.md` prioritása szerint (Qt6 először), és **várd meg a jóváhagyásomat**.
5. A jóváhagyott PR-okat **egyenként** olvaszd be: friss ág a `main`-nel, teljes CI zöld azon a fejen, utána beolvasztás. Gyűjtő- vagy integrációs PR-t ne nyiss.
6. Kiváltott PR-t csak megjegyzéssel zárj le, amiben megnevezed, mi váltotta ki.
7. Ágat nem tudsz törölni: a törölhető ágak listáját add át nekem.
8. Nézd meg, a beolvasztás után törlődik-e automatikusan az ág (a GitHubon be van kapcsolva, de 54 beolvasztott ág mégis megmaradt).
9. Minden kör végén egy sor a `docs/coordination-log.md`-be.

Más munkamenet PR-jába vagy ágába ne tölts fel semmit. Ha egy ütközést csak úgy lehet feloldani, kérdezz meg előbb.
````
