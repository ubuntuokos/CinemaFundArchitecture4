# cfa4-mi

Media Intelligence core (layer L2): pure logic with no database. Plan:
`CFA4-MEDIA-INTELLIGENCE-001` (FINAL 2.0, [`docs/plans/CFA4-MEDIA-INTELLIGENCE-001-FINAL-2.0.md`](../../docs/plans/CFA4-MEDIA-INTELLIGENCE-001-FINAL-2.0.md)).

- `hash`: the media identity of the index. The default is the **sampled SHA-256** `sha256s1`
  (whole file up to 16 MiB, otherwise 16 blocks of 1 MiB: first, last and 14 spread evenly; rule
  frozen under the name `s1`, a different rule gets a new tag). The full `sha256` is a separate,
  optional algorithm. `MediaIdentity` in `cfa4-project` is unchanged: it stays the relink key.
- `anchor`: `TimeAnchor`, a half-open `[start, end)` range of exact `Tick`s with the relations the
  query planner needs (`overlaps`, `within`, `before`, `after`, `before_within`, `intersection`).
- `provenance`: the eight provenance classes (`HUMAN`, `AI_GENERATED`, …). Machine data is never
  shown as verified.
- `error`: `MiError`; every failure is a value, never a panic.

The `mi` PostgreSQL schema will live in `cfa4-store` and implement the traits this crate grows; the
engine reaches the index only through them (the same inversion as `cfa4-tasks` and its
`TaskBackend`). Hostile input (truncated or changed media, bad hash text, overflowing ranges) is an
error, covered by tests.
