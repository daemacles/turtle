# Entity Component System (ECS) — Concepts, Design, and Mapping

An introduction to the **Entity Component System (ECS)** pattern used in **turtle** Phase 3 V2, why it exists, where it hurts at scale, how our hand-rolled Rust + Lua engine works, and how it maps onto production Rust ECS crates like [`hecs`](https://docs.rs/hecs) and [`bevy_ecs`](https://docs.rs/bevy_ecs).

---

## 1. The ECS Concept

ECS separates **identity**, **data**, and **behavior** into three distinct concepts:

| Concept | What it is | *Tidekeep* (`world.lua`) example |
| --- | --- | --- |
| **Entity** | An opaque handle (just an ID, zero logic or fields of its own) | `player`, `crate1`, `gem1`, `crab1` |
| **Component** | A small, plain data struct or marker attached to an entity | `pos = { x, y }`, `sprite = { tile, layer }`, `solid = true`, `crab = { period = 2 }` |
| **System** | Code that queries all entities sharing a set of components and updates them | Render system (`pos` + `sprite`), patrol system (`pos` + `crab`), movement/push collision |

Think of an entity as a row key in an in-memory database, each component type as a table keyed by that ID, and each system as a query over one or more tables. An entity has no class—what makes something a "Reef Crab" is simply having `pos`, `sprite`, `solid`, and `crab` components attached.

---

## 2. Problems ECS Solves

1. **Composition over deep inheritance trees.**
   Classic object-oriented games often start with `GameObject -> Actor -> Movable -> Enemy -> Crab`, then hit dead ends when a pushable crate is movable and solid like an actor, a ghost crab is an actor that ignores walls, or a gem is collectible without being solid. Multiple inheritance and giant base structs ("god objects" full of `Option<...>` fields) grow brittle fast. In ECS, behavior emerges from whichever components are attached.

2. **Runtime structural flexibility.**
   Because components live outside any fixed struct definition, entities can change role while running. Removing `solid` lets an actor turn ethereal; attaching `patrol` makes an inert statue start walking; adding `burn_ticks` applies a status effect without altering a global `Entity` struct.

3. **Data-oriented cache locality (SoA vs. AoS).**
   In an **Array of Structs (AoS)** layout (`Vec<Entity>` where each `Entity` carries `id`, `pos`, `sprite`, `ai`, `stats`, …), a system that only reads positions loads entire entities into CPU cache lines—wasting cache bandwidth on unused fields. ECS encourages **Struct of Arrays (SoA)**: each component type is stored in its own contiguous array (`Vec<Coord>`, `Vec<Sprite>`), so scanning positions walks tightly packed coordinates.

4. **Decoupling the host engine from game scripts.**
   In Phase 3 V1, `src/model.rs` baked a fixed `Entity { id, pos, tile, is_player }` record, and every step marshaled the entire `EntityStore` into Lua tables and parsed delta patch tables back. With ECS, Rust owns core engine components (`Position`, `Sprite`, `Name`, `Player`, `Solid`) so the renderer and camera work directly from dense Rust arrays, while Lua can attach arbitrary game-specific components (`crab`, `gem`, `crate`, `patrol`) without recompiling Rust.

---

## 3. Drawbacks and Friction in Larger Systems

ECS is a trade-off, not a free upgrade. As codebases grow, teams routinely run into six sources of friction:

1. **Scattered logic ("where does X happen?").**
   In OOP, opening `Crab.kt` or `Crab.rs` shows everything a crab does in one place. In ECS, crab behavior lives in a patrol system, a collision system, a damage system, and a render system. Tracing one entity's full lifecycle requires jumping across systems.

2. **Hidden system ordering and frame-delay bugs.**
   Systems communicate implicitly through shared components. If `CrabAiSystem` runs before `PlayerMoveSystem`, crabs chase where the player *was* last tick; if after, they react to where the player *just stepped*. As system counts grow, ordering constraints become delicate and non-local.

3. **Structural mutation during iteration (command buffers).**
   If you despawn or attach a component to an entity while iterating over the dense array storing that component, swap-remove shifts elements underneath the loop iterator. Large ECS engines solve this with deferred **command buffers** (queuing spawns/despawns to flush between phases), adding an extra layer of indirection and frame latency.

4. **Awkward fit for hierarchies and relational graphs.**
   Flat component tables excel at uniform grids and particle/actor sets, and struggle with trees and graphs: scene graphs (parent/child transforms), nested inventories ("the sword inside the chest held by the merchant"), and quest graphs. Modeling entity-to-entity links requires ID references that can dangle unless the engine tracks **relations** explicitly.

5. **Boilerplate for one-off state.**
   Not everything belongs on an entity. Global settings, turn counters, or an event bus feel forced if crammed onto dummy singleton entities—which is why production ECS frameworks add **Resources** and **Events** alongside components.

6. **Debuggability at runtime.**
   Because state is spread across multiple component arrays keyed by opaque numeric IDs, a plain `println!("{:?}", entity)` shows nothing useful. A runtime **ECS Inspector** (like the `Tab`/`i` inspector built into turtle V2) becomes essential once entities have more than a handful of components.

---

## 4. Turtle V2's Hand-Rolled Implementation Design

Turtle V2 implements a small, zero-dependency, pure-Rust ECS in `src/ecs.rs` paired with a scoped `mlua::UserData` bridge in `src/lua_host.rs`.

### 4.1 Generational `EntityId` and the ABA Problem

If entity IDs were plain vector indices (`0, 1, 2`), freeing slot `3` and immediately spawning a new entity into slot `3` would let an old reference (`id = 3`) silently mutate the brand-new entity—the classic **ABA problem**.

Turtle pairs every index with a monotonically increasing generation counter:

```rust
pub struct EntityId {
    index: u32,
    generation: u32, // starts at 1, increments on despawn
}
```

- When an entity despawns, its slot index goes onto `free_list` and `generations[index]` increments.
- Any lookup checks `generations[id.index] == id.generation` (and `alive[id.index]`). A stale handle is rejected immediately in `O(1)`.
- Internally formatted as `e{index}v{generation}` (e.g., `e0v1`) in the inspector.

**Packing for Lua:** In Lua 5.4, if `EntityId` were exposed as userdata, two handles to the same entity could be distinct userdata objects unless cached or interned, breaking `seen[id] = true` table keys and forcing metamethod indirection. Instead, Rust packs `(generation << 32) | index` into a positive 64-bit Lua integer (`to_lua_int` / `from_lua_int`, capping `generation <= i32::MAX` so bit 63 stays zero). Lua scripts get value equality (`==`), fast `seen[id]` table keys, and printable handles with zero allocation, while Rust still catches stale or forged integers on every call.

### 4.2 `SparseSet<T>` Storage

Each component type lives in a `SparseSet<T>`:

```
sparse:         [ Some(1), None, Some(0), ... ]   indexed by EntityId.index
                    │             │
                    ▼             ▼
dense_entities: [ e2v1,   e0v1 ]                  packed live entities
dense_values:   [ val(e2), val(e0) ]              packed component values
```

- **Insert / overwrite (`O(1)`):** Look up `sparse[id.index]`. If absent, push to `dense_entities` and `dense_values` and record the dense index in `sparse[id.index]`.
- **Get (`O(1)`):** Follow `sparse[id.index]` -> `dense_idx`; verify `dense_entities[dense_idx] == id` (guarding against stale generations); index `dense_values[dense_idx]`.
- **Remove (`O(1)` swap-remove):** Swap the target entry with the last entry in `dense_entities`/`dense_values`, update `sparse[moved.index] = Some(dense_idx)`, pop the tail, and set `sparse[id.index] = None`.
- **Iterate:** Walk `dense_values` directly—no holes, no `Option` unwrapping per element.

### 4.3 Hybrid Rust + Lua Component Model

`EcsWorld` splits components into two tiers:

1. **Core engine components (typed Rust `SparseSet`s):**
   - `Position(Coord)` (`"pos"`): authoritative world coordinates.
   - `Sprite { tile: String, layer: i32 }` (`"sprite"`): palette key plus render **z-order** (`layer`).
   - `Name(String)` (`"name"`): optional unique-friendly label for `ecs:find_by_name`.
   - `Player` (`"player"`): marker set indicating input/camera target.
   - `Solid` (`"solid"`): marker set indicating collision/blocking semantics.
   Rust's renderer (`ecs.renderables()`), camera (`ecs.player()`), and input dispatcher read these directly at native speed with zero Lua table parsing.

2. **Dynamic script components (`BTreeMap<String, SparseSet<ScriptValue>>`):**
   - Any component name other than the five core keys (such as `"crate"`, `"gem"`, `"crab"`, `"item"`, `"patrol"`) is stored as a `ScriptValue` (`Bool`, `Int`, `Float`, `Str`, `Table`, `List`) inside its own `SparseSet<ScriptValue>`.
   - Using `ScriptValue` instead of `mlua::RegistryKey` keeps `EcsWorld` 100% pure Rust: cloneable for rollback, unit-testable without spawning a Lua VM, and directly readable by the Rust ECS Inspector.

### 4.4 Queries, Scoped UserData, and Transactional Rollback

- **Smallest-set-first `query(c1, c2, ...)`:** Pick the requested component store with the fewest entries, iterate only those entities, and keep entities present in every other requested store, sorted deterministically by `spawn_order`.
- **Scoped `mlua::UserData`:** Both `world.init(seed, ecs)` and `world.step(t, requests, ecs)` execute inside `lua.scope(|scope| ...)` using `scope.create_any_userdata_ref_mut(&mut working_ecs)`. When the call returns, `mlua` invalidates the userdata handle so a Lua script cannot stash `ecs` in a global and mutate state outside a step.
- **Transactional step rollback:** Before calling `world.step`, Rust clones `current_ecs` into `working_ecs`. If Lua raises a runtime error mid-step, `working_ecs` is dropped and the previous valid `EcsWorld` remains intact beneath the red error overlay.

---

## 5. Mapping to `hecs` and `bevy_ecs`

| Turtle V2 (`src/ecs.rs`) | `hecs` | `bevy_ecs` |
| --- | --- | --- |
| `EcsWorld` | `hecs::World` | `bevy_ecs::world::World` |
| `EntityId` (`index` + `generation`) | `hecs::Entity` (index + generation) | `bevy_ecs::entity::Entity` (index + generation) |
| `SparseSet<T>` per component | **Archetype** chunks (`Entity` columns) | **Archetype Tables** (default) + optional `SparseSet` |
| Core + `BTreeMap<String, SparseSet<ScriptValue>>` | Open Rust types keyed by `TypeId` (`Component` trait) | Open Rust types implementing `Component` |
| `ecs:spawn({ pos = ..., sprite = ... })` | `world.spawn((Position(..), Sprite(..)))` (tuple bundles) | `commands.spawn((Position(..), Sprite(..)))` |
| `ecs:query("pos", "crab")` | `world.query::<(&Position, &Crab)>()` | `Query<(&Position, &Crab), With<...>>` |
| Manual `ecs` method calls | Immediate `&mut World` or deferred `CommandBuffer` | Deferred `Commands` auto-flushed between stages |

### Core Architectural Difference: Sparse Set vs. Archetypes

- **Sparse-Set ECS (Turtle V2, EnTT):** Each component type owns one dense array. Adding or removing a component on an entity only touches that single component's `SparseSet` (`O(1)`), which makes dynamic scripting and frequent component toggling cheap. Multi-component queries probe the other sets' sparse arrays during iteration.
- **Archetype ECS (`hecs`, `bevy_ecs` default):** Every unique *combination* of component types (e.g., `(Position, Sprite, Solid, Crab)`) forms an **archetype**—a single multi-column table storing all entities with that exact signature side-by-side. Multi-component iteration scans matching tables with zero sparse lookups and perfect prefetching. The trade-off: adding or removing a component moves every component of that entity from its old archetype table into a new one. (`bevy_ecs` lets you mark volatile components `#[component(storage = "SparseSet")]` to mix both strategies.)

### What `hecs` and `bevy_ecs` Provide Beyond Turtle V2

- **`hecs`** (minimal, ~few thousand lines):
  - Any `'static + Send + Sync` Rust type works as a component via `TypeId`—no central enum or field per core component.
  - Compile-time query borrowing (`&T` vs `&mut T`) catches aliasing bugs across systems.
  - `DynamicBundle` and `EntityBuilder` for constructing entities from runtime component lists (ideal for Lua scripting bridges) plus `CommandBuffer` for deferred structural edits.

- **`bevy_ecs`** (full application framework layer):
  - **Multi-threaded `Schedule`:** Analyzes each system's component and resource borrows and runs disjoint systems in parallel automatically.
  - **Change detection (`Changed<T>`, `Added<T>`):** Lets rendering or physics react only to entities modified since the last run.
  - **Resources & Events (`Res<T>`, `EventWriter<T>`):** First-class global state and typed frame-buffered message queues.
  - **Component hooks, observers, and relationships:** Lifecycle callbacks (`on_add` / `on_remove`) and built-in `Parent`/`Children` relationship invariants.

### Future Porting Path

Because `src/lua_host.rs` and `src/render.rs` talk to `EcsWorld` through a small explicit API (`spawn`, `despawn`, `get`, `set`, `query`, `at`, `renderables`, `inspect_components`), porting to `hecs` or `bevy_ecs` in a later phase stays localized to the Rust backend: core typed components become real `#[derive(Component)]` structs, custom Lua components become a component wrapping `BTreeMap<String, ScriptValue>` (or dynamic registered component IDs), while Lua scripts keep the same `ecs:*` Host API.
