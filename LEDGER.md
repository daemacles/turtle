# turtle — LEDGER (external memory / state file)

**New session? Read this whole file first**, then `README.md`, then run `git log --oneline` and `ls`. Everything needed to resume is in here. Do not redo validated work. Before handing off, update the "Current state" section and append a dated entry under Session log.

## Project snapshot
- Name: **turtle** — learning project for embedding Lua in Rust.
- Location: workspace dir `lmstudio/`; git repo rooted at `turtle/`, branch `main`, no remote yet.
- Stack: Rust 1.95, `mlua` v0.12.2 pinned with features `["lua54", "vendored"]`, `terminal_graphics` v1.1.0.
- Process rule (user-set): user validates each phase (`cargo run`) before authorizing the next; Phase 2+ are TBD until after Phase 1 is validated.

## Phase board
| Phase | Description                                                                                                                                    | Status                                                                   |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| 0     | Rust app runs inline Lua code that prints `hello, world`                                                                                       | ✅ built & smoke-tested locally — validated by user (2026-11-14)          |
| 1     | Load and execute a hardcoded `main.lua` file next to the project. Assumed location: CWD-relative `turtle/main.lua`, invoked from inside `turtle/` | ✅ built & smoke-tested locally — validated by user (2026-11-14)          |
| 2     | Interactive Lua REPL with `terminal_graphics` rendering                                                                                        | ✅ built & smoke-tested locally (2026-11-17) — awaiting user validation   |
| 3–N   | TBD — 3 Phase 3 options proposed (2026-11-17); awaiting Phase 2 validation & user choice                                                       | 🚧 proposed — awaiting user go-ahead                                      |

## Current state (most recent)
- Phase 2 completed in `src/main.rs`: interactive Lua REPL with a single `read_line` per turn, `help` / `quit` / blank-line / `Ctrl-D` EOF handling, expression return formatting (`return <expr>` compile check with statement fallback), Lua global `print` capture, and `terminal_graphics` ANSI rendering (`Display`, `Container`, `Rect`, `Text`, `Colour`, and custom `TextBlock` implementing `Graphic`).
- `main.lua` remains preserved at the repository root and runs cleanly via `dofile("main.lua")` inside the REPL.
- Unit tests (`cargo test`), clippy/doc gates (`cargo clippy`, `cargo doc`, `comment_load.py`), and end-to-end REPL smoke tests pass locally.
- Docs: `README.md` + `LEDGER.md` updated to reflect Phase 2 completion.

## How to run right now
```bash
cd turtle && cargo run
```
Displays the welcome panel and `turtle> ` prompt. Type `help` for commands, `quit` (or `Ctrl-D`) to exit, blank lines to re-prompt, or Lua expressions/statements (including `dofile("main.lua")`).

To run the unit tests:
```bash
cd turtle && cargo test
```

## Key decisions & gotchas (do not regress)
1. **Vendored Lua**: without the `vendored` feature mlua builds fail on machines lacking system liblua54 (`pkg-config` can't find `lua54.pc`). Keep features = `["lua54", "vendored"]`.
2. **mlua 0.12 API ≠ older tutorials**: there is no `Lua::run_string` anymore (it compiled-fails). The idiom is `lua.load(source).exec()?;` — a Chunk builder; use file/string sources for Phase 1+.
3. **File loading in 0.12**: `Lua::load_file(path)` does not exist. Read the file into a string and pass it to `lua.load(&source).exec()?;` — this is the safest cross-version approach.
4. Git identity set **locally only** in this repo: placeholder `jay <jay@localhost>` (global config was empty) — user may want to change it.
5. Commit style: one commit per significant change, brief imperative sentence. Cargo.lock is committed (binary project).
6. Do NOT start Phase 2 until the user validates Phase 1 and says go (explicit gate rule).
7. **`terminal_graphics` default pixel background is white**: `Pixel::default()` paints a black space on a white background (`Colour::White`). Always fill grids with `Colour::Black` via `Rect` or `Container` before drawing `Text`, or unused cells render as solid white blocks.
8. **Always call `Display::reserve_space()` before `Display::print()`**: `Display::render()` emits `\x1b[{height}F` (cursor up `height` rows), so omitting `reserve_space()` overwrites preceding terminal scrollback lines. When prompting inline on an interactive TTY, follow `flush_display` with `\x1b[1F\x1b[{width}C` so typed input sits on the prompt line.
9. **`terminal_graphics::Text` is single-row and clips at edges**: strip `'\r'`, expand `'\t'` to spaces, split on `'\n'`, and wrap lines to width before drawing into `Display` so multi-line outputs and tracebacks render cleanly without corrupting columns.

## Next steps
- [ ] Wait for user to validate Phase 2 (`cargo run` interactive REPL) and select one of the 3 proposed Phase 3 options (or adjust scope).
- [ ] Record the selected Phase 3 scope in this ledger before implementing Phase 3.

## Session log (append-only)
### 2026-11-13 — session 1 (setup + Phase 0)
- Created repo on branch `main`; local git identity placeholder set.
- Scaffold commit: mlua 0.12.2 dep, `.gitignore` (`/target`).
- First build failed (no system liblua54 found via pkg-config) → added `vendored` feature; builds clean after that.
- Phase 0 smoke-tested OK (`hello, world`, exit 0); committed d0424b3.
- Wrote README.md + LEDGER.md (their commit follows this entry).

### 2026-11-14 — session 2 (Phase 1)
- Created `main.lua` in repo root; updated `src/main.rs` to read file into a string and pass to `lua.load(&source).exec()?`.
- Smoke-test OK (`main.lua loaded and executed successfully`, `Phase 1 validated`, exit 0); committed 4863170.
- Updated README.md + LEDGER.md to reflect Phase 1 completion.

### 2026-11-17 — session 3 (Phase 2 completion)
- Resumed interrupted Phase 2 WIP (`4330771` + unstaged annotation pass in `src/main.rs`).
- Fixed duplicate `read_input()` in `run_repl`, wired `quit`/EOF/`help`/blank-line dispatch, captured Lua `print(...)` and expression return values (`return <expr>` compile check with statement fallback), and completed `terminal_graphics` rendering with `reserve_space()`, explicit `Colour::Black` backgrounds (`Container`/`Rect`), custom `TextBlock` (`Graphic`), and tab/newline wrapping.
- Added unit tests in `src/main.rs` (`cargo test`: 7 passed) and verified `cargo clippy`, `cargo doc`, `comment_load.py` (0 warnings), and end-to-end piped & PTY REPL smoke runs.
- Updated `README.md` and `LEDGER.md`; prepared 3 interactive Phase 3 proposals combining `mlua` + `terminal_graphics` for user selection.
