# cfa4-expr

CFX, the CFA4 restricted spatial expression language (plan
[`CFA4-AX-INTERPRETER-001`](../../docs/plans/CFA4-AX-INTERPRETER-001-0.1.md), phase X1; language
reference [`docs/expr-language.md`](../../docs/expr-language.md)).

A tiny typed, deterministic language that gives a value to one voxel or one point. Pure Rust, no
`unsafe`, no dependencies, no JIT.

- **No I/O in the language** (no files, network, processes, clock, environment), no recursion, only
  loops with compile-time bounds, no dynamic allocation while an element runs.
- **Fuel:** every bytecode instruction costs one unit; a run stops with `R-FUEL` (default 10 000,
  hard ceiling 1 000 000 per element).
- **Never panics:** every error is a `Diagnostic` with a stable code and a line/column; hostile
  sources (deep nesting, huge literals, 64 KiB+) are rejected by explicit limits (`E-LIMIT-*`).
- **Deterministic:** a run depends only on its inputs, not on thread count or timing (tested with
  4 threads against one). Library math (`sin`, `exp`…) is the platform's, so results are bit-identical
  on one build and platform, not across platforms.

```text
source ──lex──▶ tokens ──parse──▶ AST ──check+codegen──▶ Program ──run(Vm, fuel, Env, params, inputs, outputs)
```

## Use

```rust
let schema = Schema { context: Context::Volume, attrs: vec![("density".into(), Ty::Float), ("mask".into(), Ty::Float)] };
let program = cfa4_expr::compile("reads @density; writes @mask; @mask = @density > 0.5 ? 1.0 : 0.0;", &schema)?;
let mut vm = program.new_vm();               // reuse it for every element
let stats = program.run(&mut vm, DEFAULT_FUEL, &env, &[], &inputs, &mut outputs)?;
```

## Not in X1

Reading and writing real voxel data: done in X2 (`cfa4-vdb` `Worker::apply_expression`, engine `expr.run_preview`). Engine commands and project storage: done in X3 (`expr.check/save/list/get/delete/approve/revoke/bake/export/import`,
project schema v18, approval per machine and never in the project file). Not yet: the Qt editor (X4), neighbour access (v2, decision Q-X1). The VM's speed is **not yet
measured** (plan §7, X1/X2 on the reference machine).

## Tests

`cargo test -p cfa4-expr`: every example of the language reference, the error-code and position
table, integer and NaN rules, fuel, short-circuiting, parameters, noise properties, limits against
stack overflow, a threads-versus-single determinism check, and 4000 deterministic mutations of
valid programs that must never panic. The fuzz target is `fuzz/fuzz_targets/expr.rs`.
