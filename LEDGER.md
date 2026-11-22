# turtle — LEDGER (external memory / state file)

**New session? Read this whole file first**, then `README.md`, then run `git log --oneline` and `ls`. Everything needed to resume is in here. Do not redo validated work. Before handing off, update the "Current state" section and append a dated entry under Session log.

## Project snapshot
- Name: **turtle** — learning project for embedding Lua in Rust.
- Location: workspace dir `lmstudio/`; git repo rooted at `turtle/`, branch `main`, tracking `origin/main`.
- Stack: Rust 1.95, `mlua` v0.12.2 pinned with features `["lua54", "vendored"]`, `terminal_graphics` v1.1.0, `libc` v0.2 (POSIX raw `termios` / `poll` / `TIOCGWINSZ`, already resolved in `Cargo.lock`).
- Process rule (user-set): user validates each phase (`cargo run`) before authorizing the next; Phase 2+ are TBD until after Phase 1 is validated.

## Phase board
| Phase | Description                                                                                                                                    | Status                                                                   |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| 0     | Rust app runs inline Lua code that prints `hello, world`                                                                                       | ✅ built & smoke-tested locally — validated by user (2026-11-14)          |
| 1     | Load and execute a hardcoded `main.lua` file next to the project. Assumed location: CWD-relative `turtle/main.lua`, invoked from inside `turtle/` | ✅ built & smoke-tested locally — validated by user (2026-11-14)          |
| 2     | Interactive Lua REPL with `terminal_graphics` rendering                                                                                        | ✅ built & smoke-tested locally — validated by user (2026-11-18)         |
| 3 V1  | Lua-driven 2D tile grid world engine (`world.lua` *Tidekeep*, `examples/life.lua`, `examples/simple_room.lua`, raw-mode input, hot-reload)    | ✅ built & smoke-tested locally (2026-11-18) — validated by user (2026-11-22) |

## Current state (most recent)
- Phase 3 V1 is validated by the user (2026-11-22). Before moving to Phase 3 V2, `TURTLE_SPEC.md` (clean-room functional spec + evaluation rubric), `TURTLE_P3V1_ABI.md` (exact Phase 3 V1 host–script ABI conformance spec), and `RETROSPECTIVE_P3V1.md` (session narrative, terms of art, and worked prompting example) were created at the repository root.
- Phase 3 V1 is implemented across `src/model.rs`, `src/tty.rs`, `src/lua_host.rs`, `src/render.rs`, `src/main.rs`, `world.lua` (*Tidekeep*), `examples/life.lua`, and `examples/simple_room.lua` (plus user doc updates to `world.lua` in commit `e76905c`).
- CLI defaults to `world.lua` with `argv[1]` selecting alternate scripts and `--steps N` supporting bounded smoke runs; the Phase 2 REPL was removed as planned now that Phase 2 served its validation purpose.
- Rust owns `EntityStore` (seeded by `world.init(seed)` and mutated by `world.step(t, requests, entities)` delta patches), fixed `78×23` `terminal_graphics` rendering (`37×17` at `cell_width = 2`, `75×17` at `cell_width = 1`, transparent `bg = nil` inheritance + contrast guard), raw-mode `termios`/`poll` input with `Drop` + panic restore, and atomic Shadow-VM hot-reload back to `t = 0` on script edit (preserving frozen frames inside a red error overlay if mid-edit syntax errors occur).
- `main.lua` remains preserved at the repository root.
- All unit tests (`cargo test`: 13 passed), doc/lint gates (`cargo clippy`, `cargo doc`, LuaLS `--checklevel=Warning`, `comment_load.py --fail-over 0`), `--steps` runs, and PTY interactive + live hot-reload tests pass.

## How to run right now
```bash
cd turtle && cargo run
```
Launches **Tidekeep** (`world.lua`). Also try:
```bash
cd turtle && cargo run -- examples/life.lua
cd turtle && cargo run -- examples/simple_room.lua
```
Controls: `WASD` / `HJKL` / Arrows move (or pan in playerless worlds), `Space` / `.` wait, `e` / `Enter` act (`E` toggles cells in Life), `p` pause/play, `n` step once, `r` reset `t = 0` same seed, `R` reroll new seed, `Esc` dismiss overlay, `q` / `Ctrl-C` quit.

To run tests and bounded headless smoke frames:
```bash
cd turtle && cargo test && cargo run -- --steps 4
```

## Key decisions & gotchas (do not regress)
1. **Vendored Lua**: without the `vendored` feature mlua builds fail on machines lacking system liblua54 (`pkg-config` can't find `lua54.pc`). Keep features = `["lua54", "vendored"]`.
2. **mlua 0.12 API ≠ older tutorials**: there is no `Lua::run_string` anymore (it compiled-fails). The idiom is `lua.load(source).exec()?;` — a Chunk builder; use file/string sources for Phase 1+.
3. **File loading in 0.12**: `Lua::load_file(path)` does not exist. Read the file into a string and pass it to `lua.load(&source).exec()?;` — this is the safest cross-version approach.
4. Git identity set **locally only** in this repo: placeholder `jay <jay@localhost>` (global config was empty) — user may want to change it.
5. Commit style: one commit per significant change, brief imperative sentence. Cargo.lock is committed (binary project).
6. Do NOT start Phase 2 until the user validates Phase 1 and says go (explicit gate rule).
7. **`terminal_graphics` default pixel background is white**: `Pixel::default()` paints a black space on a white background (`Colour::White`). Always fill grids with `Colour::Black` via `Rect` or `Container` before drawing `Text`, or unused cells render as solid white blocks.
8. **Always call `Display::reserve_space()` before `Display::print()`**: `Display::render()` emits `\x1b[{height}F` (cursor up `height` rows), so omitting `reserve_space()` overwrites preceding terminal scrollback lines. In Phase 3's fixed `78×23` in-place canvas (`FRAME_HEIGHT = 23`), call `reserve_space()` once at startup and never exceed 23 rows on an 80×24 pane so line wrapping never breaks cursor-up redraws.
9. **`terminal_graphics::Text` is single-row and clips at edges**: strip `'\r'`, expand `'\t'` to spaces, split on `'\n'`, and wrap lines to width before drawing into `Display`; avoid wide 2-column Unicode/emoji glyphs because `Display` advances 1 cell per `char`.
10. **POSIX raw mode must keep `c_oflag` (`OPOST | ONLCR`) enabled**: do not use `cfmakeraw`, which disables output post-processing and breaks `Display::print()` alignment; clear only `ICANON | ECHO | ISIG | IEXTEN` on `c_lflag` and `IXON | ICRNL` on `c_iflag`, and always pair raw mode with a `Drop` guard + panic hook.
11. **LuaLS binary path on this machine**: `/home/jay/.local/bin/lua-language-server` uses `#!env bash` which fails under `bash`; invoke `/usr/local/src/lua-language-server.git/bin/lua-language-server --check=. --checklevel=Warning --configpath=.luarc.json` directly when running the Lua doc check.
12. **V2 ECS seam**: `model::EntityStore` (`EntityId -> Coord, tile, is_player`) owns entity storage independently of `lua_host`, `tty`, and `render` so V2 can evolve into a full ECS without rewriting the terminal or Lua ABI boundary.

## Next steps
- [x] Wait for user to validate Phase 3 V1 (`cargo run`, `cargo run -- examples/life.lua`, `cargo run -- examples/simple_room.lua`, and live `world.lua` editing).
- [ ] User runs external model evaluation against `TURTLE_SPEC.md` and `TURTLE_P3V1_ABI.md` (optional side quest before V2).
- [ ] On user go-ahead: plan and implement Phase 3 V2 (Entity Component System + next features).

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

### 2026-11-18 — session 4 (Phase 3 V1)
- Brainstormed architecture with Game Designer & UX Lead subagents; user validated Phase 2 and confirmed `world.lua` default + `argv[1]`, Rust-owned `EntityStore`, *Tidekeep* + `examples/life.lua` + `examples/simple_room.lua`, raw-mode `libc` input, and `hybrid` default pacing.
- Added `libc = "0.2"` (resolved offline to existing `0.2.191` in `Cargo.lock`) and implemented `src/model.rs`, `src/tty.rs`, `src/lua_host.rs`, `src/render.rs`, and `src/main.rs`.
- Authored `world.lua` (*Tidekeep* procedural archipelago with crate bridges, Sun Gems, and Reef Crabs), `examples/life.lua` (sparse Game of Life with camera pan & toggle), and `examples/simple_room.lua` (turn-based puzzle room).
- Verified `cargo test` (13 passed), `cargo clippy --all-targets -- -D warnings -D missing_docs`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`, LuaLS (`4/4` clean), `comment_load.py --fail-over 0` (`0` warnings across all 8 files), `--steps 4` runs, and PTY interactive + live Shadow-VM hot-reload tests.
- Updated `README.md` and `LEDGER.md`.

### 2026-11-22 — session 5 (Phase 3 V1 validation + evaluation specs)
- User validated Phase 3 V1 and requested clean-room + ABI conformance specs for evaluating another model prior to Phase 3 V2, plus a session retrospective.
- Created `TURTLE_SPEC.md` (clean-room functional requirements for the engine + all 3 world archetypes, V2 ECS modularity seam, automated checks, and human validation rubric; no crate/version prescriptions).
- Created `TURTLE_P3V1_ABI.md` (exact Phase 3 V1 host–script ABI specification matching unmodified `world.lua`, `examples/life.lua`, and `examples/simple_room.lua` for Stage 2 conformance testing).
- Created `RETROSPECTIVE_P3V1.md` (narrative summary, terms-of-art glossary, and simulated worked prompting example).
- Updated `README.md` and `LEDGER.md` with Phase 3 V1 validation status, `origin/main` tracking note, and links to the new docs.
