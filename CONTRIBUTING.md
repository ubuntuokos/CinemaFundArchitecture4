# Contributing to CFA4

Thanks for helping. Before you start:

1. Read **[AGENTS.md](AGENTS.md)**. Its rules are absolute. In particular: no Adobe assets of any
   kind, every asset openly licensed with an attribution sidecar, reused code only under
   [docs/licensing.md](docs/licensing.md) (GPL-3.0-compatible, recorded, no GPL-2.0-only), and ffmpeg only
   as an external test oracle.
2. **Never crash.** A crash loses someone's work, so this rule outranks feature work: no `unwrap()`,
   `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!` or `unsafe` in production code;
   return `Result` and treat every input as hostile. See [AGENTS.md](AGENTS.md) §0.
3. Follow **[docs/contributing.md](docs/contributing.md)**: setup per OS, building and running,
   the quality gates (`cargo xtask ci`), commit conventions, and how to add commands, effects,
   transitions, codecs, panels and assets.

More reading: [docs/architecture.md](docs/architecture.md), [docs/testing.md](docs/testing.md),
[docs/agents.md](docs/agents.md), [ROADMAP.md](ROADMAP.md).

Contributions are licensed under GPL-3.0-or-later, the project licence (see [LICENSE](LICENSE) and
[docs/licensing.md](docs/licensing.md)). Code inherited from the upstream project (see NOTICE) keeps its MIT OR Apache-2.0 notices.
