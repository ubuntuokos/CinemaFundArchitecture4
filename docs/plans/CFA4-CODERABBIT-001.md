# CFA4-CODERABBIT-001 – a Claude és a CodeRabbit összehangolása

**Állapot:** 1.1. A tulajdonos 2026-11-12-én a D1–D4 döntési pontokat a javasolt alapértékkel elfogadta,
2026-11-13-án pedig a próba után a D5-öt (4. fejezet). A K0–K2 lépések és a próba első része ebben a PR-ban
vannak; a K3 többi része a beolvasztás után következik.

**Összefoglaló:** a CodeRabbit a CFA4 tárolójában telepítve van, de eddig egyetlen PR-t sem ellenőrzött,
és a Claude-munkamenetek szabályai nem tudtak róla. A terv után egyetlen szabályforrás marad (az
`AGENTS.md`, a `CLAUDE.md` és a `docs/coordination.md`), és a CodeRabbit is ebből dolgozik. A fejlesztő
munkamenet kéri a CodeRabbit ellenőrzését, ő kezeli a megállapításokat, és a gazda csak lezárt
CodeRabbit-szálakkal olvaszt be. A CodeRabbit soha nem commitol és nem olvaszt be.

## 1. Kiindulási tények (2026-11-12, `main` 6be126a)

- A CodeRabbit GitHub-alkalmazás telepítve van, és nagyjából a #1020 óta minden PR-ra kiírja a
  „Run configuration” megjegyzését. A szövege szerint a tároló **nem kap automatikus ellenőrzést, mert
  10-nél kevesebb csillaga van** (most 1 van). Ellenőrzés csak `@coderabbitai review` megjegyzésre
  vagy a „Trigger review” jelölőnégyzetre indul. Ezt eddig senki nem használta: a PR-okon nincs
  CodeRabbit-review, és nincs soron belüli CodeRabbit-megjegyzés sem.
- Nincs `.coderabbit.yaml`. A CodeRabbit alapbeállítással futna: CHILL profil, angol nyelv, a CFA4
  szabályai nélkül, alapból bekapcsolt javító funkciókkal (Autofix, „Fix comments”, CI-javítás,
  ütközésfeloldás), amelyek commitot tolnának a PR-ba.
- Az `AGENTS.md`, a `CLAUDE.md`, a `docs/coordination.md` és a `/gazda` skill nem említi a
  CodeRabbitet.
- Az upstream a CodeRabbitet a Dependabot-PR-okon kikapcsolta; a CFA4 ezt eddig nem vette át
  (`docs/upstream/upstream-sync.md`, upstream #2594, „kihagy”).

## 2. Cél és hatókör

**Cél:** a CodeRabbit második szemként ellenőrizze a Claude-munkamenetek PR-jait, ugyanazokkal a
szabályokkal, és ne lépjen a gazda vagy a fejlesztő munkamenet szerepébe.

**Hatókörön kívül:** a CodeRabbit kötelező GitHub-ellenőrzéssé tétele (D2); a CodeRabbit
issue-tervező és -bővítő funkciói; a csillagszám miatti korlát megkerülése; fizetős csomag.

## 3. Lépések

| Lépés | Tartalom | Állapot |
|---|---|---|
| **K0** | `.coderabbit.yaml`: magyar nyelvű review (`tone_instructions`, mert a sémában nincs `hu` kód); szabályforrás az `AGENTS.md`, a `CLAUDE.md` és a `docs/coordination.md` (`knowledge_base.code_guidelines`); ütközésnél a CFA4 szabálya nyer (`path_instructions`); ki van kapcsolva minden commitoló funkció (`finishing_touches`), az automatikus jóváhagyás, a „slop” szűrő, a LanguageTool, a címke- és bírálójavaslat, valamint az issue-bővítés; kimarad a `vendor/`, a `Cargo.lock`, a `requirements/*.txt`, az `i18n/**/*.tsv` és a nagy upstream-táblázat; a Dependabot- és a köteg-PR-ok nem kapnak ellenőrzést. A fájl a CodeRabbit v2 sémája szerint hibátlan. | ebben a PR-ban |
| **K1** | `docs/coordination.md` §2.9 (fejlesztő munkamenet) és §3.4 (gazda): ki kéri az ellenőrzést, hogyan kell lezárni a szálakat, mi blokkol. A szabály csak itt szerepel (a szabályok egy helyen maradnak). | ebben a PR-ban |
| **K2** | `.claude/skills/gazda/SKILL.md`: a gazda a beolvasztás előtt a `ccr/review_threads` végponttal ellenőrzi a CodeRabbit-szálakat. | ebben a PR-ban |
| **K3** | Próbaüzem: a beolvasztás utáni első néhány PR-on megnézni, hogy a CodeRabbit magyarul ír-e, betartja-e a CFA4 szabályait, és nem talál-e túl sok tévedést. Szükség esetén a `.coderabbit.yaml` finomítása külön PR-ban. | a beolvasztás után |

Esetleges későbbi lépés, csak külön tulajdonosi döntéssel: a `scripts/dev/gazda-pr-allapot.sh`
bővítése egy CodeRabbit-oszloppal, hogy a gazdának ne kelljen PR-onként lekérdeznie.

## 4. Döntések (a tulajdonos 2026-11-12-én elfogadta)

- **D1 – ki indítja:** a fejlesztő munkamenet, amikor a PR-t „Ready for review”-nak jelöli
  (`@coderabbitai review`), és a gazdához adás előtt kijavítja vagy megindokolja a megállapításokat.
- **D2 – blokkol-e:** igen, a gazda megvárja, hogy ne maradjon megválaszolatlan CodeRabbit-szál.
  Technikai CI-kapu nem lesz (`request_changes_workflow: false`).
- **D3 – Dependabot:** a Dependabot-PR-ok kimaradnak, ahogy az upstreamnél is; ezeket a
  Dependabot-előkészítő munkamenet a teljes CI-vel ellenőrzi.
- **D4 – nyelv:** magyar.
- **D5 – az ingyenes keret miatt (2026-11-13, a D1 és a D2 hatókörét szűkíti):** CodeRabbit-ellenőrzés
  csak **kockázatos** PR-on kell: biztonság (CFA4-SECURITY-001, `crates/platform/`, `deny.toml`,
  `.semgrep/`), kodek és konténer (`crates/codecs/`, `crates/ffmpeg/`, `crates/gstreamer/` és a
  `docs/architecture.md` §1 kodek- és konténer-crate-jei), CI (`.github/`), szabályfájlok (`AGENTS.md`,
  `CLAUDE.md`, `docs/coordination.md`, `.coderabbit.yaml`, `.claude/`). A pontos lista csak a
  `docs/coordination.md` §2.9-ben szerepel. Ha a keret elfogyott, a kockázatos PR vár, és a fejlesztő a
  jelzett idő után újra kér; a gazda CodeRabbit nélkül nem engedi tovább. Más PR-on nem kérünk
  ellenőrzést, hogy a keret a kockázatos PR-oknak maradjon.

## 5. Kockázatok és a próba eredménye (2026-11-12, #1118)

- **A csillagkorlát:** a kézi `@coderabbitai review` kérés működik: a #1118-on a CodeRabbit a PR-ág
  `.coderabbit.yaml`-jával („Layers: Repository YAML”), magyarul ellenőrzött, és három valós hibát
  talált a §2.9/§3.4/§3a szövegében (ezeket a 4d4c635 javította).
- **Ingyenes OSS-review-korlát – a D2-t érinti:** a második kérésre ugyanazon a PR-on, kb. 3 perccel
  az első review után, a CodeRabbit ezt írta: „Review limit reached … Next included review
  available in 52 minutes”. A tároló tehát nagyjából **óránként egy review**-t kap. Ez naponta
  10–30 PR mellett szűk keresztmetszet: a D2 szerint a gazda vár, így a beolvasztás a
  CodeRabbitre várna. A tulajdonos a D5-ben
  úgy döntött (2026-11-13), hogy csak a kockázatos PR-ok kérnek ellenőrzést, és azok kivárják a keretet.
  A K3 próbaüzemben azt is figyelni kell, belefér-e a kockázatos PR-ok száma az órás keretbe.
- **A magyar nyelv nem hivatalosan támogatott:** a `tone_instructions` kérés; a próbán betartotta.
- **Ugyanaz a GitHub-fiók:** a munkamenetek a tulajdonos fiókjával írnak, így a `@coderabbitai`
  kérések is az ő nevében jelennek meg. Ez megegyezik a többi munkamenet-tevékenységgel.
