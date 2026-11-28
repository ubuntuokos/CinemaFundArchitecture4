# CFA4-DEV-AI-ORCHESTRATION-001 – F2 tervezet (jóváhagyásra)

**Állapot:** JÓVÁHAGYOTT (István, 2026-11-25). A jóváhagyás kiterjed a D-F2-1…4 döntésekre (a tervben szereplő javaslatok szerint) és a
[WP0 leltárra](CFA4-AI-ORCHESTRATION-F2-WP0-leltar.md), beleértve a C-1 és C-3 szövegmódosítást két külön kis PR-ban. A terv szövege a
jóváhagyás óta tartalmilag változatlan; D-F2-4 szerint ez a változatlan fájl az első végrehajtási PR része lesz
(`docs/plans/CFA4-AI-ORCHESTRATION-F2-001.md`). Tervmódosítás után a munka megáll (`coordination.md` §2.10).
Végrehajtás: C-1 PR #1850 (nyitott), C-3 PR #1851 (nyitott); WP2 ezek beolvasztása után indul.
**Dátum:** 2026-11-25 (jóváhagyva: 2026-11-25). **Engedély:** István: „F2 engedélyezve” (2026-11-25). Csak F2; F3–F9 és E0–E6 továbbra sincs engedélyezve.
**Forrás:** FINAL 1.0 (4. F2; 3. TC-01, TC-03), FINAL 1.1 (AIQT 1.0.6; F2 = „AIQT Policy és forráskezelési szabályok beillesztése”;
CFA4-AIQT-PROFILE-001), FINAL 1.2 felhős kiegészítés (D-C1–D-C5), F0 v2/v3 audit, F1 felhős felmérés (#650, #1596).

## 1. Mi az F2 hatóköre (a repóból)

A FINAL 1.0 szerint az F2 – „Szabály- és SPEC-integráció” – hat feladat; a FINAL 1.1 ehhez hozzáteszi az AIQT-szabályok és a
forráskezelés beillesztését, a README pedig a CFA4-AIQT-PROFILE-001 elkészítését az F2 részének mondja. A FINAL 1.2 szerint a
ThinkRail és a Pi + Claude **szünetel** (D-C2), tehát az F2 felhős értelmezése a **Claude Code** útvonalra szól, a Pi-oldal
helye csak fenn van tartva.

| # | FINAL 1.0 F2-feladat | Felhős értelmezés (FINAL 1.2 szerint) | Ebben a tervben |
|---|---|---|---|
| 1 | TC-01 Agent Policy Adapter | egy szabálynyilvántartás + Claude Code-oldali végrehajtás; Pi-ág fenntartva, nem épül | WP2, WP4 |
| 2 | TC-03 Context/Spec Bridge | ThinkRail-független követelményazonosító-gráf | WP3 |
| 3 | Közös szabályforrás | `AGENTS.md` marad az egyetlen szövegforrás; mellé gépileg ellenőrizhető szabályjegyzék | WP2 |
| 4 | Pi és Claude Code konfigurációk összerendelése | csak Claude Code (`CLAUDE.md`, `.claude/`); Pi: SZÜNETEL | WP2/WP4 |
| 5 | SPEC-gráf | követelmény ↔ modul/crate ↔ terv ↔ GitHub-issue | WP3 |
| 6 | Követelményparitási tesztek | Python/pytest teszt a CI-ben | WP2, WP3 |
| + | FINAL 1.1: AIQT-szabályok beillesztése | CFA4-AIQT-PROFILE-001 (AIQT-pont ↔ TC ↔ AC, átvett/kihagyott indoklással) | WP1 |

**Kilépési feltétel (FINAL 1.0):** szabály- és követelményparitás PASS. Felhős pontosítás: a paritást a CI (R2) igazolja, nem
a munkamenet-konténer (R1).

**Nem része az F2-nek:** ThinkRail, Pi, TC-02 (kész: D-C3), TC-04–TC-15 megvalósítása (csak a profilban hivatkozás), ruleset- és
hálózati beállítás, bármi a `main`-be olvasztással.

## 2. Mi van most (F0 v2/v3 szerint, a repóból ellenőrizve)

- A szabályok szövege négy helyen él: `AGENTS.md` (23 KB), `CLAUDE.md`, `docs/agents.md`, `docs/coordination.md`. Nincs gépi
  nyilvántartás, így nincs mit paritás-tesztelni (TC-01: „csak szöveges”).
- Ismert belső ütközés (F0 v2 5.3/1): az autonóm munkahurok (`docs/agents.md` §5–6, `CLAUDE.md` 3. lépés „Don't stop to ask”)
  ↔ előzetes jóváhagyás (FINAL 1.0 8.7, `coordination.md` §2.10). A szabályjegyzék nem épülhet ütköző szövegre.
- Nincs `SPEC.md`, nincs követelményazonosító-rendszer; a tervek `docs/plans/`-ban vannak, azonosítójuk a fájlnév.
- `.claude/` tartalma: `settings.json`, `agents/` (2 csak olvasó alügynök), `skills/` (`gazda`, `cfa4-skill-review`).
- A tervhelyre két szabály mutat: `coordination.md` §2.11 (terv a repóba, `docs/plans/`) és a te szabályod (terv md-fájlban,
  jóváhagyásra, nem PR-ban). Lásd D-F2-4.

## 3. Munkacsomagok

Minden csomag külön munkamenet, külön ág, legfeljebb egy PR (TC-02 / D-C3), minden zöld lépés után push (8.15a). Minden PR
Ready for review, a beolvasztást a gazda végzi; a csomagok végén „Gazda: …” issue a beolvasztandó PR-okkal.

**WP0 – Felmérés (csak olvasás, jelentés md-ben, nincs PR).** A négy szabályforrás és a `.claude/settings.json` szabályonkénti
leltára: mi kötelező, mi csak ajánlás, mi ütközik. Kimenet: leltár-md jóváhagyásra a projekt mappájába. Ez alapozza WP2-t.

**WP1 – CFA4-AIQT-PROFILE-001** (`docs/development/ai-orchestration/CFA4-AIQT-PROFILE-001.md`). Az AIQT 1.0.6 (`docs/reference/aiqt/1.0.6/`)
minden pontja: átvéve / kihagyva + indoklás, a megfelelő TC-komponens és AC-kapu (AC-01…AC-14), a felhős rétegek (R1/R2/R3)
szerinti helye. Az AIQT elsőbbsége a README szerint a harmadik (VISION/AGENTS/coordination és FINAL előzi meg). Csak dokumentum.

**WP2 – Szabályjegyzék + paritásteszt (TC-01).**
- `docs/development/ai-orchestration/rules.toml`: minden szabály = azonosító (`CFA4-RULE-001`…), forrás (fájl + szakaszkapocs),
  érvényesítési szint (`text` / `hook` / `ci`), hatókör (`claude-code`; `pi` mező fenntartva, üres), státusz. A szabály **szövege nem
  kerül át**, csak hivatkozás – így nincs második szövegforrás, ami elcsúszhatna.
- `python/tests/test_agent_rules_parity.py` (ruff + mypy strict + pytest, a meglévő CI-ben): minden azonosító egyedi; minden
  forráskapocs létezik; a CLAUDE.md kötelező pontjai mind le vannak fedve a jegyzékben; `ci`/`hook` szintű szabályhoz létezik a
  megnevezett ellenőrzés; ütközésként megjelölt szabálypár nem maradhat nyitva.
- Eredmény: azonos szabályhalmaz minden Claude Code munkamenetnek, és az eltérés a CI-ben megbukik (TC-01 „eltérő szabályok
  felismerése”).

**WP3 – Követelmény- és SPEC-gráf (TC-03).**
- `docs/development/ai-orchestration/spec/requirements.toml` + olvasható `SPEC.md` nézet: követelményazonosító (`CFA4-REQ-…`) →
  forrásdokumentum/szakasz (VISION, AGENTS, jóváhagyott terv) → érintett modul/crate → terv → GitHub-issue.
- Nem ThinkRail-formátum (az szünetel); az azonosítók és a gráf viszont átvihetők, ha a ThinkRail visszatér.
- Paritásteszt: nincs lógó hivatkozás, nincs azonosító nélküli jóváhagyott terv a bevezetett körben, ellentmondó követelmény
  (`conflicts_with`) nem maradhat feloldatlanul. Első körben csak a `VISION.md` + `AGENTS.md` + az AI-orchestration tervek
  követelményei; további tervek fokozatosan, külön jóváhagyással.

**WP4 – Végrehajtási szintű érvényesítés (TC-01 „végrehajtási szinten is”).** Claude Code `permissions`/hook a tiltásokra
(upstream márkanév a tárolóban, CI-kapu megkerülése, `main`-be olvasztás, más munkamenet ágára push). **Ehhez a
`.claude/settings.json` módosítandó, ami külön tulajdonosi jóváhagyás** (D-F2-3); jóváhagyás nélkül WP4 kimarad, a tiltások
`text`/`ci` szinten maradnak, ezt a jegyzék őszintén mutatja.

**WP5 – F2 záró jelentés és kapu.** `docs/audit/AI-ORCHESTRATION-F2-2026-…md`: AC-01a/AC-02 (Claude Code-ág)/AC-05 érintett pontjai,
VERIFIED/PENDING/BLOCKED táblával, a CI-futás hivatkozásával. A SKIPPED/BLOCKED/NOT_RUN nem siker (AC-06). A kapu elfogadása a tiéd.

Javasolt sorrend: WP0 → (jóváhagyás) → WP1 ∥ WP2 → WP3 → WP4 (ha jóváhagyott) → WP5. WP1 és WP2 egymástól független, külön
munkamenetben párhuzamosan futhat (különböző fájlok, nincs közös §2.5 fájl, kivéve a `README.md` táblázatot – azt csak WP5
frissíti).

## 4. Kockázatok

| Kockázat | Kezelés |
|---|---|
| Második szövegforrás elcsúszik az első mellett | a jegyzék csak hivatkozik, a teszt a kapcsokat ellenőrzi |
| Ütköző szabály (autonóm hurok) beégetve a jegyzékbe | WP0 felszínre hozza; a feloldás D-F2-1 szerint, a jegyzék addig `conflict` jelöléssel tartja |
| Túlzott szigor megakasztja a többi projektet | WP2 csak jelez és a saját CI-tesztben bukik; nem blokkol más munkamenetet; szigorítás külön döntés |
| Python/Qt CI-job instabilitása (F0 I-02) | az új teszt önálló, a Qt GUI-tesztek nélkül is fut (`--ignore=…test_qt_gui.py`) |
| Konténer megszűnik | 8.15a: zöld lépés = push |

## 5. Döntések, amik a tieid

- **D-F2-1 – autonóm munkahurok ↔ előzetes jóváhagyás.** Melyik érvényes? Javaslat: a `coordination.md` §2.10 (jóváhagyott terven
  belül autonóm, a terv módosítása megáll). Ezt a WP0 leltár pontosan szövegezi, a javítás külön, kis PR-ban menne.
- **D-F2-2 – a közös szabályforrás formája.** Javaslat: `AGENTS.md` marad az egyetlen szöveg, mellé `rules.toml` hivatkozás-jegyzék
  (A). Alternatíva: `CLAUDE.md` generálása egy forrásból (B) – több mozgó rész, nagyobb kockázat, nem javaslom.
- **D-F2-3 – `.claude/settings.json` módosítása (WP4).** Engedélyezed-e? Javaslat: igen, de csak a WP2 jegyzék elfogadása után,
  külön jóváhagyott lépésként.
- **D-F2-4 – hova kerül ez a terv a jóváhagyás után.** A szabályod szerint a tervet md-fájlban hagyod jóvá, nem PR-ban; a
  `coordination.md` §2.11 szerint a jóváhagyott terv a `docs/plans/`-ba kerül. Javaslat: jóváhagyás után a változatlan tartalom
  az első végrehajtási PR része (`docs/plans/CFA4-AI-ORCHESTRATION-F2-001.md`), és ettől kezdve az az irányadó. Ha nem kéred, a
  terv csak a projekt mappájában marad.

## 6. Mi történik jóváhagyás után

1. Te jóváhagyod ezt a fájlt (vagy módosítást kérsz; módosítás után újra jóváhagyás kell, §2.10).
2. WP0 indul (csak olvasás), a jelentést ugyanígy md-fájlban hozom jóváhagyásra.
3. Csak ezután nyílik az első PR (WP1 vagy WP2), draft → Ready for review, majd „Gazda: AI-ORCHESTRATION F2 …” issue.
