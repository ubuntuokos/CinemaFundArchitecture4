# CFA4 – beolvasztási gazda és PR-szabályok

**Tulajdonosi döntés, 2026-10-27; módosítva 2026-11-01 (a gazda csak a tulajdonos hívására
dolgozik).** Ez a dokumentum minden munkamenetre (Claude, ChatGPT/Codex, más MI-ügynök vagy
ember) kötelező. A cél: ne keletkezzenek egymásra épülő, egymással ütköző, soha be nem
olvasztott PR-ok és ágak.

## 1. Kinek mi a dolga

| Szerep | Ki | Mit tehet |
|---|---|---|
| **Tulajdonos** | ubuntuokos | Dönt a sorrendről, jóváhagyja a beolvasztásokat, ágat töröl a GitHubon. |
| **Beolvasztási gazda** | egyetlen gazda munkamenet, a tulajdonos `/gazda`-val hívja | **Csak a tulajdonos utasítására cselekszik.** Egyedül ő olvaszt be a `main`-be, kizárólag tulajdonosi jóváhagyással. Sorrendet javasol, ütközést old fel, lezár, ha valamit egy másik PR kivált. |
| **Fejlesztő munkamenet** | minden más | Egy témán dolgozik, egy PR-ban. **Nem olvaszt be**, más PR-jához és ágához nem nyúl. |

Egy gazda munkamenet van. A tulajdonos a `/gazda` paranccsal hívja
([`.claude/skills/gazda/SKILL.md`](../.claude/skills/gazda/SKILL.md)), és minden sikeres
feladat után `/clear`-rel üríti. A gazda magától nem ellenőriz, nem ütemez és nem dolgozik.
Gazdát csak a tulajdonos jelölhet ki vagy válthat le. Fejlesztő munkamenet nem lép gazda szerepbe.

## 2. Szabályok minden fejlesztő munkamenetnek

1. **Friss `main`-ből** indíts ágat. Ne építs másik, még nyitott PR ágára. Ha a munkád egy nyitott
   PR-tól függ, várd meg, amíg az bekerül, vagy jelezd a PR-od leírásában.
2. **Egy PR = egy téma.** Ne csomagolj össze független változtatásokat.
3. **Kész PR-t ne olvassz be magad.** Jelöld „Ready for review”-nak, és írd a leírásába, hogy kész.
   A beolvasztás a gazda dolga, a tulajdonos hívására.
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
   vagy ütközést találsz egy másik projekttel, jelezd a PR-od leírásában vagy megjegyzésében, és
   az érintett részen ne dolgozz tovább. A tulajdonos és a gazda együtt dönt; **a döntés
   kötelező**, a munkamenetek nem nyitják újra.

## 3. A gazda munkamódja (minden `/gazda` hívásnál)

0. **Minden hívás önálló.** A gazda a korábbi hívások eredményeire nem épít; minden tényt
   a `main` és a GitHub aktuális állapotából olvas újra.
1. **Állapotfelmérés** minden hívás elején (csak olvasás): nyitott PR-ok, az alapáguk, utolsó
   commit ideje, beolvaszthatóság a mostani `main`-re, teljes CI állapota, a PR-leírásokban
   jelzett kérések.
2. **Aktív PR-hoz nem nyúl:** ha az utolsó commit 3 óránál frissebb, vagy a leírás szerint még
   folyik a munka, kihagyja.
3. **Sorrendet javasol** a tulajdonosnak (`VISION.md` prioritása szerint: Qt6 először), és csak
   a tulajdonos által ugyanabban a hívásban jóváhagyott PR-okat olvasztja be, **egyenként**.
   Gyűjtő-PR-t nem nyit.
4. Beolvasztás előtt: a PR ága friss a `main`-nel (`update-branch`), a teljes CI
   (`cfa4-full-validation`) zöld **azon a fejen**.
   Ezt a kötelező `full-validation-gate` ellenőrzés technikailag is kikényszeríti (tulajdonosi
   döntés, 2026-10-31): csak akkor zöld, ha minden blokkoló teljes job azon a fejen ténylegesen
   lefutott és zöld. A Qt6 GUI-tesztek a GUI elkészültéig futnak, de nem blokkolnak.
5. Ha egy PR-t egy másik kivált, a gazda lezárja, és a megjegyzésben megnevezi, melyik váltotta ki.
6. Minden hívás eredményét egy sorban beírja a [`docs/coordination-log.md`](coordination-log.md)-be.
7. Ágat nem töröl (a munkamenetek nem kapnak rá engedélyt): a törölhető ágak listáját átadja
   a tulajdonosnak.
8. **Átfedés vagy ütközés két projekt között** (§2.8): a tulajdonossal együtt dönt, a döntést
   egy sorban a [`docs/coordination-log.md`](coordination-log.md)-be írja, és az érintett
   munkameneteknek a PR-jukban jelzi. A döntés ezután kötelező.
9. A feladat végén rövid jelentést ad, és **megáll**. Ütemezett vagy ismétlődő ellenőrzést nem hoz létre.

## 4. Ágak

A GitHubon be van kapcsolva az „Automatically delete head branches”. 2026-10-27-én mégis
54 olyan ág volt, ami már teljesen benne van a `main`-ben (42-nek a PR-ja ugyanazzal a fejjel
lett beolvasztva). Ennek oka még nincs kiderítve; a gazda a tulajdonos kérésére megnézi,
a következő beolvasztás után törlődik-e az ág.

## 5. A gazda hívása

- `/gazda` – állapotfelmérés és beolvasztási javaslat, utána megáll.
- `/gazda <utasítás>` – csak az adott utasítást hajtja végre (pl. `/gazda mergeld a #123-at`).

A szerep teljes leírása: [`.claude/skills/gazda/SKILL.md`](../.claude/skills/gazda/SKILL.md).
A tulajdonos minden sikeres feladat után `/clear`-rel üríti a gazda munkamenetet.
Időzített gazda-ellenőrzés nincs.
