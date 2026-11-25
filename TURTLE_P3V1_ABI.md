# Phase 3 V1 — Host–Script ABI & Conformance Specification

This document specifies the exact Phase 3 V1 host–script Application Binary Interface (ABI) and runtime semantics. It is designed as a **Stage 2 conformance specification**: an engine originally built against [TURTLE_SPEC.md](TURTLE_SPEC.md) can be adapted to this contract so that the unmodified reference scripts ([world.lua](world.lua), [examples/life.lua](examples/life.lua), and [examples/simple_room.lua](examples/simple_room.lua)) load, render, and play identically.

This specification intentionally avoids prescribing Rust crate dependencies or library versions; any embedded Lua runtime and terminal backend that implements the contract below conforms.

---

## 1. CLI Invocation Contract

```text
<binary> [script.lua] [--steps N]
<binary> -h | --help
```

- **Default script:** If no positional script path is given, load `world.lua` from the current working directory.
- **Script override:** The first positional argument overrides the script path. Passing a second positional argument returns an error.
- **`--steps N`:** Requires a non-negative integer (`u64`). Runs non-interactively without waiting for user input:
  1. Loads the script, executes `world.init(seed)` (if present) and `world.view(bounds, 0)`, and renders the initial `t = 0` frame.
  2. Loops `N` times (`t = 1..=N`), calling `world.step(t, {}, entities)` and `world.view(bounds, t)` and rendering each frame.
  3. Exits with status `0`. Must tolerate non-TTY stdin/stdout and `BrokenPipe` cleanly without error.
- **Flags:** Unknown `--*` flags return an error. `-h` / `--help` outputs usage text cleanly.

---

## 2. Script Loading, VM Environment & Global Hooks

### 2.1 Isolated VM & Standard Libraries
Each script load or hot-reload evaluates inside a fresh, isolated Lua virtual machine with standard Lua libraries available (scripts rely on `math.abs`, `math.floor`, `math.sin`, `math.cos`, `math.random`, `math.randomseed`, `tostring`, `pairs`, `ipairs`, and `string.sub`).

### 2.2 Module Export Convention
When evaluating the script chunk:
1. If the chunk returns a Lua **table**, that table is the world module.
2. Else if the chunk returns **`nil`**, read the global variable `world`, which must be a **table**.
3. Any other return value or missing/non-table global `world` is a load error.

### 2.3 Eager Validation on Load
Before accepting a script (especially during hot-reload), validate:
- `world.palette` exists and is a `table` with valid entries (§4).
- `world.step` exists and is a `function`.
- `world.view` exists and is a `function`.
- `world.init` is optional; if present, it is a `function`.
- Optional metadata fields (`title`, `cell_width`, `seed`, `pacing`) conform to §3.

### 2.4 Global `print(...)` Capture
Replace Lua's global `print` function so scripts never write directly to stdout over the terminal grid:
- Convert every argument via Lua's global `tostring`.
- Join converted strings with `"\t"`.
- Append the resulting line to a per-call capture buffer cleared before each `init`, `step`, and `view` call.
- After the hook returns, push captured lines into the UI message log (normalizing `\r`, `\n`, `\t` to spaces, trimming outer whitespace, and dropping blank lines).

### 2.5 Random Seeding
Immediately before calling `world.init(seed)` (or even if `world.init` is omitted), call Lua's `math.randomseed(seed)`.

---

## 3. World Configuration Fields (`world.*`)

All four configuration fields on the exported world table are optional:

| Field | Type | Default | Constraints & Semantics |
|---|---|---|---|
| `title` | `string` | Script filename stem (fallback `"world"`) | Display name shown in the HUD header. |
| `cell_width` | `integer` | `2` | Must be `1` or `2`; any other integer is a load error. `2` renders each logical tile across 2 terminal columns; `1` renders each tile in 1 column. |
| `seed` | `integer` | Host-generated seed | Must be `>= 0`; negative values error. On initial load, overrides the host seed. On hot-reload, if declared and different from the previously declared `seed`, adopts the new value; otherwise preserves the active seed unless the user triggers a reroll. |
| `pacing` | `table` | `{ mode = "hybrid", idle_ms = 650 }` | Controls how steps trigger (§3.1). |

### 3.1 `world.pacing` Table
- `mode` (`string`, optional, default `"hybrid"`): must be `"turn"`, `"tick"`, or `"hybrid"`. Any other value is a load error.
  - `"turn"`: steps execute only on player input.
  - `"tick"`: steps fire on a fixed interval:
    - If `tick_ms` (`integer >= 0`) is present: interval is `max(tick_ms, 16)` ms.
    - Else if `hz` (`number`) is present: interval is `round(1000.0 / max(hz, 0.5))` ms.
    - Else: defaults to `120` ms.
  - `"hybrid"`: steps immediately on player input and also fires an ambient step after `idle_ms` (`integer`, default `650`, clamped `>= 50` ms) without input.

---

## 4. Palette, Tile Styles & Color Names

`world.palette` is a dictionary mapping short tile key strings (e.g., `"~"`, `"@"`, `'"'`) to style tables.

### 4.1 Style Table Schema
Both positional and named forms must be supported (named `ch` / `fg` take precedence over indices `1` / `2`; `bg` falls back to index `3` when `bg` is `nil`):

```lua
-- Positional (used by reference scripts):
["~"] = { "≈~", "BrightCyan", "Blue" }  -- opaque terrain tile
["@"] = { "@ ", "BrightYellow" }        -- transparent entity (bg = nil)

-- Named (equivalent):
["~"] = { ch = "≈~", fg = "BrightCyan", bg = "Blue" }
```

- **`ch` / `[1]`** (`string`, required): 1 or 2 Unicode scalar values.
  - In `cell_width = 2`: renders scalar 1 in the left column and scalar 2 in the right column; a 1-character string pads the right column with `' '`; an empty string renders as `'  '`; characters past the 2nd are ignored.
  - In `cell_width = 1`: renders only the first scalar (or `' '` if empty).
- **`fg` / `[2]`** (`string`, required): exact case-sensitive color name from §4.2.
- **`bg` / `[3]`** (`string | nil`, optional): exact case-sensitive color name from §4.2, or `nil`. Non-string, non-nil values raise an error.

### 4.2 Exact 16 Color Names (Case-Sensitive)
Palette entries and inline view style tables use these exact 16 ANSI color names:

- **Standard (8):** `"Black"`, `"Red"`, `"Green"`, `"Yellow"`, `"Blue"`, `"Magenta"`, `"Cyan"`, `"White"`
- **Bright (8):** `"BrightBlack"`, `"BrightRed"`, `"BrightGreen"`, `"BrightYellow"`, `"BrightBlue"`, `"BrightMagenta"`, `"BrightCyan"`, `"BrightWhite"`

Any other color string must raise a load/view error.

### 4.3 Background Inheritance, Fallback & Contrast Guard
- **Terrain `bg == nil`:** defaults to `"Black"`. (Inline style tables returned by `world.view` also default `bg = nil` to `"Black"`.)
- **Entity `bg == nil`:** transparent — inherits the resolved background color of the terrain tile beneath the entity. If an entity's palette entry specifies an explicit `bg`, that explicit background overrides terrain.
- **Unknown entity `tile` key:** falls back to glyph `"?"`, `fg = "BrightMagenta"`, `bg = nil` (transparent).
- **Contrast guard:** when an entity's resolved `fg == resolved_bg`, substitute `"Black"` if `resolved_bg` is `"White"`, `"BrightWhite"`, `"BrightYellow"`, `"BrightCyan"`, `"BrightGreen"`, or `"Yellow"`; otherwise substitute `"BrightWhite"`.

---

## 5. Coordinate System & Viewport Bounds

- **Coordinate frame:** Signed 32-bit integers `(x, y)`. `+x` points **east** (right); `+y` points **south** (down). Negative coordinates are first-class and used by all three reference scripts.
- **Inclusive bounds:** The viewport covers `[min.x ..= max.x]` horizontally and `[min.y ..= max.y]` vertically, centered on `camera.center`:
  ```text
  min.x = center.x - floor(view_cols / 2)
  min.y = center.y - floor(view_rows / 2)
  max.x = min.x + view_cols - 1
  max.y = min.y + view_rows - 1
  ```
- **Reference viewport sizes:** The Phase 3 V1 reference engine uses `37 × 17` logical tiles at `cell_width = 2` and `75 × 17` logical tiles at `cell_width = 1` (odd dimensions keep `camera.center` at the exact center cell). All three reference scripts compute dimensions dynamically from `start_pos` and `end_pos`, so any odd viewport whose `start_pos`/`end_pos` match the host's expected row/column count is compatible.

---

## 6. Lifecycle Hooks & Data Shapes

### 6.1 `world.init(seed) -> entities?, hud?` *(optional)*

Called on initial startup, on hot-reload, and on reset (`r` / `R`) after `math.randomseed(seed)`.

- **Input:** `seed` (`integer`).
- **Return 1 — `entities`** (`table[] | nil`, optional): 1-based array of entity spawn records:
  ```lua
  { id = "player", x = 0, y = 0, tile = "@", player = true }
  ```
  - `id` (`string`, required)
  - `x`, `y` (`integer`, required)
  - `tile` (`string`, required; palette key)
  - `player` (`boolean`, optional, default `false`)
  - **Duplicate `id` handling:** collapses duplicates by overwriting the earlier entry in place while preserving insertion order.
  - **Implicit player promotion:** if no entity in the resulting store has `player == true` and an entity with `id == "player"` exists, the host promotes that entity to `player = true`.
- **Return 2 — `hud`** (`table | nil`, optional): `{ status?: string, msg?: string }`. Sets initial HUD `status` (defaults to `""` if omitted/nil) and appends `msg` (if non-nil) to the event log.
- Non-table, non-nil return values in position 1 or 2 raise an error.

### 6.2 `world.step(t, requests, entities) -> patches?, hud?` *(required)*

Advances simulation state by one step.

- **Inputs:**
  1. `t` (`integer`): 1-based step counter (`1, 2, 3, ...`; resets to `0` on init/reload/reset, then `1` on first step).
  2. `requests` (`table[]`): 1-based array of action intent tables:
     ```lua
     { id = "player", action = "move", x = 0, y = 0, dx = 1, dy = 0 }
     ```
     - `id` (`string`): target entity ID, or `"$cursor"` for playerless `"act"` commands.
     - `action` (`"move" | "wait" | "act"`):
       - `"move"`: `dx, dy` hold tile step delta (`(-1|0|1, -1|0|1)`).
       - `"wait"`: `dx = 0, dy = 0`.
       - `"act"`: `dx = 0, dy = 0`.
     - `x`, `y` (`integer`): entity position (or `camera.center` for `"$cursor"`) when the request was constructed.
     - Empty `{}` when no user intent was queued for this step (e.g., timed ticks, idle hybrid steps, `--steps N`, or `"wait"` in a playerless world).
  3. `entities` (`table<string, Entity>`): read-only snapshot dictionary keyed by entity `id`, each value containing `{ id, x, y, tile, player }` prior to applying the returned patches.

- **Return 1 — `patches`** (`table[] | nil`, optional): 1-based sequence of delta mutation records applied in order by the host:
  ```lua
  { id = "player", x = 1, y = 0 }        -- move existing entity
  { id = "crate1", remove = true }       -- remove entity
  { id = "gem6", x = 3, y = -2, tile = "G" } -- spawn new entity
  ```
  - `id` (`string`, required)
  - `remove` (`boolean`, optional, default `false`): if `true`, deletes the matching entity and skips other fields.
  - `x`, `y` (`integer`, optional): must either **both** be present or **both** be omitted/`nil`; providing only one raises an error.
  - `tile` (`string`, optional)
  - `player` (`boolean`, optional)
  - **Patch application rules:**
    - If `id` exists in the store: updates any non-nil fields (`pos`, `tile`, `player`) in place, preserving insertion order.
    - If `id` does not exist and both `x` and `y` are present: appends a new entity at the end (`tile` defaults to `"?"`, `player` defaults to `(id == "player")`).
    - If `id` does not exist and `(x, y)` are omitted: no-op.

- **Return 2 — `hud`** (`table | nil`, optional): `{ status?: string, msg?: string }`.
  - If `hud.status` is non-nil, replaces the persistent HUD status string (if `nil`, preserves the previous HUD status string).
  - If `hud.msg` is non-nil, appends it to the event log.

### 6.3 `world.view(start_pos, end_pos, t) -> rows` *(required)*

Queries the visible terrain matrix covering the inclusive world rectangle.

- **Inputs:**
  1. `start_pos`: `{ x = min_x, y = min_y }` (inclusive top-left corner).
  2. `end_pos`: `{ x = max_x, y = max_y }` (inclusive bottom-right corner).
  3. `t` (`integer`): current step counter (`0` immediately after init/reload/reset, `1+` after step `t`).
- **Return — `rows`** (`table`): 1-based array of `height = max_y - min_y + 1` rows ordered north to south (`y = start_pos.y .. end_pos.y`). Each row `rows[ry]` may be either:
  1. **A 1-based array table** of `width = max_x - min_x + 1` cells ordered west to east (`x = start_pos.x .. end_pos.x`), where each cell is either:
     - A palette key `string` (looked up in `world.palette`), OR
     - An inline style table `{ ch/[1], fg/[2], bg?/[3] }` (inline `bg = nil` defaults to `"Black"`).
  2. **A `string`** containing at least `width` Unicode scalar characters, where each scalar is looked up as a 1-character key in `world.palette`.
- **Strict validation:** Missing rows (`nil`), wrong row type, short row strings/arrays, `nil` cells, unknown palette keys, or malformed inline style tables must raise an error.

---

## 7. Host Runtime Semantics Required for Compatibility

1. **Entity Draw Order:** Entities render over terrain in deterministic insertion order in two passes — all `player == false` entities first, then `player == true` entities — so the player sprite always stays visible when sharing a cell or moving past items.
2. **Player-Present vs. Playerless Input Semantics:**
   - **Player present:** On init/reload/reset, snap camera to `player.pos`. After each step, track `player.pos` (keeping the player inside the viewport). Movement/Wait/Act dispatch an `Intent` for the player entity.
   - **Playerless (e.g., `examples/life.lua`):** Directional `Move` pans `camera.center` by `(dx, dy)` and calls `world.view(bounds, t)` *without* advancing `t` or calling `world.step`. Render a visible cursor reticle at the viewport center. Pressing `Act` (`E`/`Enter`) emits `{ id = "$cursor", action = "act", x = camera.center.x, y = camera.center.y, dx = 0, dy = 0 }` to `world.step`. Pressing `Wait` (`Space`/`.`) calls `world.step` with `{}`.
3. **Pacing Dispatch Rules:**
   - **`turn` and `hybrid`:** Player actions run `world.step` immediately (`hybrid` resets its idle timer from the step timestamp).
   - **`tick`:** Player actions enqueue into a single pending-intent slot (latest move/action wins before the next tick) and are passed into `world.step` on the next scheduled tick (or on manual single-step `N`).
4. **Atomic Shadow-VM Hot-Reload:**
   - On file change, compile and run `init(next_seed)` + `view(bounds, 0)` inside a **candidate VM** before modifying live state.
   - Only swap once the candidate passes all validation and hooks; reset `t = 0`, clear error state, unpause, and announce reload.
   - On any compile, validation, or runtime error (at load, init, step, or view), freeze the previous valid world, pause automatic stepping, and display the diagnostic without crashing or corrupting the terminal.
5. **Reset (`r` / `R`):**
   - `r` re-initializes the loaded world with the current seed back to `t = 0` (or retries disk reload if startup failed).
   - `R` rolls a new random seed and re-initializes back to `t = 0`.
6. **Empty HUD Fallback:** When `hud_status` is empty (`""`), render `"entities:<count>"` in the status area (e.g., `entities:0`).

---

## 8. Conformance Acceptance Checks

An implementation conforms to the Phase 3 V1 ABI when all checks below pass against the **unmodified** reference scripts:

1. **Automated smoke checks (no modifications to Lua files):**
   ```bash
   cargo test
   cargo run -- --steps 4
   cargo run -- examples/life.lua --steps 4
   cargo run -- examples/simple_room.lua --steps 4
   ```
   All four commands exit `0` with no errors or raw Lua print leakage onto stdout.
2. **Interactive verification against unmodified scripts:**
   - **`world.lua` (*Tidekeep*):** Camp renders centered on `@ `; pushing crates `[]` into water `≈~` removes the crate entity (`remove = true`) and turns that cell into bridge `==`; collecting Sun Gems `<>` increments `GEMS n/5`; Reef Crabs `><` move on `t % 2 == 0` in hybrid idle and bounce `@ ` back to `(0, 0)` on collision; ocean `≈~`/`~≈` shimmers on odd/even steps.
   - **`examples/life.lua` (*Game of Life*):** Dense 1-column tiles (`cell_width = 1`); arrows pan camera smoothly without calling `step` or advancing `t`; pressing `E` sends `$cursor` `act` and toggles the center cell; `P` pauses and `N` steps one generation; `R` changes the seeded colony.
   - **`examples/simple_room.lua` (*Simple Room*):** Strictly turn-based; picking up `k-` sets `KEY: YES` and removes `"key"`; bumping red gate `[]` turns it into open passage `..`; reaching `<>` sets `ESCAPED!`; sentinel `M ` patrols horizontally and resets player to `(-4, -2)` on bump.
   - **Live hot-reload:** Editing `SEA_LEVEL` in `world.lua` reloads to `t = 0` preserving or updating world cleanly; introducing a syntax error freezes the previous frame with an error notice; fixing the error recovers automatically.
