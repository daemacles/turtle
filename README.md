# turtle

A learning project about embedding **Lua** in **Rust** with [mlua](https://crates.io/crates/mlua) (pinned v0.12.x, Lua 5.4 backend). Work proceeds in small phases; the user validates each phase by running it before authorizing the next one.

## Phases

| Phase | Description                                                          | Status                                                        |
| ----- | -------------------------------------------------------------------- | ------------------------------------------------------------- |
| 0     | Rust app runs inline Lua code that prints `hello, world`             | ✅ done (2026-11-14) — validated by smoke test                |
| 1     | Load and execute a hardcoded `main.lua` file next to the project     | ✅ done (2026-11-14) — validated by smoke test                |
| 2     | Interactive Lua REPL with `terminal_graphics` rendering               | ✅ done (2026-11-15) — REPL loop, command dispatch, colored output |

## Phase 2: interactive Lua REPL

**Goal:** turtle becomes an interactive terminal program that:
- Renders a Lua REPL prompt and output using `terminal_graphics`.
- Accepts user input and executes Lua code.
- Keeps the same Phase 0/1 Lua embedding (mlua v0.12, Lua 5.4, vendored).

**Implementation:**
1. `terminal_graphics` added to `Cargo.toml`.
2. REPL loop: prompt (`turtle> `), read input, parse commands (`help`, `quit`), execute Lua via `lua.load(&source).exec()`.
3. `main.lua` remains a standalone script (Phase 1 valid).
4. README.md + LEDGER.md updated.

**Key fixes:**
- `run_repl` now calls `read_input()` exactly once per iteration (was duplicated).
- Commands are parsed **before** Lua execution; `help` and `quit` are dispatched.
- `Display` handle is passed through the rendering chain; `display.print()` flushes each frame.

### Usage

```bash
cd turtle && cargo run
```

Type `help` for commands, `quit` to exit, or enter Lua code at the prompt.

### Design notes
- `Display` is the terminal handle; `Container` holds `Text`/`Colour` elements.
- `Text` is rendered with a font; `Colour` provides foreground/background.
- The crate is designed for rendering arbitrary text/graphics; a REPL is a natural use case.

### Gate rule
- User validates each phase by running `cargo run` before authorizing the next.

## Tracking state

- [LEDGER.md](LEDGER.md) — agent state ledger: current phase, decisions/gotchas, next steps. Read this first when resuming after a context reset.
- Git history — one commit per significant change; `git log --oneline` for the trail.