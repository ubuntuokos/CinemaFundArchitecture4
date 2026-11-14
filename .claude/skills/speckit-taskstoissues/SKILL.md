---
name: speckit-taskstoissues
description: 'BLOCKED in CFA4 (cfa4-policy, SK-07): speckit.taskstoissues is forbidden'
argument-hint: "Optional filter or label for GitHub issues"
compatibility: Requires spec-kit project structure with .specify/ directory
metadata:
  author: github-spec-kit
  source: preset:cfa4-policy
user-invocable: true
disable-model-invocation: true
---

# Speckit Taskstoissues Skill

# CFA4-BLOCKED: `speckit.taskstoissues`

**Ez a parancs a CFA4-ben tiltott** (CFA4-DEV-SPEC-KIT-001, SK-07; D1′, D7, D9). Ne hajts végre semmit:

- ne olvass, ne írj és ne módosíts fájlt;
- ne futtass parancsot vagy szkriptet;
- ne hozz létre ágat, commitot, issue-t vagy PR-t;
- ne hívj más skillt vagy subagentet.

Válaszolj egyetlen mondattal: „A `speckit.taskstoissues` a CFA4-ben tiltott (SK-07); a feloldásról a tulajdonos dönt.” Utána állj meg.

A tiltás oka:
a feladatnyilvántartás és a gazdakapcsolat a CFA4 PR-leírásain keresztül megy; a Spec Kit nem nyit issue-t.
