# turtle

A learning project about embedding **Lua** in **Rust** with [mlua](https://crates.io/crates/mlua) (pinned v0.12.x, Lua 5.4 backend) and rendering character grids with [terminal_graphics](https://crates.io/crates/terminal_graphics). Work proceeds in small phases; the user validates each phase by running it before authorizing the next one.

## Phases

| Phase | Description                                                                                             | Status                                                                   |
| ----- | ------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| 0     | Rust app runs inline Lua code that prints `hello, world`                                                | ✅ done (2026-11-14) — validated by user                                 |
| 1     | Load and execute a hardcoded `main.lua` file next to the project                                        | ✅ done (2026-11-14) — validated by user                                 |
| 2     | Interactive Lua REPL with `terminal_graphics` rendering                                                 | ✅ done (2026-11-18) — validated by user                                 |
| 3 V1  | Lua-driven 2D tile grid world engine (`world.lua` *Tidekeep*, `examples/life.lua`, `examples/simple_room.lua`) | ✅ done (2026-11-23) — validated by user                                 |
| 3 V2  | Hand-rolled sparse-set ECS, scoped `mlua::UserData` Host API, live resize & ECS inspector               | ✅ done (2026-11-25) — validated by user                                 |

## Phase 3 V2: Hand-Rolled ECS, Host API & Terminal UX

> **New to ECS?** Read [docs/ECS_INTRO.md](docs/ECS_INTRO.md) for an introduction to the Entity Component System pattern, the problems it solves and friction it introduces, turtle's sparse-set design, and how it maps onto `hecs` and `bevy_ecs`.

**Goal:** Rust runs an adaptive terminal engine (alternate screen buffer, instant `SIGWINCH` live resize, synchronized output framing, atomic Shadow-VM hot-reload, and a hand-rolled generational sparse-set ECS exposed via scoped `mlua::UserData`), while migrated V2 Lua scripts (`world.lua` by default, or `argv[1]`) define the map, palette, and gameplay rules.

### Running

```bash
cd turtle && cargo run
```

Runs `world.lua` (**Tidekeep** — procedural archipelago with pushable driftwood bridges, Sun Gems, and Reef Crabs in *Ultima*-style hybrid pacing). Open `world.lua` in an editor next to your terminal, tweak `SEA_LEVEL` or `CRAB_PERIOD` at the top, and save to see the world hot-reload at `t = 0`. Resize your terminal window while playing to expand the viewport or reveal the docked ECS Inspector drawer (`>= 106` columns).

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
| `Tab` / `i` | Toggle ECS Inspector | Docks as a side drawer (`>= 106` cols) or opens a modal overlay on narrower panes |
| `[` / `]` | Select prev / next entity | Cycles live entities in the ECS Inspector while gameplay continues |
| `p` | Pause / Play | Freezes `tick` and `hybrid` timers |
| `n` | Next step | Pauses and advances a single simulation step |
| `r` | Reset `t = 0` (same seed) | Replays the current world seed from `t = 0` |
| `R` | Reroll `t = 0` (new seed) | Generates a fresh random seed and runs `init(seed, ecs)` |
| `Esc` | Dismiss overlay | Closes active error modal or inspector overlay |
| `q` / `Ctrl-C` | Quit | Exits alternate screen and restores `termios` state and cursor visibility |

### Lua Script Contract (`world.lua` V2)

A world script returns a table (or defines global `world`) with:
- **`title`** (`string`, optional): header label.
- **`cell_width`** (`1` or `2`, default `2`): terminal columns per logical tile (`37×17` default at `2`, `75×17` default at `1`; scales with terminal size).
- **`pacing`** (`table`, optional):
  - `{ mode = "turn" }` — steps only on player input.
  - `{ mode = "tick", tick_ms = 140 }` (or `hz`) — steps on a fixed interval.
  - `{ mode = "hybrid", idle_ms = 650 }` — steps immediately on input and ticks ambient updates when idle.
- **`seed`** (`integer`, optional): pins a fixed seed on startup and reset unless rerolled with `R`.
- **`palette`** (`table`): maps short tile keys to `{ glyph, fg, bg? }` (using `terminal_graphics::Colour` names; omitting `bg` lets entities inherit the underlying terrain background).
- **`world.init(seed, ecs) -> hud?`**: spawns initial entities into the scoped `ecs` handle and returns optional `{ status, msg }`.
- **`world.step(t, requests, ecs) -> hud?`**: receives player/cursor intents `{ id, action, x, y, dx, dy }` and the transactional `ecs` userdata handle; mutates entities directly (rolled back on Lua runtime errors) and returns optional `{ status, msg }`.
- **`world.view(start_pos, end_pos, t) -> rows`**: receives inclusive `{ x, y }` bounds and returns `[rows][cols]` of palette keys or inline `{ ch, fg, bg? }` tables.

#### Scoped `ecs` UserData Methods

- `ecs:spawn(components?) -> integer` — creates a generational entity ID (packed positive 64-bit integer) with optional initial components (`name`, `pos = { x, y }`, `sprite = tile | { tile, layer? }`, `player`, `solid`, plus arbitrary custom components).
- `ecs:despawn(id) -> boolean` — removes an entity and increments its slot generation.
- `ecs:alive(id) -> boolean` — checks whether a handle is currently live.
- `ecs:get(id, component) -> any` / `ecs:set(id, component, val)` / `ecs:remove(id, component) -> boolean` / `ecs:has(id, component) -> boolean` — inspects or mutates core and custom components (`set(..., nil)` removes).
- `ecs:query(...) -> integer[]` — returns live entities carrying all named components in creation order (smallest sparse set first).
- `ecs:at(x, y) -> integer[]` — returns live entities at `(x, y)` in creation order.
- `ecs:find_by_name(name) -> integer?` / `ecs:count() -> integer`.

### Architecture (`src/`)

- `src/ecs.rs` — pure-Rust generational `EntityId`, generic `SparseSet<T>`, `Sprite` z-layering, `ScriptValue` trees, and `EcsWorld` (see [docs/ECS_INTRO.md](docs/ECS_INTRO.md)).
- `src/model.rs` — `Coord`, `RectBounds`, `Pacing`, `CellWidth`, `FrameLayout` (adaptive odd viewport sizing + drawer threshold), `TileStyle`, `Action`, `Intent`, `RepeatCadence`, and `Camera` (deadzone + pan).
- `src/tty.rs` — zero-extra-crate raw POSIX `termios` + `poll` guard (`libc`), alternate screen buffer, async-signal-safe `SIGWINCH` handler, synchronized output framing (`?2026h`/`l`), panic restore hook, and escape sequence decoder.
- `src/lua_host.rs` — `LuaWorld` marshaling, scoped `mlua::UserData` `EcsWorld` bindings with transactional step rollback, captured `print`, `FileStampWatcher`, and atomic Shadow-VM hot-reload.
- `src/render.rs` — adaptive `terminal_graphics` compositor with `Colour::Black` fills, z-ordered transparent entity compositing, contrast guard, event log, adaptive ECS Inspector (side drawer or modal), and error overlay.
- `src/main.rs` — CLI entry point (`world.lua` default, `argv[1]` override, `--steps N` smoke flag), live resize sync, and unified `poll()` loop.

(`main.lua` from Phase 1 remains untouched on disk.)

### Gate rule
- User validates each phase by running `cargo run` before authorizing the next.

## Future Work (post-phase V2)

Richer *Tidekeep* gameplay components are moving into **Phase 3 V3**. After the current phases are complete, future V2 tracks include:

- **Port hand-rolled ECS to an archetype crate (`hecs` or `bevy_ecs`):** evaluate replacing `src/ecs.rs` internals with `hecs` or `bevy_ecs` behind the Lua `UserData` boundary (see [docs/ECS_INTRO.md](docs/ECS_INTRO.md)).
- **Next-stage `mlua` & engine capabilities:** Lua coroutines (`coroutine.yield()` for multi-turn actor behaviors), stateful hot-reload and save/load serialization (`serde` snapshots of `EcsWorld`), and multi-file sandboxed `require`.

## Tracking state

- [LEDGER.md](LEDGER.md) — agent state ledger: current phase, decisions/gotchas, next steps. Read this first when resuming after a context reset.
- Git history — one commit per significant change; `git log --oneline` for the trail.
