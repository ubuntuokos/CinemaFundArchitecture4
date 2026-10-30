# SkillSpector használata Claude Code-ból

## Cél

A CFA4 projekt-szintű Claude Code-skillje a külső agent skill-ek felhasználás előtti felülvizsgálatát segíti. A parancs kézi indítású:

    /cfa4-skill-review <helyi skill-útvonal>

A skill nem telepíti és nem futtatja a célkódot, nem írja át a célkönyvtárat, és nem indít külön Claude CLI-szolgáltatót. A Claude Code a meglévő munkamenetben végzi a forrásalapú áttekintést.

## Támogatott környezet

- Ubuntu 26.04 LTS és Ubuntu-változatai, amd64.
- Python 3.12, 3.13 vagy 3.14, saját virtuális környezetben.
- SkillSpector 2.13.2, rögzített Python-csomagverzió.
- Claude Code projekt-skill útvonala: `.claude/skills/cfa4-skill-review/SKILL.md`.

A SkillSpector nem CFA4 futásidejű vagy kiadási függőség. Nincs telepítve rendszer-Pythonba, nem kerül a CFA4 .deb/Flatpak/AppImage/tar csomagokba, és nem változtatja meg a CFA4 vagy az upstream szülőprojekt telepítéseit.

## Kifejezett telepítés

A telepítés csak akkor történik meg, ha a fejlesztő külön elindítja a telepítőparancsot:

    bash scripts/dev/install-skillspector.sh

A SkillSpector **nincs fent a PyPI-n**. A szkript a GitHubon közzétett v2.13.2 címke commitját (`abd82935b67f765a9541013c0866c6349251dcdf`) telepíti egy elkülönített venv-be (`pip install "skillspector @ git+https://github.com/NVIDIA/SkillSpector.git@<commit>"`); ehhez `git` kell. Ha a telepítés nem fejeződik be, a szkript csak az általa ebben a futásban létrehozott venv-et törli. Alapértelmezett helye:

    ${XDG_DATA_HOME:-~/.local/share}/cfa4/tools/skillspector/2.13.2/venv

A CFA4_SKILLSPECTOR_HOME változóval a cfa4/tools gyökérkönyvtár külön megadható. A szkript nem használ sudo-t, nem aktiválja a venv-et a shellben, nem frissít meglévő telepítést, és a célkönyvtáron kívül nem ír.

A SkillSpector commitra rögzített, de a PyPI-ről telepített tranzitív Python-függőségei nem mind verzió- és hash-rögzítettek ebben az első változatban. Ezért ez fejlesztői, egyedi használatú eszköz; nem szabad a CFA4 kiadási csomagjába vagy általános CI-kapujába tenni. Telepítés után a pip check lefut, majd a CLI verziója megjelenik.

## Kézi CLI-használat

A lokális célkönyvtár vizsgálata a wrapperen keresztül történik:

    bash scripts/dev/skillspector.sh scan /abszolut/utvonal/a/skillhez --no-llm --fail-on-incomplete --format json --output /tmp/skill-review.json

A parancs a rögzített, elkülönített venv binárisát használja; nem esik vissza a PATH-on talált másik skillspectorra. Ha a bináris hiányzik, a wrapper leáll, és megadja a kifejezett telepítési parancsot. A telepítést nem indítja el.

Az online OSV.dev ellenőrzés a függőségi neveket és verziókat továbbíthatja. Offline vagy részleges elemzés nem tekinthető teljes ellenőrzésnek. A szkenner nem futtatja a vizsgált skillt; ez nem sandbox, és nem bizonyítja, hogy a skill futtatáskor biztonságos.

## Adatkezelés

Az alapbeállítás LLM nélküli szkennelés. Claude Code forrásellenőrzésekor a vizsgált fájlok tartalma a Claude-munkamenet része lesz. A skill nem állít be ANTHROPIC_API_KEY-t, nem választja át a SKILLSPECTOR_PROVIDER értéket, és nem használja a SkillSpector claude_cli szolgáltatóját. A külön Claude CLI-hívások többletköltsége így elkerülhető.

A jelentést egyedi, védett ideiglenes fájlba kell írni, a felülvizsgálat végén pedig csak ezt a jelentésfájlt szabad törölni. A nyers jelentés ne kerüljön automatikusan commitba vagy megosztott CI-artifaktumba.

## Verdict- és hibaszabályok

- APPROVE csak teljes statikus lefedettség és lezárt forrásfelülvizsgálat esetén adható.
- CAUTION emberi döntést igényel; ide tartozik minden hiányos vagy nem futott szkennelés.
- REJECT indokolt, ha a forrás rosszindulatú vagy megtévesztő, illetve megmagyarázatlan kritikus viselkedést mutat.
- A SkillSpector pontszámát és ajánlását együtt kell olvasni a konkrét bizonyítékokkal. Az alapértelmezett folyamat kifejezetten hiányos elemzésnél sikertelen kilépési kódot kér.
- A skill tartalmát és parancsfájljait nem szabad futtatni vagy módosítani. Baseline kivétel nem készül automatikusan.

## Telepítés frissítése

A verzióváltás külön felülvizsgálatot igényel: ellenőrizni kell az új kiadást, Python-tartományt, licencet és függőségeket; módosítani kell a verziózott telepítési útvonalat, a tesztet és ezt a dokumentumot; majd Ubuntu 26.04/amd64-en validálni kell. Nincs automatikus frissítés.

## Validációs feljegyzés (2026-10-30)

Környezet: **sandbox, Ubuntu 24.04.5, Python 3.13.18, x86_64 – nem a célgép.** A célgépes (Ubuntu 26.04) futtatás és a Claude Code-skill felismerése: **PENDING**.

| Ellenőrzés | Eredmény |
|---|---|
| Telepítés `pip install skillspector==2.13.2` (az első változat) | FAIL: a csomag nincs a PyPI-n |
| Telepítés a rögzített GitHub-commitról | OK: `SkillSpector v2.13.2`, `pip check`: nincs hiba |
| Ismételt futtatás | OK: a meglévő, egyező verziót nem módosítja |
| Megszakadt telepítés (elérhetetlen csomagindex) | OK: kilépési kód 1, félkész venv nem marad |
| `scan .claude/skills/cfa4-skill-review --no-llm --fail-on-incomplete` | kilépési kód 1; score 8, LOW, ajánlás CAUTION; lefedettség 100%, státusz `partial` (a skill a repó saját fájljaira hivatkozik, ezek nincsenek a skillkönyvtárban); 1 MEDIUM találat (AS3, 9. sor): a forrásmegjelölésben szereplő upstream útvonal – kézi felülvizsgálat szerint nem skill-felsorolás |
| `scan .agents/skills/cfa4-elephant --no-llm --fail-on-incomplete` | kilépési kód 1; score 0, LOW, CAUTION; lefedettség 100%, státusz `partial` (ugyanezen okból); találat nincs |

A `partial` státusz és a CAUTION a tervezett, biztonságos viselkedés: hiányos elemzésre a folyamat nem ad APPROVE-ot.

## Forrás és licenc

A CLI a NVIDIA SkillSpector v2.13.2 kiadását rögzíti (tag commit: abd82935b67f765a9541013c0866c6349251dcdf). A CFA4-skill a projekt Skill Inspector útmutatójának magyar, CFA4-specifikus adaptációja a 54921c8c7c23008c8af2328d5ab492a904b6ece4 revízióból. Az upstream licenc Apache-2.0; a CFA4-ben található licencszöveg a LICENSE-APACHE fájlban, az átvétel nyilvántartása a docs/licensing.md fájlban van.
