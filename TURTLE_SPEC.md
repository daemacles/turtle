# Phase 3 V1 — Functional Specification

This specification defines the requirements for a Lua-scripted 2D terminal grid world engine. It is a **clean-room functional specification**: it states *what* the system must do and *which non-negotiable platform constraints* apply, while intentionally leaving library choices, internal data structures, exact Lua API schemas, and visual/UX polish to the implementer.

---

## 1. Goal & Non-Negotiable Constraints

Build a terminal engine in **Rust** that runs an **embedded Lua** runtime in-process, executes interactively on **Linux in a terminal**, and provides **automated verification** (`cargo test` plus a non-interactive smoke mode).

### Non-negotiable constraints
1. **Rust:** Implemented as a Rust binary project (`Cargo.toml` + `src/`).
2. **Embedded Lua:** Scripts execute via an in-process embedded Lua interpreter (never by spawning an external `lua` subprocess). Prefer a self-contained/vendored build so compilation does not depend on system-installed Lua development headers.
3. **Linux terminal execution:** Runs natively inside an interactive Linux terminal emulator without requiring a GUI window server.
4. **Automated verification:** Ships with an automated test suite (`cargo test`) and a bounded non-interactive smoke mode that run cleanly in headless/piped environments.

### Implementer freedom
- **No prescribed libraries or versions:** Choose any Rust crates, language/runtime versions, terminal backends, or file-watching mechanisms you judge best.
- **No prescribed Lua API syntax:** Design the cleanest, most idiomatic Lua script contract that satisfies the functional capabilities in §3 and supports the three required worlds in §4.
- **No prescribed visual layout or UX constants:** Screen dimensions, borders, color choices, camera smoothing, key-repeat tuning, and HUD formatting are yours to design for clarity and playability.

---

## 2. Architectural Intent & V2 Modularity Seam

Split responsibilities cleanly between the Rust host engine and Lua world scripts:

- **Rust host engine owns:**
  - Terminal raw-mode setup, non-blocking keyboard input, and guaranteed terminal restoration.
  - The main event loop, timing/pacing clock, and frame composition/rendering.
  - Authoritative storage of live world entities (positions, identifiers, visual keys, player designation).
  - Watching the active Lua script on disk and performing safe, atomic hot-reloads.
- **Lua world scripts own:**
  - World metadata (title, display density, pacing mode, optional fixed seed).
  - Visual palette (mapping terrain and actor keys to glyphs and colors).
  - Initialization logic (starting entities and initial world state).
  - Simulation rules (movement, collisions, interactions, AI, win/loss conditions, HUD updates).
  - On-demand viewport queries (producing the visible terrain grid for a requested bounding box).

> **V2 Extensibility Note (ECS Seam):** Keep Rust's entity storage decoupled behind a clean module boundary separate from Lua marshaling, terminal I/O, and rendering. A future V2 phase will evolve this entity store into an Entity Component System (ECS); keeping ownership and mutation cleanly isolated in V1 avoids rewriting the host–script bridge or terminal layer later.

---

## 3. Functional Requirements — Engine Capabilities

### 3.1 Command-Line Interface & Script Loading
- Running the binary with no script argument loads a default world script (`world.lua` in the working directory).
- Passing a positional script path overrides the default so different worlds can be launched from the CLI.
- Supports `--help` / `-h` usage output and returns clear, non-panicking errors on invalid CLI arguments or unreadable startup paths.
- Supports a bounded non-interactive smoke flag (e.g., `--steps N`) that initializes the world, advances `N` simulation steps without waiting for keyboard input, renders output safely even on non-TTY/piped streams, and exits cleanly with status `0`.

### 3.2 2D Grid World & Viewport Model
- **Unbounded signed coordinate plane:** Supports positive, zero, and negative integer `(x, y)` coordinates so worlds are not restricted to `(0, 0)`-rooted positive arrays.
- **Viewport-scoped rendering:** On each frame or view refresh, the host requests only the visible rectangular tile bounds around the active camera rather than materializing an infinite map in memory.
- **Layered composition:** Renders a terrain grid layer with live entities composited on top (preserving visual clarity when actors stand over varied terrain).
- **Dual presentation density:** Supports script-selectable tile density — both wide square-proportioned tiles (e.g., two terminal columns per logical tile so rooms and islands do not look vertically stretched) and compact single-column tiles for dense simulations like cellular automata.
- **Player-driven and playerless worlds:**
  - When a player-designated entity exists, movement inputs control that actor and the camera follows the player across the world.
  - When no player entity exists, directional inputs pan the viewport camera directly across the plane, a visible center reticle/cursor indicates the focal cell, and context-action interacts with that cell.

### 3.3 Simulation Pacing Modes
World scripts declare how simulation time `t` advances:
1. **Turn mode:** Simulation advances strictly on player input (movement, wait, or context action); nothing moves while the player is idle.
2. **Tick mode:** Simulation advances automatically on a fixed time interval (`tick_ms` or `hz`), independent of user input; user actions fold into the next simulation tick (or trigger pause/step controls).
3. **Hybrid mode:** Simulation advances immediately on player input *and* fires ambient steps automatically after a configurable idle period without input.

### 3.4 Deterministic Seeding & Reset
- World initialization receives an integer seed that drives both Lua's random number generator and script-level procedural generation so a given seed replays identically.
- Scripts may optionally declare a fixed seed, or leave seeding to the host.
- Users can reset the active world back to `t = 0` with the **same seed** (to replay/retry) or reroll back to `t = 0` with a **fresh random seed** (to generate a new layout).

### 3.5 Interactive Terminal Controls & Hygiene
- Reads single keypresses immediately in non-blocking raw mode (no pressing Enter, no echoing raw keystrokes or escape sequences onto the grid).
- Supports intuitive controls for:
  - **Directional move / pan:** Arrow keys, `WASD`, and Vi-style `HJKL`.
  - **Wait / pass turn:** e.g., `Space` or `.`.
  - **Context action / interact / toggle:** e.g., `E` or `Enter`.
  - **Pause / resume clock** (`P`) and **single-step while paused** (`N`).
  - **Reset same seed** (`r`) and **reroll new seed** (`R`).
  - **Dismiss non-fatal overlay** (`Esc`).
  - **Quit cleanly** (`q` / `Q` / `Ctrl-C`).
- **Terminal safety invariant:** Terminal input attributes and cursor visibility *must* be restored cleanly on normal quit, `Ctrl-C`, EOF/closed stdin, broken stdout pipe, and unexpected panics — never leaving the user's shell stuck in raw mode or with a hidden cursor.

### 3.6 Live Hot-Reload & Non-Fatal Error Isolation
- Monitors the active Lua script file for modifications while the engine is running (handling atomic editor saves cleanly).
- On a valid save, reloads the updated script in isolation and restarts the world cleanly at `t = 0` with clear visual confirmation in the UI.
- **Atomic error isolation:** If an edited script fails to compile, fails validation, or throws a runtime error during `init` or `view` (or during `step`):
  - The engine must **not** crash, exit unexpectedly, or spew raw stack traces across the terminal.
  - The previous valid world state remains frozen on screen while a readable, non-fatal error notice/overlay surfaces the Lua diagnostic.
  - Saving a corrected edit automatically recovers and resumes live simulation at `t = 0`.
- **Output capture:** Lua `print(...)` calls inside world scripts are captured into the engine's message/event log rather than writing directly to stdout and corrupting the terminal frame.

### 3.7 Visual Presentation (High-Level)
- Renders in-place in the terminal without endlessly scrolling screen history.
- Displays:
  - The 2D tile viewport with distinct foreground/background colors and glyphs for terrain and entities.
  - A concise HUD showing world title, run state (`LIVE` / `PAUSED` / `ERROR`), pacing mode, tick counter `t`, active seed, player/camera coordinates, and script-reported status text.
  - A recent event/print message area.
  - A compact control legend tailored to whether the world is player-driven or camera-driven.
- If the terminal pane is too small to fit the interface cleanly, displays a readable notice (instead of garbled wrapped lines) and still responds to resize or quit.

---

## 4. Functional Requirements — Required Sample Worlds

Deliver three complete, playable Lua world scripts using the engine's Lua API to prove every engine capability above:

### 4.1 Default World (`world.lua`) — Procedural Archipelago Adventure (`hybrid` pacing)
- **Pacing & density:** Hybrid pacing; square-proportioned wide tiles.
- **Terrain & determinism:** Seeded procedural archipelago generating deep water, coastal shoals, sandy beaches, grassy island interiors, and impassable stone cliffs/ruins around a guaranteed walkable starting camp (plus an immediately reachable nearby crate-and-water puzzle).
- **Gameplay mechanics:**
  - Player hero (`@` or equivalent) moves across walkable land and bridges; blocked by water and stone.
  - Pushable crates: pushing a crate onto adjacent walkable terrain slides it forward; pushing a crate into water consumes the crate and turns that water tile into a permanent walkable bridge; pushing a crate against stone or another entity fails with a helpful HUD/log message.
  - Collectible goal items (e.g., Sun Gems) placed across starting camp and islands; HUD tracks progress (`found / total`) and celebrates victory when all are collected.
  - Patrolling/chasing hazard creatures (e.g., Reef Crabs) that wander or pursue the player when nearby; colliding with a hazard sends the player back to camp with an event message.
  - Subtle ambient terrain animation (e.g., shimmering ocean waves driven by step `t`) that runs visibly when idle in hybrid mode.
- **Hot-reload friendliness:** Exposes clear tuning constants at the top of the file (e.g., sea level, hazard movement period, goal count) so editing a number and saving visibly transforms the world on reload.

### 4.2 Example 1 (`examples/life.lua`) — Sparse Cellular Automaton / Conway's Game of Life (`tick` pacing, playerless)
- **Pacing & density:** Fixed-interval tick pacing; single-column dense grid mode.
- **Playerless camera:** Zero entities; directional keys pan freely over an unbounded sparse coordinate plane without stopping or stepping the simulation prematurely; a visible center cursor marks the target cell.
- **Simulation & controls:**
  - Implements Conway's Game of Life (`B3/S23`) on a sparse live-cell set (no fixed map boundary).
  - Seeds recognizable classic patterns (such as gliders and an R-pentomino) plus a seed-dependent random colony so rerolling (`R`) produces varied evolution.
  - Context action (`E` / `Enter`) toggles the cell under the center cursor alive or dead; pause (`P`) freezes generations while allowing panning, toggling, and single-stepping (`N`) generation by generation.
  - HUD displays live cell count.

### 4.3 Example 2 (`examples/simple_room.lua`) — Turn-Based Puzzle Room (`turn` pacing)
- **Pacing & density:** Strict turn-based pacing; square-proportioned wide tiles.
- **Gameplay mechanics:**
  - Compact static walled room defining walls, walkable floor, a brass key pickup, a locked door/gate, a patrolling sentinel guard, and an exit shrine.
  - Player collects the key (updating HUD status), unlocks the door on contact (turning it into an open passage), avoids or gets bumped back to start by the sentinel patrol, and reaches the exit shrine to win.
  - Demonstrates minimal, clean Lua script structure suitable as a starting template for custom worlds.

---

## 5. Automated Verification Requirements

The project must include automated verification runnable without a human or interactive TTY:

1. **Unit & integration tests (`cargo test`):**
   - Core geometry, camera bounds/following, entity store spawning/updating/removal, input decoding, and rendering invariants.
   - Loading and stepping all three sample worlds (`world.lua`, `examples/life.lua`, `examples/simple_room.lua`) through initialization, step execution, and viewport queries.
   - Deterministic replay verification (same seed yields identical initial state and step behavior).
   - Hot-reload safety test verifying that attempting to load a broken/syntax-error Lua script returns an error cleanly and leaves an active valid world undisturbed.
2. **Non-interactive smoke execution:**
   - Running the compiled binary in smoke mode (e.g., `--steps 4`) against `world.lua`, `examples/life.lua`, and `examples/simple_room.lua` succeeds with exit code `0`, including when stdin/stdout are piped rather than attached to a terminal.
3. **Clean build:**
   - `cargo build` and `cargo test` complete with zero compiler warnings.

---

## 6. Human Validation Checklist (Evaluation Rubric)

Use this checklist when testing and comparing candidate implementations side by side in a Linux terminal:

### A. Automated Checks
- [ ] `cargo test` builds cleanly (zero warnings) and passes all tests without a TTY.
- [ ] Smoke mode runs against `world.lua`, `examples/life.lua`, and `examples/simple_room.lua` and exits `0` even with redirected/piped stdio.

### B. World Gameplay & Archetypes
- [ ] **Default (`world.lua`):** Starts cleanly on a walkable camp; player moves with `WASD`/`HJKL`/Arrows; pushing a crate into water builds a walkable bridge; gems collect and update HUD to victory; hazards patrol/chase and bounce player back to camp; ocean shimmers when idle.
- [ ] **Cellular Automaton (`examples/life.lua`):** Launches via positional CLI argument; runs continuously in dense grid mode; arrows pan the camera across negative/positive coordinates; `E` toggles the center cell; `P` pauses and `N` single-steps one generation cleanly.
- [ ] **Puzzle Room (`examples/simple_room.lua`):** Advances *only* on player input (`turn` mode); guard patrols; key unlocks the red/locked gate; reaching the exit triggers win feedback.

### C. Controls, Pacing & Determinism
- [ ] Single keypresses take effect immediately with no Enter needed and no echoed characters corrupting the frame.
- [ ] `r` resets `t = 0` identically with the same seed; `R` rolls a new seed and changes procedural layouts/colonies.
- [ ] `Space`/`.` waits one turn; `P` pauses/resumes; `N` advances exactly one step.

### D. Live Hot-Reload & Error Recovery
- [ ] While running `world.lua`, edit a top-level constant (e.g., sea level or hazard speed) and save: engine reloads back to `t = 0` automatically with visible confirmation.
- [ ] Save a syntax error (or runtime error) into `world.lua`: engine does **not** crash; the previous valid frame stays visible/frozen and an error overlay displays the Lua diagnostic.
- [ ] Fix and save `world.lua`: error overlay clears and the updated world resumes cleanly at `t = 0`.
- [ ] Calling Lua `print("hello")` inside a world script appears in the UI log rather than breaking the grid layout.

### E. Terminal Hygiene & Edge Cases
- [ ] Quitting via `q` or `Ctrl-C` restores cooked terminal input, echo, and cursor visibility cleanly in the parent shell.
- [ ] Shrinking the terminal pane below comfortable size shows a graceful warning instead of garbled/staircased output; resizing back recovers.

### F. Qualitative Comparison Notes (Evaluator)
- **V2 ECS Readiness:** How cleanly is entity state separated from Lua marshaling and terminal rendering in Rust?
- **Visual Polish & Responsiveness:** Frame stability (flicker/scroll), color legibility (especially entities over colored backgrounds), key-repeat comfort, and camera feel.
- **Lua Script Ergonomics:** Clarity, conciseness, and expressiveness of the model's chosen Lua script API.
