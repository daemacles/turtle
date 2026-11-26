# turtle — LEDGER (external memory / state file)

**New session? Read this whole file first**, then `README.md`, then run `git log --oneline` and `ls`. Everything needed to resume is in here. Do not redo validated work. Before handing off, update the "Current state" section and append a dated entry under Session log.

## Project snapshot
- Name: **turtle** — learning project for embedding Lua in Rust.
- Location: workspace dir `lmstudio/`; git repo rooted at `turtle/`, branch `main`, tracking `origin/main` (`git@github.com:daemacles/turtle.git`).
- Stack: Rust 1.95, `mlua` v0.12.2 pinned with features `["lua54", "vendored"]`, `terminal_graphics` v1.1.0, `libc` v0.2 (POSIX raw `termios` / `poll` / `TIOCGWINSZ`, already resolved in `Cargo.lock`).
- Process rule (user-set): user validates each phase (`cargo run`) before authorizing the next.

## Phase board
| Phase | Description                                                                                                                                    | Status                                                                   |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| 0     | Rust app runs inline Lua code that prints `hello, world`                                                                                       | ✅ built & smoke-tested locally — validated by user (2026-11-14)          |
| 1     | Load and execute a hardcoded `main.lua` file next to the project. Assumed location: CWD-relative `turtle/main.lua`, invoked from inside `turtle/` | ✅ built & smoke-tested locally — validated by user (2026-11-14)          |
| 2     | Interactive Lua REPL with `terminal_graphics` rendering                                                                                        | ✅ built & smoke-tested locally — validated by user (2026-11-18)         |
| 3 V1  | Lua-driven 2D tile grid world engine (`world.lua` *Tidekeep*, `examples/life.lua`, `examples/simple_room.lua`, raw-mode input, hot-reload)    | ✅ built & smoke-tested locally — validated by user (2026-11-22)          |
| 3 V2  | Hand-rolled sparse-set ECS (`src/ecs.rs`), scoped `mlua::UserData` Host API, live resize & alt-screen UX, live ECS inspector, `docs/ECS_INTRO.md` | ✅ built & smoke-tested locally — validated by user (2026-11-25)          |
| 3 V3  | Richer *Tidekeep* gameplay & ECS systems (`world.lua`: `hp`, `combat`, `pickup`/`inventory`, `status` expiry, `ai` FSM, `bridge` entities; entity bg inheritance in `src/render.rs`) | ✅ built & smoke-tested locally — validated by user (2026-11-25)          |

## Current state (most recent)
- Phase 3 V3 validated by user (`2026-11-25`), committed, and pushed to `origin/main` (implemented across `world.lua`, `src/render.rs` opaque entity background propagation, and `src/lua_host.rs` V3 integration tests).
- Phase 3 V2 validated by user on 2026-11-25, committed on `main` (`f830d10`), tagged as `P3V2` (pushed to `origin`), and merged with `origin/main` (`e26b9c4`).
- Phase 3 V1 validated and tagged as `P3V1` on `e76905c` (pushed to `origin`); evaluation specs and retrospective (`TURTLE_SPEC.md`, `TURTLE_P3V1_ABI.md`, `RETROSPECTIVE_P3V1.md`) preserved at the repository root.
- Rust owns a pure-Rust hand-rolled generational `SparseSet<T>` ECS (`EcsWorld` in `src/ecs.rs`) with core typed components (`Position`, `Sprite` z-layer, `Name`, `Player`, `Solid`) plus dynamic `ScriptValue` stores, exposed to migrated Lua scripts via scoped `mlua::UserData` (`world.init(seed, ecs)`, `world.step(t, requests, ecs)`) with transactional step rollback on Lua errors.
- *Tidekeep* (`world.lua`) runs an ordered ECS pipeline (`status_tick_system` -> `player_intent_system` -> `ai_system` -> `hud_system`) exercising melee combat (`hp`/`combat`), collectible pickups (`tonic`, `spear`), Salt Flares (`e`), crate slams, timed `status` expiry/pruning, crab AI states (`patrol`/`chase`/`flee`/`stunned`), and stacked `bridge` entities (`layer = 1`).
- `main.lua` remains preserved at the repository root.
- All verification gates pass: `cargo test` (21 passed), `cargo clippy --all-targets -- -D warnings -D missing_docs`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`, LuaLS (`4/4` clean), `comment_load.py --fail-over 0` (`0` warnings across all 9 code files), `--steps 4` runs (`world.lua`, `examples/life.lua`, `examples/simple_room.lua`), and PTY interactive tests.

## How to run right now
```bash
cd turtle && cargo run
```
Launches **Tidekeep** (`world.lua`). Also try:
```bash
cd turtle && cargo run -- examples/life.lua
cd turtle && cargo run -- examples/simple_room.lua
```
Controls: `WASD` / `HJKL` / Arrows move (or pan in playerless worlds), `Space` / `.` wait, `e` / `Enter` act (`E` toggles cells in Life), `Tab` / `i` toggle ECS Inspector, `[` / `]` cycle selected entity, `p` pause/play, `n` step once, `r` reset `t = 0` same seed, `R` reroll new seed, `Esc` dismiss overlay, `q` / `Ctrl-C` quit.

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
11. **LuaLS binary path on this machine**: invoke `/mnt/win_f/llms/tools/lua-language-server/bin/lua-language-server --check=. --checklevel=Warning --configpath=.luarc.json` directly when running the Lua doc check.
12. **V2 hand-rolled ECS & Lua handle packing (`src/ecs.rs`, `src/lua_host.rs`)**: `EntityId` packs `(generation << 32) | index` into a positive signed 64-bit Lua integer (`generation <= i32::MAX`) so Lua gets native value equality and `t[id]` table keys without userdata pointer-identity traps, while Rust catches stale handles in `O(1)`.
13. **Scoped `mlua::UserData` + transactional step rollback**: `LuaWorld::init` and `LuaWorld::step` pass `EcsWorld` via `lua.scope(|s| s.create_any_userdata_ref_mut(&mut working_ecs))` so leaked handles expire immediately on return, and `step` clones `current_ecs` beforehand so mid-step Lua errors roll back cleanly.
14. **Alternate screen, `SIGWINCH`, and cursor anchor (`src/tty.rs`)**: interactive TTYs enter `\x1b[?1049h` (exiting in `Drop` + panic hook) and position at `\x1b[{height + 1};1H` inside `\x1b[?2026h`…`l` before `Display::write_to`, because `Display::render()` always leads with `\x1b[{height}F`. Non-TTY `--steps` runs keep plain `reserve_space()` + `print()`. `SIGWINCH` uses `sigaction` with `sa_flags = 0` so `poll()` wakes on resize immediately.
15. **ECS primer**: `docs/ECS_INTRO.md` documents ECS concepts, problems/friction, turtle's sparse-set design, and mapping to `hecs`/`bevy_ecs`.
16. **`terminal_graphics::Container` does not clear cell characters**: `Container::draw` only sets `pixel.set_background(...)` and leaves underlying glyphs and foreground colors intact. Always paint an opaque `Rect::new(..., Colour::Black)` before drawing any modal `Container` over map terrain or entities (centralized in `render::draw_opaque_modal`).

## Next steps
- [x] Wait for user to validate Phase 3 V1 (`cargo run`, `cargo run -- examples/life.lua`, `cargo run -- examples/simple_room.lua`, and live `world.lua` editing) — validated 2026-11-22.
- [ ] User runs external model evaluation against `TURTLE_SPEC.md` and `TURTLE_P3V1_ABI.md` (optional side quest).
- [x] Tag `P3V1` on `e76905c` and push to `origin`; scope Phase 3 V2 with user (`1A, 2A, migrate Lua, terminal UX + ECS inspector, docs/ECS_INTRO.md`).
- [x] Wait for user to validate Phase 3 V2 (`cargo run`, resize terminal to test live resize + docked ECS drawer at `>= 106` cols, `Tab`/`i` + `[`/`]` inspector, `examples/simple_room.lua`, `examples/life.lua`, and live `world.lua` editing) — validated 2026-11-25.
- [x] Confirm commit/push and `P3V2` tag for Phase 3 V2, merge `origin/main` (`e26b9c4`), then scope the next phase before implementing.
- [x] Approve and implement Phase 3 V3 (richer *Tidekeep* gameplay components & ECS systems).
- [x] Wait for user to validate Phase 3 V3 (`cargo run` — test combat, tonics/spear/flares, status expiry & AI state flips in `Tab` inspector, crate slams, and `bridge` layering) — validated 2026-11-25.
- [ ] **Future work (post-phase V2 roadmap):**
  - Port hand-rolled sparse-set ECS (`src/ecs.rs`) to an archetype ECS crate (`hecs` or `bevy_ecs`) behind the Lua `UserData` ABI boundary.
  - Explore next-stage `mlua` engine features: Lua coroutines (`coroutine.yield()`), stateful hot-reload & save/load serialization (`serde` snapshots of `EcsWorld`), and multi-file sandboxed `require`.

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

### 2026-11-23 — session 6 (Phase 3 V1 tagged; Phase 3 V2 implemented)
- User validated Phase 3 V1 and authorized starting Phase 3 V2.
- Recorded intervening commits since session 4 (`512d211` dexiotites update, `e76905c` `jim: updated docs in world.lua`) and configured `origin` remote (`git@github.com:daemacles/turtle.git`).
- Created annotated tag `P3V1` on `e76905c` and pushed to `origin`.
- Scoped Phase 3 V2 with user: hand-rolled generational sparse-set ECS (`src/ecs.rs`), hybrid Rust/Lua split via scoped `mlua::UserData` with transactional step rollback (`src/lua_host.rs`), full migration of `world.lua`, `examples/simple_room.lua`, and `examples/life.lua` to V2, terminal UX upgrades (alt-screen, `SIGWINCH` live resize, synchronized framing in `src/tty.rs` + `FrameLayout` in `src/model.rs`), live adaptive ECS inspector (`src/render.rs`), and `docs/ECS_INTRO.md` (linked from `README.md`).
- Updated `README.md` and `LEDGER.md` with V2 docs and Future Work (`hecs`/`bevy_ecs` port; richer Tidekeep gameplay components).

### 2026-11-24 / 2026-11-25 — session 7 (Modal overlay fix, Phase 3 V2 validated & pushed)
- Fixed modal overlay glyph bleed-through in `src/render.rs` (`draw_opaque_modal` painting an opaque `Rect` fill before `Container`) with cell-level regression tests; recorded `Container` gotcha #16.
- User validated Phase 3 V2 (`2026-11-25`); updated `LEDGER.md` and `README.md` phase status and opened next-phase scoping.
- Committed Phase 3 V2 (`f830d10`), created and pushed annotated tag `P3V2` to `origin`, merged `origin/main` (`e26b9c4`, preserving `TURTLE_SPEC.md`, `TURTLE_P3V1_ABI.md`, and `RETROSPECTIVE_P3V1.md`), selected richer *Tidekeep* gameplay components as Phase 3 V3, and flagged `hecs`/`bevy_ecs` port plus next-stage `mlua` features (coroutines, stateful hot-reload/save-load, multi-file `require`) as post-phase V2 future work.

### 2026-11-25 — session 8 (Phase 3 V3 implemented)
- Propagated opaque lower-layer entity backgrounds across `draw_entities` / `draw_one_entity` in `src/render.rs` so higher transparent sprites inherit `bridge` backgrounds cleanly, with regression test.
- Upgraded `world.lua` (*Tidekeep* V3) with composable `hp`, `combat`, `pickup`/`inventory`, timed `status` (`stun`, `invuln`, `regen`) with sparse-set pruning, component-driven `ai` FSM (`patrol`, `chase`, `flee`, `stunned`), Shell Crab enemy (`K`), crate slams, and real stacked `bridge` entities (`layer = 1`), organized into `status_tick_system` -> `player_intent_system` -> `ai_system` -> `hud_system`.
- Added V3 integration test in `src/lua_host.rs` and verified `cargo test` (21 passed), `cargo clippy`, `cargo doc`, LuaLS (`4/4`), `comment_load.py --fail-over 0`, `--steps 4` runs, and PTY interactive smoke tests.
- Updated `README.md` and `LEDGER.md`.
- User validated Phase 3 V3 (`2026-11-25`); synced `README.md` and `LEDGER.md`, committed on `main`, and pushed to `origin/main`.
