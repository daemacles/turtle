# turtle

A learning project about embedding **Lua** in **Rust** with [mlua](https://crates.io/crates/mlua) (pinned v0.12.x, Lua 5.4 backend). Work proceeds in small phases; the user validates each phase by running it before authorizing the next one.

## Phases

| Phase | Description                                                          | Status                                                                 |
| ----- | -------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| 0     | Rust app runs inline Lua code that prints `hello, world`             | ✅ done (2026-11-14) — validated by smoke test                         |
| 1     | Load and execute a hardcoded `main.lua` file next to the project     | ✅ done (2026-11-14) — validated by smoke test                         |
| 2     | Interactive Lua REPL with `terminal_graphics` rendering              | ✅ built & tested (2026-11-17) — awaiting user validation (`cargo run`) |

## Phase 2: interactive Lua REPL

**Goal:** turtle becomes an interactive terminal program that:
- Renders a Lua REPL prompt, panels, output, and errors using `terminal_graphics`.
- Accepts user input and executes Lua expressions and statements with persistent global state.
- Keeps the same Phase 0/1 Lua embedding (`mlua` v0.12.2, Lua 5.4, `vendored`).

**Implementation:**
1. `terminal_graphics` v1.1 added to `Cargo.toml`.
2. REPL loop in `src/main.rs`: startup banner, prompt (`turtle> `), single `read_line` per turn, command parsing (`help`, `quit`, blank lines, or `Ctrl-D` EOF), and Lua execution via `mlua` Chunk API.
3. Expressions (`return <expr>` compile check) print their formatted return values (`tostring`), statements persist globals across turns, and Lua's global `print(...)` is captured into the rendered output frame.
4. Output, errors, and framed `help`/welcome panels render with ANSI colors via `terminal_graphics` (`Display`, `Container`, `Rect`, `Text`, and `Colour`).
5. `main.lua` remains intact and executable from the REPL via `dofile("main.lua")`.

### Usage

```bash
cd turtle && cargo run
```

Type `help` for commands, `quit` (or `Ctrl-D`) to exit, blank lines to re-prompt, or enter Lua expressions/statements at the `turtle> ` prompt.

Run the test suite with:

```bash
cd turtle && cargo test
```

### Design notes
- `Display` is the in-memory terminal cell grid; `Display::reserve_space()` allocates terminal rows before `Display::print()` flushes each frame so earlier scrollback is not overwritten.
- `Pixel::default()` uses a white background, so panels (`Container`) and output frames (`Rect`) explicitly paint `Colour::Black` backgrounds before drawing `Text` / custom `Graphic` blocks.
- Multi-line strings and tabs are expanded and wrapped to `PANEL_WIDTH` (72 cells) before entering `Display`, since `Text` draws a single horizontal line and clips past grid edges.

### Gate rule
- User validates each phase by running `cargo run` before authorizing the next.

## Tracking state

- [LEDGER.md](LEDGER.md) — agent state ledger: current phase, decisions/gotchas, next steps. Read this first when resuming after a context reset.
- Git history — one commit per significant change; `git log --oneline` for the trail.
