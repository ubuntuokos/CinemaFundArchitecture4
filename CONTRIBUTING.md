# Contributing to FilmCraft

Thanks for helping. Before you start:

1. Read **[AGENTS.md](AGENTS.md)**. Its rules are absolute. In particular: no Adobe assets of any
   kind, every asset openly licensed with an attribution sidecar, codecs and containers written
   clean-room from specifications, reused GPL-3.0-compatible code elsewhere only with its notices kept and
   an entry in `THIRD_PARTY_CODE.md`, and ffmpeg only as an external test oracle.
2. **Never crash.** A crash loses someone's work, so this rule outranks feature work: no `unwrap()`,
   `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!` or `unsafe` in production code;
   return `Result` and treat every input as hostile. See [AGENTS.md](AGENTS.md) §0.
3. Follow **[docs/contributing.md](docs/contributing.md)**: setup per OS, building and running,
   the quality gates (`cargo xtask ci`), commit conventions, and how to add commands, effects,
   transitions, codecs, panels and assets.

More reading: [docs/architecture.md](docs/architecture.md), [docs/testing.md](docs/testing.md),
[docs/agents.md](docs/agents.md), [ROADMAP.md](ROADMAP.md).

Contributions are licensed under GPL-3.0-or-later, the project licence (see `LICENSE` and `NOTICE`).
