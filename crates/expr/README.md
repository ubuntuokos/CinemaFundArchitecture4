# cfa4-expr

The expression engine of CFA4-VFX-001 VFX-4 (decision D-2: one expression core). After Effects-style
property expressions on the boa JavaScript engine; the crate knows no project model, it reads other
properties only through the `Host` trait.

- **Language**: JavaScript plus the legacy array maths (`value + [10, 0]`), by a source rewrite
  (`rewrite.rs`). **Object model** (`src/prelude.js`): `time`, `value`, `index`, `thisProperty`,
  `prop(path)`, `valueAtTime`, `wiggle`, `loopIn/Out`, `smooth`, `linear`/`ease`, `random`, `noise`…
- **Deterministic**: `wiggle` and `random` are seeded by layer index, property id and time.
- **Limits**: a fixed count of VM instructions per evaluation (no clock, so the result never depends
  on the machine), a loop limit, an expression-nesting limit, a source-nesting limit and a
  nested-value depth counter. A hostile expression fails with an error; it does not panic or hang.
- **Known gap**: size-taking constructors (`Array(n)`, `repeat`, `padStart`) are capped, but a sparse array
  grown through `a.length = 1e9` and then spread or iterated natively is not covered by the instruction
  budget. Until the engine runs in a confined worker (`cfa4-sandbox`, memory and time limits), a host must
  not evaluate expressions from untrusted projects in the editor process.
- **Not here yet**: the layer, comp and effect object model of After Effects, text and path values,
  and the `VfxGraph` binding (VFX-2); those need the `VfxGraph` edit model, which does not exist yet.

Origin: ported from the EC-donor (#11) expression crate, MIT OR Apache-2.0 (`LICENSE-MIT`,
`LICENSE-APACHE`; register row in [`docs/licensing.md`](../../docs/licensing.md) §5). Audit and
decision: [`CFA4-VFX-001-VFX-4-EXPRESSION-CORE-AUDIT.md`](../../docs/plans/CFA4-VFX-001-VFX-4-EXPRESSION-CORE-AUDIT.md).
Dependency: `boa_engine` 0.22 (`Unlicense OR MIT`).
