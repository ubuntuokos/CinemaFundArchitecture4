# cfa4-policy – Spec Kit preset (forrás)

A CFA4-DEV-SPEC-KIT-001 1.2 terv SK-F2 fázisának terméke. **Inert forrás**: itt semmi nem töltődik be munkamenetbe, és ez nem a `.specify/` könyvtár (D7: Spec Kit-fájl a `main`-en csak az SK-F7-ben).

| Fájl | Szerep |
| --- | --- |
| `constitution.md` | Hivatkozó constitution (SK-01). Telepítéskor a `.specify/memory/constitution.md` helyére kerül, a `<SHA>` helyett a `main` aktuális rövid SHA-jával. Kézzel karbantartott; a `/speckit-constitution` tiltott. |
| `templates/spec-template-cfa4.md` | A spec-sablon végére fűzött kötelező „CFA4-követelmény” blokk (SK-02, `append`). |
| `commands/speckit.specify.md` | A `/speckit-specify` köré tett CFA4-előfeltételek és -lezárás (`wrap`). |
| `preset.yml` | A preset leírója (Spec Kit ≥ 1.2.5). |

## Telepítés eldobható worktree-ben vagy a pilotágon (SK-F2-ben mérve)

```sh
specify init --here --force --non-interactive --integration claude --script sh \
  --integration-options="--no-model-invocation --events false" \
  --preset docs/development/spec-kit/cfa4-policy
sed "s/<SHA>/$(git rev-parse --short HEAD)/" docs/development/spec-kit/cfa4-policy/constitution.md \
  > .specify/memory/constitution.md
```

**A presetet ne a `specify preset add`/`update` paranccsal telepítsd.** A v1.2.5-ben ezek a skilleket az alapértelmezéssel írják újra, és a felülírt `speckit-specify` fejlécébe `disable-model-invocation: false` kerül. Ha mégis ezzel települt, futtasd újra a fenti `specify init`-et. Minden preset-művelet után ellenőrizd, hogy mind a 10 `.claude/skills/speckit-*/SKILL.md` fejlécében `disable-model-invocation: true` áll (AC-SK-16).

A `--force` csak eldobható worktree-ben vagy a pilotágon használható, utána kötelező a teljes diff; meglévő fájl módosulása = leállás (D10).
