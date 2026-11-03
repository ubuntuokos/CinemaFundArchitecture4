# CFA4 – Spec Kit constitution (hivatkozó fájl)

**Forrás-SHA:** `main` @ `<SHA>` · **Terv:** CFA4-DEV-SPEC-KIT-001 1.2, SK-01 · **Verzió:** 1.0.0

Ez a fájl **nem** tartalmaz CFA4-szabályt. A CFA4 szabályai egyetlen helyen állnak; minden Spec Kit-lépés előtt a `main` aktuális állapotából olvasd be őket, ebben a sorrendben:

1. [`AGENTS.md`](../../AGENTS.md) – szabályok, amelyeket soha nem szabad megszegni; elsőbbsége van.
2. [`VISION.md`](../../VISION.md) – a projekt célja és a prioritási sorrend.
3. [`docs/coordination.md`](../../docs/coordination.md) – ágak, PR-ok, beolvasztás (csak a gazda).
4. [`docs/licensing.md`](../../docs/licensing.md) – licencszabályok.
5. [`CLAUDE.md`](../../CLAUDE.md) – munkautasítások.

Ha ezek ellentmondanak egymásnak vagy ennek a fájlnak, állj meg, és kérj tulajdonosi döntést. Másolatot, régi összefoglalót vagy emlékezetből vett szabályt nem használsz forrásként.

## Csak a Spec Kitre vonatkozó szabályok

Ezek máshol nem szerepelnek; forrásuk a terv SK-07 pontja és a D1′, D7, D9, D10 döntés.

- **Tiltott parancsok:** `/speckit-implement`, `/speckit-taskstoissues`, `/speckit-constitution` (és `/speckit.*` alakjuk), valamint a `specify workflow run speckit`. Ez a fájl kézzel karbantartott; újragenerálni tilos.
- **Tiltott bővítés:** a `git` és a GitHub-issue extension, bármely külső extension, preset vagy bundle tulajdonosi döntés nélkül; Claude-eseménykezelők (`.claude/settings.json`).
- **Ág, commit, PR:** a Spec Kit nem nyit ágat, nem commitol, nem nyit issue-t vagy PR-t. Ezt a munkamenet végzi a `docs/coordination.md` szerint.
- **Hatókör:** csak egy tulajdonos által jóváhagyott projekt jóváhagyott fázisán belül dolgozol; más projekt feladatát nem veszed át.
- **Ellenőrzés:** a Converge és az Analyze eredménye PENDING. VERIFIED csak lefutott CFA4-ellenőrzés (teszt, CI) után lehet.
- **Követelményazonosítók:** minden specifikáció megőrzi a CFA4-követelmény azonosítóját és forrását (a spec-sablon „CFA4-követelmény” blokkja).
