# turtle

A learning project about embedding **Lua** in **Rust** with [mlua](https://crates.io/crates/mlua) (pinned v0.12.x, Lua 5.4 backend). Work proceeds in small phases; the user validates each phase by running it before authorizing the next one.

## Phases

| Phase | Description                                                          | Status                                                        |
| ----- | -------------------------------------------------------------------- | ------------------------------------------------------------- |
| 0     | Rust app runs inline Lua code that prints `hello, world`             | ✅ done (2026-11-13) — awaiting user validation               |
| 1     | Load and execute a hardcoded `main.lua` file next to the project     | planned — starts after Phase 0 is validated                    |
| 2–N   | TBD — user defines these once Phase 1 is done                        | —                                                              |

## Running (current phase: Phase 0)

Requires only the Rust toolchain (`cargo`). No system Lua installation is needed — Lua 5.4 is compiled statically into the binary via mlua's `vendored` feature.

```bash
cd turtle
cargo run
```

Expected output, exactly one line:

```
hello, world
```

## Tracking state

- [LEDGER.md](LEDGER.md) — agent state ledger: current phase, decisions/gotchas, next steps. Read this first when resuming after a context reset.
- Git history — one commit per significant change; `git log --oneline` for the trail.
