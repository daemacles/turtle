# turtle

A learning project about embedding **Lua** in **Rust** with [mlua](https://crates.io/crates/mlua) (pinned v0.12.x, Lua 5.4 backend). Work proceeds in small phases; the user validates each phase by running it before authorizing the next one.

## Phases

| Phase | Description                                                          | Status                                                        |
| ----- | -------------------------------------------------------------------- | ------------------------------------------------------------- |
| 0     | Rust app runs inline Lua code that prints `hello, world`             | ✅ done (2026-11-14) — validated by smoke test                |
| 1     | Load and execute a hardcoded `main.lua` file next to the project     | ✅ done (2026-11-14) — validated by smoke test                |
| 2     | Interactive Lua REPL with `terminal_graphics` rendering               | 🚧 proposed — awaiting user go-ahead                          |

## Phase 2 proposal: interactive Lua REPL

**Goal:** turtle becomes an interactive terminal program that:
- Renders a Lua REPL prompt and output using `terminal_graphics`.
- Accepts user input (readline-style) and executes Lua code.
- Keeps the same Phase 0/1 Lua embedding (mlua v0.12, Lua 5.4, vendored).

**Scope to implement (Phase 2):**
1. Add `terminal_graphics` to `Cargo.toml`.
2. Build a minimal REPL loop:
   - Print a prompt (e.g. `turtle> `) with colors.
   - Read a line of input.
   - Execute the line via mlua (safe sandboxing: `lua.load(&source).exec()`).
   - Print Lua output/errors with colors.
3. Keep `main.lua` as a standalone script (Phase 1 remains valid).
4. Update README.md + LEDGER.md.

**Design notes from docs.rs:**
- `Display` is the terminal handle; `Container` holds `Text`/`Colour` elements.
- `Text` is rendered with a font; `Colour` provides foreground/background.
- The crate is designed for rendering arbitrary text/graphics; a REPL is a natural use case.

### Before I start, I need your go-ahead

Please confirm one of the following:

**A. Go ahead with Phase 2 as proposed** (interactive Lua REPL with `terminal_graphics`).

**B. Adjust the scope** (e.g., add a Lua command history, colored syntax highlighting, a Lua `print` redirect, or a Lua REPL prompt that uses Lua's own `io.read` instead of Rust's `stdin`).

**C. Pause Phase 2** and instead do something else (e.g., a Lua script runner, a Lua module loader, a Lua debugger, or a Lua-based config system).

Tell me which option you prefer, and I'll implement it. I'll keep the gate rule: you validate each phase before I proceed to the next.

## Tracking state

- [LEDGER.md](LEDGER.md) — agent state ledger: current phase, decisions/gotchas, next steps. Read this first when resuming after a context reset.
- Git history — one commit per significant change; `git log --oneline` for the trail.
