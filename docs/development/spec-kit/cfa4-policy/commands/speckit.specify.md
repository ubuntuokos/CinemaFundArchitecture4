---
description: "Create a feature specification under the CFA4 rules (cfa4-policy wrap)"
strategy: wrap
---

## CFA4 előfeltételek (cfa4-policy)

Mielőtt bármit írsz, olvasd be a `main` aktuális állapotából, **teljes egészében, nem csak a címsorokat**:

1. `.specify/memory/constitution.md`;
2. az `AGENTS.md` CFA4-szakaszát (a fájl elejétől a „CFA4 — rules for agents and contributors” címig) és a 0. pontját („Never crash”);
3. a `VISION.md` 2. („Alapelvek”) és 6. („Prioritási sorrend”) pontját;
4. a `docs/coordination.md` 2. pontját (szabályok minden fejlesztő munkamenetnek).

A constitution többi forrását (`docs/licensing.md`, `CLAUDE.md`, az `AGENTS.md` további részei) akkor olvasd be, ha a funkció licencet, függőséget, assetet, médiát vagy kódot érint. A specifikáció végén sorold fel, mit olvastál be.

Ha a kért funkció nem egy tulajdonos által jóváhagyott projekt jóváhagyott fázisához tartozik, állj meg, és kérdezd meg a tulajdonost. Ágat, commitot, issue-t vagy PR-t nem hozol létre.

{CORE_TEMPLATE}

## CFA4 lezárás (cfa4-policy)

A kész specifikációban a „CFA4-követelmény” blokk minden mezője ki legyen töltve, vagy PENDING legyen. A `validation_state` PENDING marad. A specifikáció végén legyen egy „Beolvasott CFA4-források” lista.
