# turtle

A learning project about embedding **Lua** in **Rust** with [mlua](https://crates.io/crates/mlua) (pinned v0.12.x, Lua 5.4 backend) and rendering character grids with [terminal_graphics](https://crates.io/crates/terminal_graphics). Work proceeds in small phases; the user validates each phase by running it before authorizing the next one.

## Phases

| Phase | Description                                                                                             | Status                                                                   |
| ----- | ------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| 0     | Rust app runs inline Lua code that prints `hello, world`                                                | ✅ done (2026-11-14) — validated by user                                 |
| 1     | Load and execute a hardcoded `main.lua` file next to the project                                        | ✅ done (2026-11-14) — validated by user                                 |
| 2     | Interactive Lua REPL with `terminal_graphics` rendering                                                 | ✅ done (2026-11-18) — validated by user                                 |
| 3 V1  | Lua-driven 2D tile grid world engine (`world.lua` *Tidekeep*, `examples/life.lua`, `examples/simple_room.lua`) | ✅ built & tested (2026-11-18) — awaiting user validation (`cargo run`)  |

## Phase 3 V1: 2D Grid World Engine

**Goal:** Rust acts as a lightweight terminal engine (`78×23` `terminal_graphics` canvas + raw-mode POSIX input + entity store + atomic hot-reload), while a Lua script (`world.lua` by default, or `argv[1]`) defines the map, palette, rules, and entity behavior.

### Running

```bash
cd turtle && cargo run
```

Runs `world.lua` (**Tidekeep** — procedural archipelago with pushable driftwood bridges, Sun Gems, and Reef Crabs in *Ultima*-style hybrid pacing). Open `world.lua` in an editor next to your terminal, tweak `SEA_LEVEL` or `CRAB_PERIOD` at the top, and save to see the world hot-reload at `t = 0`.

Try the included examples too:

```bash
cd turtle && cargo run -- examples/life.lua
cd turtle && cargo run -- examples/simple_room.lua
```

Non-interactive smoke runs and unit tests:

```bash
cd turtle && cargo run -- --steps 4
cd turtle && cargo test
```

### Controls

| Key | Action | Notes |
| --- | --- | --- |
| `WASD` / `HJKL` / `Arrows` | Move player or pan camera | In playerless worlds like `examples/life.lua`, pans the viewport directly |
| `Space` / `.` | Wait in place | Passes a turn (`dx = 0, dy = 0`) |
| `e` / `Enter` | Context action (`act`) | Toggles the center cell in `examples/life.lua` |
| `p` | Pause / Play | Freezes `tick` and `hybrid` timers |
| `n` | Next step | Pauses and advances a single simulation step |
| `r` | Reset `t = 0` (same seed) | Replays the current world seed from `t = 0` |
| `R` | Reroll `t = 0` (new seed) | Generates a fresh random seed and runs `init(seed)` |
| `Esc` | Dismiss overlay | Hides non-fatal overlays |
| `q` / `Ctrl-C` | Quit | Cleanly restores terminal `termios` state and cursor visibility |

### Lua Script Contract (`world.lua`)

A world script returns a table (or defines global `world`) with:
- **`title`** (`string`, optional): header label.
- **`cell_width`** (`1` or `2`, default `2`): terminal columns per logical tile (`37×17` square tiles at `2`, `75×17` dense tiles at `1`).
- **`pacing`** (`table`, optional):
  - `{ mode = "turn" }` — steps only on player input.
  - `{ mode = "tick", tick_ms = 140 }` (or `hz`) — steps on a fixed interval.
  - `{ mode = "hybrid", idle_ms = 650 }` — steps immediately on input and ticks ambient updates when idle.
- **`seed`** (`integer`, optional): pins a fixed seed on startup and reset unless rerolled with `R`.
- **`palette`** (`table`): maps short tile keys to `{ glyph, fg, bg? }` (using `terminal_graphics::Colour` names; omitting `bg` lets entities inherit the underlying terrain background).
- **`world.init(seed) -> entities?, hud?`**: seeds initial entities `{ id, x, y, tile, player? }` and optional `{ status, msg }`.
- **`world.step(t, requests, entities) -> updates?, hud?`**: receives player/cursor intents `{ id, action, x, y, dx, dy }` and the current `entities` map; returns delta patches `{ id, x?, y?, tile?, player?, remove? }` and optional `{ status, msg }`.
- **`world.view(start_pos, end_pos, t) -> rows`**: receives inclusive `{ x, y }` bounds and returns `[rows][cols]` of palette keys or inline `{ ch, fg, bg? }` tables.

### Architecture (`src/`)

- `src/model.rs` — `Coord`, `RectBounds`, `Pacing`, `CellWidth`, `TileStyle`, `Action`, `Intent`, `RepeatCadence`, `Camera` (deadzone + pan), and `EntityStore` (the V2 ECS seam).
- `src/tty.rs` — zero-extra-crate raw POSIX `termios` + `poll` guard (`libc`), panic restore hook, window size query, and escape sequence decoder.
- `src/lua_host.rs` — `LuaWorld` marshaling, captured `print`, `FileStampWatcher`, and atomic Shadow-VM hot-reload (mid-edit syntax/runtime errors freeze the previous valid frame inside a red overlay).
- `src/render.rs` — constant `78×23` `terminal_graphics` compositor with `Colour::Black` background fill, transparent entity compositing, contrast guard, event log, and error modal.
- `src/main.rs` — CLI entry point (`world.lua` default, `argv[1]` override, `--steps N` smoke flag) and unified `poll()` loop.

(`main.lua` from Phase 1 remains untouched on disk.)

### Gate rule
- User validates each phase by running `cargo run` before authorizing the next.

## Tracking state

- [LEDGER.md](LEDGER.md) — agent state ledger: current phase, decisions/gotchas, next steps. Read this first when resuming after a context reset.
- Git history — one commit per significant change; `git log --oneline` for the trail.
