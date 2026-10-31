# CFA4 Superpowers — szelektív integráció (2026-10-29)

**Jóváhagyás:** CFA4-SUPERPOWERS-COST-001 FINAL 1.0.
**Forrás:** obra/superpowers v7.0.1, commit
`2ce8aa15d05bd57213e6f746f953aafb7ded7a8b` (MIT).
Nincs upstream kódmásolás vagy plugintelepítés. Az itteni ellenőrző eszköz
eredeti CFA4-fejlesztés GPL-3.0-or-later licenccel.

## Felülírási és jogosultsági sorrend

`VISION.md`, `AGENTS.md`, `docs/coordination.md`, a CFA4 jóváhagyott
AI-orkesztrációs szabályai és AIQT 1.0.6 elsőbbséget élveznek.
Terv jóváhagyása nem jelent automatikus fejlesztés-, PR-, merge- vagy
release-engedélyt. Az upstream szülőprojekt nem módosítható. Célplatform kizárólag
Ubuntu 26.04 LTS amd64. A Qt6-felület marad a fejlesztés elsődleges feladata.

## Valóban rendelkezésre álló komponensek

- A CFA4 `main` ágán nem igazolt a ThinkRail Pi / Claude Code / EGM
  tényleges, közösen vezérelt modellhívási határfelülete.
- Az EGM-integráció külön, nyitott PR #334; nem szabad rá építeni, más
  munkamenet ágát módosítani vagy késznek minősíteni.
- Nem kerül automatikus Superpowers bootstrap, SessionStart hook vagy
  párhuzamos alügynök a CFA4-be.
- Nincs új daemon, hálózati hívás, új fizetős API vagy automatikus CI-újrafuttatás.

A Superpowers képességeit egyenként, célfeladathoz kell alkalmazni.
Alapmód az egymenetes végrehajtás. Csak az explicit engedélyezett modulok
használhatók; a worktree-koordinációt a meglévő CFA4 folyamat végzi.
`config/dev/superpowers-allowlist.toml` kezdetben **enabled=false**.

## Read-only token- és költség-előellenőrzés

`python/cfa4_devtools/superpowers_cost.py` Python standard library
alkalmazás, nem futtat AI-ügynököt, nem indít fizetős műveletet, nem ment
személyes adatot és nem módosít hostbeállításokat.

A `--request` JSON mezői:
`skill`, `mode`, `approved`, `paid`, `input_tokens`,
`max_output_tokens`, `input_usd_per_million`,
`output_usd_per_million`.

A `--usage` JSON mezői:
`baseline_tokens`, `baseline_paid_usd`,
`spent_tokens`, `spent_paid_usd`.

Használat (nem generál és nem fizet tokeneket):

```bash
PYTHONPATH=python python3 -m cfa4_devtools.superpowers_cost \
  --policy config/dev/superpowers-allowlist.toml \
  --request /path/to/approved-request.json \
  --usage /path/to/trusted-usage.json
```

Kimenet: `DENY` (exit 2), vagy a feltételek teljesülésekor
`ALLOW_ESTIMATE_ONLY` (exit 0). Utóbbi **nem jogosultság** és **nem
futásidejű kikényszerítés**. A kérésben szereplő `approved=true`
nem hitelesít emberi engedélyt: ezt a tényleges koordinátor ellenőrzi.
A használati adatokat hiteles külső mérésből kell betölteni; a modellügynök
önbevallása nem elfogadható.

A valódi költségkorlátokhoz a modellhívási határfelületen kell:
1. bizonyíthatóan árazott modellt és maximális generálási hosszt beállítani;
2. egyetlen atomikus ledgerben költséget előre lefoglalni;
3. minden hívást és újrapróbálkozást ugyanazon kapun átvezetni;
4. tényleges számlázási adatból elszámolni és konzisztensen felszabadítani;
5. nem mért költség, ütköző lefoglalás vagy hiányzó engedély esetén leállni.

Ezek megvalósulásáig **a futtatás engedélyezése TILOS**. A helyi
preflight nem védi a közvetlen Claude vagy Pi hívásokat.

## S0: bázismérés

Csak ténylegesen hozzáférhető provider/CI adatok használhatók:
feladatosztály, érintett commit, modell, input/output/cache tokenek,
billed USD, helyi CPU/GPU idő és energia, CI költség és a minőségi kapuk.
Személyes promptok, titkok és hozzáférési tokenek nem tárolhatók.
Ha az összehasonlítható bázis hiányzik, a megtakarítás és a
költség-nem-növekedés **NEM IGAZOLT**.

## SC-01–SC-16 validációs állapot

| Kapu | Jelenlegi állapot | Feltétel |
|---|---|---|
| SC-01 | PARTIAL | Nincs automatikus indítás; élő koordináció nincs mérve |
| SC-02 | BLOCKED | Provider oldali költségkapu hiányzik |
| SC-03 | PARTIAL | Előbecslés kész; atomikus foglalás nincs |
| SC-04 | PARTIAL | Hiányzó adat DENY |
| SC-05 | BLOCKED | ThinkRail feladatdeduplikáció nem igazolt |
| SC-06 | PARTIAL | Nincs globális bootstrap |
| SC-07 | PASS-SCOPE | Az upstream szülőprojekt változatlan |
| SC-08 | BLOCKED | Éles koordinátori jogosultságkezelés nincs összekötve |
| SC-09 | PENDING | CFA4 kötelező tesztjei változatlanok |
| SC-10 | PARTIAL | Új modul csak vizsgált kimenet alapján értékelhető |
| SC-11 | BLOCKED | Élő prompt-injection és secret tesztek hiányoznak |
| SC-12 | BLOCKED | Atomikus ledger és helyreállítás hiányzik |
| SC-13 | PENDING | AIQT teljes szöveg szerinti validáció szükséges |
| SC-14 | PENDING | Szülő TC/AC kapuk külön ellenőrzendők |
| SC-15 | BLOCKED | Költség-összehasonlító baseline hiányzik |
| SC-16 | PARTIAL | Kikapcsolható konfiguráció; éles rollback nincs |

A `PASS-SCOPE` nem teljes funkcionális vagy kiadási PASS.

## S0–S6 és átvétel

- **S0:** mérhető BASELINE-COST-001. Enélkül továbblépés tiltott.
- **S1:** szelektív policy és biztonságos offline adapter.
- **S2:** COST-GATE-001 tényleges, atomikus szolgáltatói kikényszerítéssel.
- **S3:** deduplikált ThinkRail/Claude/EGM orkestráció.
- **S4:** Rust/Qt6/Python/FFmpeg és teljes CI változatlan megőrzése.
- **S5:** már jóváhagyott CFA4 feladatokon mért pilot; új fizetős
  kísérleti futtatás nélkül.
- **S6:** tulajdonosi élesítési döntés, ellenőrzött rollback.

Költségcél: új token- és végrehajtási költség legfeljebb a hasonló,
minőségében ellenőrzött korábbi feladaté. A 60–80% / 70–85% / 70–90%
megtakarítási arány csak célszám, nem tény. A kötelező tesztet
költségcsökkentés címén elhagyni tilos.
