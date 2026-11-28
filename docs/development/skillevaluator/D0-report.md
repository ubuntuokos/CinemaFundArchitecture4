# CFA4-DEV-NVIDIA-SKILLEVALUATOR-001 – D0 jelentés (előkészítés)

**Dátum:** 2026-11-28 · **Terv:** CFA4-DEV-NVIDIA-SKILLEVALUATOR-001 FINAL 1.1 (jóváhagyta a tulajdonos; DG-10 megadva, 2026-11-28)
**Munkacsomag:** D0 (a terv 6. pontja). Ez a jelentés **nem kódváltoztatás**: a repóban csak ez a jelentés és egy javasolt, még
nem használt hash-zárolt függőséglista (`skillspector-2.13.7.lock`) kerül be. A telepítő, a wrapper és a skill változatlan.

Mérés helye: felhős munkamenet, Ubuntu (nem a célgép), x86_64, `uv` 0.x, Python 3.13.18 és 3.14.6 (`uv python install`).
Ez az R1 réteg (a `ai-orchestration` FINAL 1.2 szerint): a 3.14 eredmény tájékoztató, az R3 referenciagépen nincs mérve.

## 1. Összefoglaló

| D0-feladat | Eredmény |
|---|---|
| SkillEvaluator vizsgálata | **Apache-2.0**, van `NOTICE` és `THIRD_PARTY_NOTICES.md`; v1.2.1 (tag-commit `f7fcd35d79d0506fadbd79f2b3ff384c98bd10f3`, 2026-11-25); Python 3.12–3.14. Kódátvétel nincs (DG-01), csak funkcionális minta. |
| SkillSpector 3.14 | **Telepíthető és működik 3.13-on és 3.14-en is** (v2.13.7), a vizsgálat eredménye azonos. A `yara-python` 3.14-re **nem manylinux-kerékként** települ (lásd 3.), natív fordítás valószínű. |
| Verzióeltérés | A CFA4 `2.13.2`-re van rögzítve, a SkillEvaluator 1.2.1 **legalább 2.13.7-et** kér. A frissítés külön felülvizsgálat (`skillspector.md` „Telepítés frissítése”). |
| Hash-zárolt lista | 76 csomag, 1774 hash, azonos 3.13 és 3.14 alatt: `skillspector-2.13.7.lock`. Nincs GPL/AGPL. |
| Skill-formátumok | Agent Skills (`SKILL.md`); 7 harness-profil. A CFA4-ben első körben `.claude/skills` és `.agents/skills`. |
| Port vagy marad | Python marad, külön folyamatban (indoklás: 6.). |
| Licenc-nyilvántartás | Kódátvétel nincs, ezért `docs/licensing.md` nem változik. |

## 2. SkillEvaluator (donor)

- Tároló: `NVIDIA/SkillEvaluator`, HEAD a vizsgálat napján `faea417340a2a10ef9278c5f48ed6e68694a2398`; kiadás: `v1.2.1`.
- Licenc: `Apache-2.0` (`LICENSE`, `license = "Apache-2.0"` a `pyproject.toml`-ban), `NOTICE`, `THIRD_PARTY_NOTICES.md`.
  A CFA4 szabálya szerint (AGENTS: GPL-3.0-kompatibilis kód fejléccel és regiszter-sorral átvehető) a licenc nem akadály;
  **a döntés mégis: nincs kódátvétel**, a terv szerinti funkcionális minta.
- Alap függőségek (Tier 1): bandit, click, idna, jinja2, markdown-it-py, packaging, pathspec (MPL-2.0), pydantic, pyyaml, regex,
  rich, **uv** (a SkillSpector külön környezetben való futtatásához). Az `llm`, `tier2`, `tier3`, `security`, `dashboard` extrák
  külön vannak; a Tier 3 a **Harbor** keretrendszerre és (a dokumentáció szerint) konténeres környezetre épül, ami a DG-04
  döntéssel (Landlock + seccomp, nincs Docker) ütközik: a Tier 3-at a CFA4-ben nem átvenni, hanem a terv P8 szerint újra kell
  kötni a `cfa4-sandbox`-hoz.
- **Hálózati viselkedés:** a `skillevaluator validate` Tier 1 előtt **letölti** a rögzített SkillSpector-kereket és a Gitleaks
  8.30.0 binárist a GitHub kiadásaiból (SHA-256-tal ellenőrizve; `utils/scanner_install.py`). A CFA4-ben nincs automatikus letöltés
  (VISION §5, SECURITY-001 D5: letöltés csak kért módon, a `cfa4-fetch` útján). Ezért a SkillEvaluator **nem hívható közvetlenül**;
  a CFA4 saját, kifejezett telepítési lépést használ.
- A `harness-check` statikus, hálózat nélkül fut: kipróbálva a `.agents/skills/cfa4-elephant` skillen (7 harness).

## 3. SkillSpector (biztonsági donor)

Mérések (izolált `uv` venv, `git+https://github.com/NVIDIA/SkillSpector.git@v2.13.7`):

| | Python 3.13.18 | Python 3.14.6 |
|---|---|---|
| Telepítés | OK | OK |
| `uv pip check` | OK | OK |
| `skillspector --version` | `v2.13.7` | `v2.13.7` |
| `yara-python` | 4.5.4, `cp313 … manylinux_2_17_x86_64` (előre épített) | 4.5.4, `cp314 … linux_x86_64` (**nem manylinux**: a helyben fordítás valószínű; `gcc` volt a gépen) |
| `scan .agents/skills/cfa4-elephant --no-llm --fail-on-incomplete` | pontszám 0, `SAFE`, nincs találat | **azonos** (csak az időbélyeg tér el) |

Következmények:

1. **A 3.14-kockázat (terv 12.2) enyhült, de nem tűnt el:** működik, de a YARA-összetevő 3.14-en natív fordítást igényelhet
   (C fordító, Python-fejlécek). Ez a P réteg csomagolására hat (a `.deb`/Flatpak/AppImage-ben a Python-futtatókörnyezet és a
   kerekek eredete), ezért **P-függőség** (lásd 7.).
2. **A kimenet változott a 2.13.2-höz képest.** A CFA4 doksija szerint 2.13.2 a `cfa4-elephant`-ra `CAUTION`/`partial`-t adott;
   a 2.13.7 `SAFE` és `complete_with_caveats` (`analysis_completeness.is_complete: true`). A JSON-szerkezet is más lehet. A D3
   adaptert a **lefedettségi mezőkre** (`analysis_completeness.*`), nem az ajánlásra kell építeni: a `SAFE` önmagában nem PASS
   (a `skillspector.md` szabálya: hiányos elemzés ≠ APPROVE).
3. **A verzióváltás külön lépés:** a `skillspector.md` előírja az új kiadás, a Python-tartomány, a licenc és a függőségek
   felülvizsgálatát; a telepítő, a teszt és a doksi módosítását; felhős validációt. Javaslat: a D3 első tevékenysége (külön PR).
4. A SkillSpector tárolója tartalmaz `semver`-függőséget a `HEAD`-en, a `v2.13.7` címkén nem: **mindig a címke-commit
   függőségeit kell zárolni**, nem a `HEAD`-et.

## 4. Hash-zárolt függőséglista (javaslat)

Fájl: [`skillspector-2.13.7.lock`](skillspector-2.13.7.lock) (`uv pip compile --generate-hashes --universal`, a
`v2.13.7` címke-commit `c323ad4c43e3c8f2de051572cc25b2711d11e648` `pyproject.toml`-jából). 76 csomag, 1774 hash. A 3.13 és a
3.14 célra ugyanaz a csomagkészlet jött ki.

Licenc-áttekintés (telepített metaadatok alapján, 3.14-es venv):

- Nincs GPL, AGPL vagy LGPL a SkillSpector függőségei között. Túlnyomórészt MIT, BSD, Apache-2.0, ISC, PSF.
- **MPL-2.0 (fájlszintű copyleft, módosítatlan használat):** `certifi`, `orjson`. Elfogadható, ha nem módosítjuk és a forrás-
  elérhetőség megmarad.
- **Kézzel igazolandó (a metaadat pontatlan):** `python-dateutil` („Dual License”, ismerten Apache-2.0 vagy BSD-3-Clause),
  `pyasn1_modules` („BSD”), `numpy` (többszörös, a csomagolt komponensek kerekében).
- Nem fedi: magát a SkillSpectort (commit/kerék szerint rögzített), a SkillEvaluatort (nem függőség), a Semgrep-et (a
  SkillEvaluator `THIRD_PARTY_NOTICES.md` szerint LGPL-2.1 eszköz; nálunk nem használt).

A lista **nincs bekötve**: a `install-skillspector.sh` még a 2.13.2-es, nem hash-zárolt telepítést végzi. A bekötés (D3)
és a `--require-hashes` kapcsolóra váltás külön PR. Reprodukálás:

    git clone https://github.com/NVIDIA/SkillSpector && cd SkillSpector && git checkout v2.13.7
    uv pip compile --generate-hashes --python-version 3.13 --universal pyproject.toml

## 5. Skill-formátumok és harness-profilok (H1)

- A SkillEvaluator az **Agent Skills** specifikációt (`SKILL.md`, YAML frontmatter) és hét harness-profilt ismer: Claude Code,
  Codex, Copilot CLI, Hermes, OpenClaw, OpenCode, pi. A profilok verziózottak.
- A loaderek eltérnek: ugyanaz a `SKILL.md` az egyikben betöltődik, a másikban eldobódik (ismétlődő kulcs, tabulátor stb.).
- **Valós lelet a CFA4 repóban:** a `harness-check` a `.agents/skills/cfa4-elephant` skillre azt mondja, hogy a **Claude Code
  nem találja** (csak a `.claude/skills` és a `~/.claude/skills` mappát olvassa), a többi hat harness betölti. Ez nem hiba
  a skillben (a repó EGM-skillje a nem Claude-útvonalra szól), de a D2 kompatibilitás-szabályainak ezt a különbséget le kell írnia.
- **A CFA4 skill-fogalma első körben (javaslat a D1-hez):** egy könyvtár, benne `SKILL.md` (YAML frontmatter: `name`, `description`),
  opcionálisan `scripts/`, `references/`, eszköz- és MCP-deklaráció a frontmatterben; hely: `.claude/skills/<név>/` vagy
  `.agents/skills/<név>/`. Plugin-manifestek (a SkillEvaluator ismeri) **kívül maradnak**, külön jóváhagyásig (FINAL 1.1 §2.1).

## 6. Port vagy marad (AGENTS: „port or keep the original language”)

**Marad Python**, külön folyamatban, a Rust-motor mellett:

- A SkillSpector és a SkillEvaluator Python; a SkillSpector a tree-sitter-, YARA- és LLM-keretrendszer-függőségeivel (76 csomag)
  nem portolható észszerű időn belül.
- A D réteg fejlesztői eszköz (`python/cfa4_devtools`), nem a médiaútban fut; a P rétegben is külön, felügyelt alfolyamat
  (ADR 0006 worker-minta): összeomlása hiba, nem a CFA4 összeomlása.
- „Nem lassít” igazolása: a P réteg mérési kapuja a referenciagépen (lejátszás és renderkockaidő változatlan a vizsgálat
  alatt); a D0-ban nincs futásidejű kód, ezért nincs mit mérni.
- Rust-portolás: nincs tervezve. A policy és a gate logika egyetlen helyen él (a D-ben), hogy ne legyen két kiértékelő.

## 7. P-függőségek (jelzés a P szálnak; a P-hez nem nyúltam)

1. **Csomagolás:** a SkillSpector 76 csomagos Python-futtatókörnyezetének eredete a `.deb`/AppImage/Flatpak-ban; a `yara-python`
   3.14-es natív fordítása (C fordító) vagy a 3.13-ra rögzített futtatókörnyezet választása. Döntés: P3 lezárása előtt.
2. **Letöltés:** a szkenner telepítése nem automatikus; a P-ben a `cfa4-fetch` útján, látható forrással.
3. **A `skillevaluator`-féle automatikus letöltések** (Gitleaks, SkillSpector-kerék) nem engedhetők a termékben.
4. A P szál D→P szerződés-javaslata (parancssori belépő `--json` borítékkal, hibakódok, fájl-registry elrendezés) a D2–D5
   tervezésénél figyelembe veendő; nincs ellentmondásban a D0 megállapításaival. A szerződést a D5 tervezésekor véglegesítjük.

## 8. Nyitott pontok és a következő lépés (D1)

- A `python-dateutil`, `pyasn1_modules` és `numpy` licencének kézi igazolása a bekötés (D3) előtt.
- A 3.14 mérés R3 (referenciagép) szerinti ismétlése a `deferred-physical-tests.md` szerint **csak ha a P csomagolás 3.14-et
  választ**; most nincs kihagyott fizikai teszt, mert a D0 nem futásidejű.
- **D1:** Intake és parser: a skill-fogalom (5. pont) szerint; korlátok (méret, fájlszám, mélység, `..` és szimbolikus link);
  végrehajtás nélkül; hash és eredet.
