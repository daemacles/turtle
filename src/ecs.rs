//! Pure-Rust generational entity and sparse-set component storage.
//!
//! Stores entity handles, core spatial and visual components, and dynamic script-defined
//! components inside dense arrays independent of Lua or terminal libraries. Keeping storage
//! self-contained lets simulation steps clone snapshots for rollback and lets future versions swap
//! the backend without changing the engine boundary.

use std::collections::BTreeMap;
use std::fmt;

use crate::model::Coord;

/// Largest generation counter allowed so packed Lua integer handles remain positive.
const MAX_GENERATION: u32 = i32::MAX as u32;

/// Generational handle identifying one slot and lifecycle incarnation in an entity world.
///
/// Reusing a freed slot advances its generation so stale handles retained by scripts or subsystems
/// are rejected instead of aliasing a newly spawned occupant.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId {
    /// Zero-based slot index inside entity generation metadata tables.
    index: u32,
    /// Monotonic incarnation counter starting at one for each occupied slot lifetime.
    generation: u32,
}

impl EntityId {
    /// Returns the zero-based slot index encoded in this handle.
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Returns the non-zero lifecycle generation encoded in this handle.
    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// Packs this handle into a positive signed integer suitable for Lua table keys.
    pub const fn to_lua_int(self) -> i64 {
        (((self.generation as u64) << 32) | (self.index as u64)) as i64
    }

    /// Unpacks a signed integer returned from Lua into an entity handle if structurally valid.
    ///
    /// Rejects non-positive values and reserved zero or overflowing generations before checking live
    /// slot tables.
    ///
    /// - `raw`: packed signed integer supplied by script code.
    pub const fn from_lua_int(raw: i64) -> Option<Self> {
        if raw <= 0 {
            return None;
        }
        let bits = raw as u64;
        let generation = (bits >> 32) as u32;
        let index = (bits & 0xffff_ffff) as u32;
        if generation == 0 || generation > MAX_GENERATION {
            return None;
        }
        Some(Self { index, generation })
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e{}v{}", self.index, self.generation)
    }
}

/// Contiguous sparse-set store mapping entity handles to one component type.
///
/// Trades a sparse index vector for constant-time lookup, insertion, and swap-remove while keeping
/// live component payloads densely packed for iteration.
#[derive(Clone, Debug)]
pub struct SparseSet<T> {
    /// Dense slot position indexed by entity slot index.
    sparse: Vec<Option<usize>>,
    /// Parallel dense vector of entity handles matching each component entry.
    dense_entities: Vec<EntityId>,
    /// Densely packed component values ordered alongside entity handles.
    dense_values: Vec<T>,
}

impl<T> Default for SparseSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SparseSet<T> {
    /// Creates an empty sparse-set component store.
    pub fn new() -> Self {
        Self {
            sparse: Vec::new(),
            dense_entities: Vec::new(),
            dense_values: Vec::new(),
        }
    }

    /// Returns the number of component entries currently stored.
    pub fn len(&self) -> usize {
        self.dense_values.len()
    }

    /// Reports whether this store currently holds zero component entries.
    pub fn is_empty(&self) -> bool {
        self.dense_values.is_empty()
    }

    /// Reports whether the given entity handle currently owns an entry in this store.
    ///
    /// - `entity`: target generational entity handle to test.
    pub fn contains(&self, entity: EntityId) -> bool {
        self.dense_index(entity).is_some()
    }

    /// Returns an immutable reference to the component attached to an entity if present.
    ///
    /// - `entity`: target generational entity handle to look up.
    pub fn get(&self, entity: EntityId) -> Option<&T> {
        self.dense_index(entity).map(|i| &self.dense_values[i])
    }

    /// Inserts or replaces the component value for an entity handle, returning any prior value.
    ///
    /// - `entity`: live entity handle receiving the component value.
    /// - `value`: component payload to store.
    pub fn insert(&mut self, entity: EntityId, value: T) -> Option<T> {
        let idx = entity.index() as usize;
        if idx >= self.sparse.len() {
            self.sparse.resize(idx + 1, None);
        }

        if let Some(dense_idx) = self.sparse[idx]
            && self.dense_entities[dense_idx] == entity
        {
            let prev = std::mem::replace(&mut self.dense_values[dense_idx], value);
            return Some(prev);
        }

        let dense_idx = self.dense_entities.len();
        self.sparse[idx] = Some(dense_idx);
        self.dense_entities.push(entity);
        self.dense_values.push(value);
        None
    }

    /// Removes the component entry for an entity via swap-remove and returns it if present.
    ///
    /// Removing shifts the tail entry into the vacated dense slot so iteration order across this
    /// store alone is not preserved across removals.
    ///
    /// - `entity`: target entity handle whose component should be detached.
    pub fn remove(&mut self, entity: EntityId) -> Option<T> {
        let dense_idx = self.dense_index(entity)?;
        let last_idx = self.dense_entities.len() - 1;

        let removed_value = self.dense_values.swap_remove(dense_idx);
        self.dense_entities.swap_remove(dense_idx);
        self.sparse[entity.index() as usize] = None;

        if dense_idx != last_idx {
            let moved_entity = self.dense_entities[dense_idx];
            self.sparse[moved_entity.index() as usize] = Some(dense_idx);
        }

        Some(removed_value)
    }

    /// Returns the densely packed slice of entity handles owning this component.
    pub fn entities(&self) -> &[EntityId] {
        &self.dense_entities
    }

    /// Iterates over every entity handle and component reference in dense storage order.
    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &T)> {
        self.dense_entities
            .iter()
            .copied()
            .zip(self.dense_values.iter())
    }

    /// Resolves the dense array index if the sparse slot matches the exact entity generation.
    fn dense_index(&self, entity: EntityId) -> Option<usize> {
        let idx = entity.index() as usize;
        let dense_idx = *self.sparse.get(idx)?.as_ref()?;
        (self.dense_entities.get(dense_idx).copied() == Some(entity)).then_some(dense_idx)
    }
}

/// Visual appearance component binding an entity to a palette sprite and render layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sprite {
    /// Key inside the active script palette selecting glyph and colors.
    pub tile: String,
    /// Vertical draw priority where larger numbers paint on top of lower layers.
    pub layer: i32,
}

/// Pure-Rust representation of dynamic component values authored in Lua scripts.
///
/// Storing plain value trees rather than script VM handles keeps entity worlds cloneable for
/// rollback, testable offline, and inspectable directly by the terminal UI.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptValue {
    /// Boolean flag or tag value.
    Bool(bool),
    /// Signed integer scalar.
    Int(i64),
    /// Double-precision floating-point scalar.
    Float(f64),
    /// Text string scalar.
    Str(String),
    /// Key-value map ordered deterministically by key name.
    Table(BTreeMap<String, ScriptValue>),
    /// Sequence of ordered values originating from a one-based Lua array.
    List(Vec<ScriptValue>),
}

impl ScriptValue {
    /// Formats this dynamic value into a concise single-line summary for the inspector pane.
    pub fn format_compact(&self) -> String {
        match self {
            Self::Bool(b) => b.to_string(),
            Self::Int(n) => n.to_string(),
            Self::Float(f) => format!("{f}"),
            Self::Str(s) => format!("\"{s}\""),
            Self::Table(map) => {
                if map.is_empty() {
                    return "{}".to_string();
                }
                let fields: Vec<String> = map
                    .iter()
                    .map(|(k, v)| format!("{k}={}", v.format_compact()))
                    .collect();
                format!("{{ {} }}", fields.join(", "))
            }
            Self::List(items) => {
                let parts: Vec<String> = items.iter().map(Self::format_compact).collect();
                format!("[{}]", parts.join(", "))
            }
        }
    }
}

/// Central entity-component world owning live handles, core components, and script components.
///
/// Rust owns entity lifetimes and typed engine components directly while letting scripts attach
/// arbitrary named component values. Cloning produces an independent snapshot used by the host to
/// roll back failed simulation steps.
#[derive(Clone, Debug, Default)]
pub struct EcsWorld {
    /// Per-slot generation counter indexed by entity slot index.
    generations: Vec<u32>,
    /// Per-slot liveness flag indexed by entity slot index.
    alive: Vec<bool>,
    /// Stack of despawned slot indices available for reuse.
    free_list: Vec<u32>,
    /// Dense creation sequence used to keep queries and render tie-breaking deterministic.
    spawn_order: SparseSet<u64>,
    /// Next monotonic sequence number assigned to a newly spawned entity.
    next_order: u64,
    /// Authoritative world coordinates indexed by entity handle.
    positions: SparseSet<Coord>,
    /// Visual palette sprites and z-layers indexed by entity handle.
    sprites: SparseSet<Sprite>,
    /// Optional human-readable names for lookup and inspector display.
    names: SparseSet<String>,
    /// Marker set identifying entities driven by local player input and camera focus.
    players: SparseSet<()>,
    /// Marker set identifying physically blocking entities.
    solids: SparseSet<()>,
    /// Dynamic script-defined component stores keyed by component name.
    custom: BTreeMap<String, SparseSet<ScriptValue>>,
}

impl EcsWorld {
    /// Creates an empty entity-component world ready to spawn entities.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocates a new live entity handle, reusing a retired slot when available.
    ///
    /// Newly spawned entities begin with zero attached components and receive a monotonically
    /// increasing creation order used to break query ties deterministically.
    pub fn spawn(&mut self) -> EntityId {
        let id = if let Some(index) = self.free_list.pop() {
            let idx = index as usize;
            self.alive[idx] = true;
            EntityId {
                index,
                generation: self.generations[idx],
            }
        } else {
            let index = u32::try_from(self.generations.len())
                .expect("entity slot capacity fits within 32-bit indices");
            self.generations.push(1);
            self.alive.push(true);
            EntityId {
                index,
                generation: 1,
            }
        };

        let order = self.next_order;
        self.next_order = self.next_order.saturating_add(1);
        self.spawn_order.insert(id, order);
        id
    }

    /// Destroys a live entity and detaches all core and custom components from it.
    ///
    /// Advances the slot generation before recycling so stale handles fail subsequent checks.
    /// Returns false if the handle is already dead or out of range.
    ///
    /// - `id`: target generational entity handle to destroy.
    pub fn despawn(&mut self, id: EntityId) -> bool {
        if !self.is_alive(id) {
            return false;
        }
        let idx = id.index() as usize;
        self.alive[idx] = false;

        self.spawn_order.remove(id);
        self.positions.remove(id);
        self.sprites.remove(id);
        self.names.remove(id);
        self.players.remove(id);
        self.solids.remove(id);
        self.custom.retain(|_, store| {
            store.remove(id);
            !store.is_empty()
        });

        if let Some(next_gen) = self.generations[idx].checked_add(1)
            && next_gen <= MAX_GENERATION
        {
            self.generations[idx] = next_gen;
            self.free_list.push(id.index());
        } else {
            // Retire slots whose generation counter saturates so handles never wrap or go negative.
            self.generations[idx] = u32::MAX;
        }

        true
    }

    /// Reports whether an entity handle matches a currently live slot and generation.
    ///
    /// - `id`: generational entity handle to verify.
    pub fn is_alive(&self, id: EntityId) -> bool {
        let idx = id.index() as usize;
        idx < self.alive.len() && self.alive[idx] && self.generations[idx] == id.generation()
    }

    /// Returns the count of currently live entities in this world.
    pub fn len(&self) -> usize {
        self.spawn_order.len()
    }

    /// Reports whether this world currently holds zero live entities.
    pub fn is_empty(&self) -> bool {
        self.spawn_order.is_empty()
    }

    /// Returns the total number of entity slots ever allocated, including free slots.
    pub fn slot_capacity(&self) -> usize {
        self.generations.len()
    }

    /// Returns the number of non-empty core and script-defined component stores.
    pub fn store_count(&self) -> usize {
        let core = [!self.positions.is_empty(),
            !self.sprites.is_empty(),
            !self.names.is_empty(),
            !self.players.is_empty(),
            !self.solids.is_empty()]
        .into_iter()
        .filter(|&present| present)
        .count();
        core + self.custom.len()
    }

    /// Attaches or updates the world coordinate of a live entity.
    ///
    /// Returns false without mutating storage if the entity handle is dead.
    ///
    /// - `id`: target entity handle.
    /// - `pos`: world grid coordinate to store.
    pub fn set_position(&mut self, id: EntityId, pos: Coord) -> bool {
        if !self.is_alive(id) {
            return false;
        }
        self.positions.insert(id, pos);
        true
    }

    /// Returns the world coordinate attached to an entity if live and present.
    ///
    /// - `id`: target entity handle.
    pub fn position(&self, id: EntityId) -> Option<Coord> {
        self.positions.get(id).copied()
    }

    /// Detaches the position component from an entity, returning whether one existed.
    ///
    /// - `id`: target entity handle.
    pub fn remove_position(&mut self, id: EntityId) -> bool {
        self.positions.remove(id).is_some()
    }

    /// Attaches or replaces the visual sprite component of a live entity.
    ///
    /// Returns false without mutating storage if the entity handle is dead.
    ///
    /// - `id`: target entity handle.
    /// - `sprite`: palette sprite key and render layer.
    pub fn set_sprite(&mut self, id: EntityId, sprite: Sprite) -> bool {
        if !self.is_alive(id) {
            return false;
        }
        self.sprites.insert(id, sprite);
        true
    }

    /// Returns the sprite component attached to an entity if live and present.
    ///
    /// - `id`: target entity handle.
    pub fn sprite(&self, id: EntityId) -> Option<&Sprite> {
        self.sprites.get(id)
    }

    /// Detaches the sprite component from an entity, returning whether one existed.
    ///
    /// - `id`: target entity handle.
    pub fn remove_sprite(&mut self, id: EntityId) -> bool {
        self.sprites.remove(id).is_some()
    }

    /// Attaches or updates the lookup name of a live entity.
    ///
    /// Returns false without mutating storage if the entity handle is dead.
    ///
    /// - `id`: target entity handle.
    /// - `name`: label used for lookups and inspector display.
    pub fn set_name(&mut self, id: EntityId, name: impl Into<String>) -> bool {
        if !self.is_alive(id) {
            return false;
        }
        self.names.insert(id, name.into());
        true
    }

    /// Returns the name string attached to an entity if live and present.
    ///
    /// - `id`: target entity handle.
    pub fn name(&self, id: EntityId) -> Option<&str> {
        self.names.get(id).map(String::as_str)
    }

    /// Detaches the name component from an entity, returning whether one existed.
    ///
    /// - `id`: target entity handle.
    pub fn remove_name(&mut self, id: EntityId) -> bool {
        self.names.remove(id).is_some()
    }

    /// Sets or clears the player control marker on a live entity.
    ///
    /// Returns false without mutating storage if the entity handle is dead.
    ///
    /// - `id`: target entity handle.
    /// - `is_player`: true to attach the marker or false to remove it.
    pub fn set_player(&mut self, id: EntityId, is_player: bool) -> bool {
        if !self.is_alive(id) {
            return false;
        }
        if is_player {
            self.players.insert(id, ());
        } else {
            self.players.remove(id);
        }
        true
    }

    /// Reports whether an entity carries the player control marker.
    ///
    /// - `id`: target entity handle.
    pub fn is_player(&self, id: EntityId) -> bool {
        self.players.contains(id)
    }

    /// Sets or clears the solid collision marker on a live entity.
    ///
    /// Returns false without mutating storage if the entity handle is dead.
    ///
    /// - `id`: target entity handle.
    /// - `is_solid`: true to attach the marker or false to remove it.
    pub fn set_solid(&mut self, id: EntityId, is_solid: bool) -> bool {
        if !self.is_alive(id) {
            return false;
        }
        if is_solid {
            self.solids.insert(id, ());
        } else {
            self.solids.remove(id);
        }
        true
    }

    /// Reports whether an entity carries the solid collision marker.
    ///
    /// - `id`: target entity handle.
    pub fn is_solid(&self, id: EntityId) -> bool {
        self.solids.contains(id)
    }

    /// Attaches or replaces a custom script component on a live entity.
    ///
    /// Returns false if the entity handle is dead or if the component name collides with a built-in
    /// core component key.
    ///
    /// - `id`: target entity handle.
    /// - `name`: dynamic component store key.
    /// - `value`: pure-Rust representation of the script value.
    pub fn set_custom(&mut self, id: EntityId, name: &str, value: ScriptValue) -> bool {
        if !self.is_alive(id) || is_core_component(name) {
            return false;
        }
        self.custom
            .entry(name.to_string())
            .or_default()
            .insert(id, value);
        true
    }

    /// Returns a custom script component attached to an entity if present.
    ///
    /// - `id`: target entity handle.
    /// - `name`: dynamic component store key.
    pub fn custom(&self, id: EntityId, name: &str) -> Option<&ScriptValue> {
        self.custom.get(name)?.get(id)
    }

    /// Detaches a custom script component and prunes its backing store if empty.
    ///
    /// - `id`: target entity handle.
    /// - `name`: dynamic component store key.
    pub fn remove_custom(&mut self, id: EntityId, name: &str) -> bool {
        let Some(store) = self.custom.get_mut(name) else {
            return false;
        };
        let removed = store.remove(id).is_some();
        if store.is_empty() {
            self.custom.remove(name);
        }
        removed
    }

    /// Reports whether an entity carries a named core or script-defined component.
    ///
    /// - `id`: target entity handle.
    /// - `name`: core component key or dynamic script component key.
    pub fn has_component(&self, id: EntityId, name: &str) -> bool {
        match name {
            "pos" => self.positions.contains(id),
            "sprite" => self.sprites.contains(id),
            "name" => self.names.contains(id),
            "player" => self.players.contains(id),
            "solid" => self.solids.contains(id),
            other => self.custom.get(other).is_some_and(|s| s.contains(id)),
        }
    }

    /// Detaches any named core or script-defined component from an entity.
    ///
    /// - `id`: target entity handle.
    /// - `name`: core component key or dynamic script component key.
    pub fn remove_component(&mut self, id: EntityId, name: &str) -> bool {
        match name {
            "pos" => self.remove_position(id),
            "sprite" => self.remove_sprite(id),
            "name" => self.remove_name(id),
            "player" => self.players.remove(id).is_some(),
            "solid" => self.solids.remove(id).is_some(),
            other => self.remove_custom(id, other),
        }
    }

    /// Returns the active player handle and position for input dispatch and camera follow.
    ///
    /// When multiple entities carry the player marker, returns the earliest spawned entity that also
    /// has a valid position component.
    pub fn player(&self) -> Option<(EntityId, Coord)> {
        self.players
            .entities()
            .iter()
            .copied()
            .filter_map(|id| self.positions.get(id).map(|&pos| (id, pos)))
            .min_by_key(|(id, _)| self.spawn_order_of(*id))
    }

    /// Returns the earliest spawned live entity carrying the requested name if any exists.
    ///
    /// - `name`: exact entity label to search for.
    pub fn find_by_name(&self, name: &str) -> Option<EntityId> {
        self.names
            .iter()
            .filter(|(_, val)| val.as_str() == name)
            .map(|(id, _)| id)
            .min_by_key(|id| self.spawn_order_of(*id))
    }

    /// Returns all live entities carrying every requested component in creation order.
    ///
    /// Passing an empty slice returns every live entity in creation order.
    ///
    /// - `components`: slice of core or dynamic component names that matching entities must possess.
    pub fn query(&self, components: &[&str]) -> Vec<EntityId> {
        if components.is_empty() {
            return self.live_entities();
        }

        // Probe the smallest backing store first so multi-component filters skip unrelated entities.
        let mut smallest: Option<&[EntityId]> = None;
        for &name in components {
            let Some(entities) = self.component_entities(name) else {
                return Vec::new();
            };
            if entities.is_empty() {
                return Vec::new();
            }
            if smallest.is_none_or(|cur| entities.len() < cur.len()) {
                smallest = Some(entities);
            }
        }

        let Some(candidates) = smallest else {
            return Vec::new();
        };
        let mut matches: Vec<EntityId> = candidates
            .iter()
            .copied()
            .filter(|&id| components.iter().all(|&name| self.has_component(id, name)))
            .collect();
        matches.sort_by_key(|&id| self.spawn_order_of(id));
        matches
    }

    /// Returns all live entities occupying the exact world coordinate in creation order.
    ///
    /// - `pos`: world grid coordinate to match.
    pub fn at(&self, pos: Coord) -> Vec<EntityId> {
        let mut matches: Vec<EntityId> = self
            .positions
            .iter()
            .filter(|&(_, p)| *p == pos)
            .map(|(id, _)| id)
            .collect();
        matches.sort_by_key(|&id| self.spawn_order_of(id));
        matches
    }

    /// Returns all renderable entities sorted back-to-front by layer, player status, and spawn order.
    pub fn renderables(&self) -> Vec<(EntityId, Coord, &Sprite, bool)> {
        let mut items: Vec<(EntityId, Coord, &Sprite, bool)> = self
            .sprites
            .iter()
            .filter_map(|(id, sprite)| {
                let pos = self.positions.get(id).copied()?;
                Some((id, pos, sprite, self.is_player(id)))
            })
            .collect();
        items.sort_by_key(|(id, _, sprite, is_player)| {
            (sprite.layer, *is_player, self.spawn_order_of(*id))
        });
        items
    }

    /// Returns every live entity handle sorted deterministically by creation order.
    pub fn live_entities(&self) -> Vec<EntityId> {
        let mut pairs: Vec<(EntityId, u64)> = self
            .spawn_order
            .iter()
            .map(|(id, &order)| (id, order))
            .collect();
        pairs.sort_by_key(|&(_, order)| order);
        pairs.into_iter().map(|(id, _)| id).collect()
    }

    /// Formats every component currently attached to an entity for inspection overlays.
    ///
    /// Returns core components first in a fixed order followed by custom components sorted
    /// alphabetically by component name.
    ///
    /// - `id`: target entity handle to inspect.
    pub fn inspect_components(&self, id: EntityId) -> Vec<(String, String)> {
        if !self.is_alive(id) {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(name) = self.name(id) {
            out.push(("name".to_string(), format!("\"{name}\"")));
        }
        if let Some(pos) = self.position(id) {
            out.push(("pos".to_string(), format!("({}, {})", pos.x, pos.y)));
        }
        if let Some(sprite) = self.sprite(id) {
            out.push((
                "sprite".to_string(),
                format!("tile=\"{}\" layer={}", sprite.tile, sprite.layer),
            ));
        }
        if self.is_player(id) {
            out.push(("player".to_string(), "true".to_string()));
        }
        if self.is_solid(id) {
            out.push(("solid".to_string(), "true".to_string()));
        }
        for (key, store) in &self.custom {
            if let Some(val) = store.get(id) {
                out.push((key.clone(), val.format_compact()));
            }
        }
        out
    }

    /// Returns the creation sequence number for an entity, or maximum sentinel if absent.
    fn spawn_order_of(&self, id: EntityId) -> u64 {
        self.spawn_order.get(id).copied().unwrap_or(u64::MAX)
    }

    /// Returns the dense entity slice backing a core or custom component store.
    fn component_entities(&self, name: &str) -> Option<&[EntityId]> {
        match name {
            "pos" => Some(self.positions.entities()),
            "sprite" => Some(self.sprites.entities()),
            "name" => Some(self.names.entities()),
            "player" => Some(self.players.entities()),
            "solid" => Some(self.solids.entities()),
            other => self.custom.get(other).map(SparseSet::entities),
        }
    }
}

/// Reports whether a component name belongs to the typed core engine stores.
fn is_core_component(name: &str) -> bool {
    matches!(name, "pos" | "sprite" | "name" | "player" | "solid")
}

#[cfg(test)]
mod tests {
    //! Unit tests verifying generational handles, sparse-set swap-remove, queries, and inspection.

    use super::*;

    #[test]
    fn entity_id_reuses_slots_with_incremented_generation_and_roundtrips_lua_int() {
        let mut ecs = EcsWorld::new();
        let a = ecs.spawn();
        let b = ecs.spawn();
        assert_eq!((a.index(), a.generation()), (0, 1));
        assert_eq!((b.index(), b.generation()), (1, 1));
        assert_eq!(EntityId::from_lua_int(a.to_lua_int()), Some(a));
        assert_eq!(EntityId::from_lua_int(0), None);
        assert_eq!(EntityId::from_lua_int(-5), None);

        assert!(ecs.despawn(a));
        assert!(!ecs.is_alive(a));
        assert!(!ecs.despawn(a));

        let c = ecs.spawn();
        assert_eq!(c.index(), 0);
        assert_eq!(c.generation(), 2);
        assert_ne!(a, c);
        assert!(!ecs.is_alive(a));
        assert!(ecs.is_alive(c));
    }

    #[test]
    fn sparse_set_swap_remove_preserves_remaining_entries_and_rejects_stale_handles() {
        let mut ecs = EcsWorld::new();
        let e0 = ecs.spawn();
        let e1 = ecs.spawn();
        let e2 = ecs.spawn();

        let mut set = SparseSet::new();
        set.insert(e0, 10);
        set.insert(e1, 20);
        set.insert(e2, 30);

        // Removing the first item swap-moves e2 into dense index 0.
        assert_eq!(set.remove(e0), Some(10));
        assert_eq!(set.get(e0), None);
        assert_eq!(set.get(e1), Some(&20));
        assert_eq!(set.get(e2), Some(&30));

        assert!(ecs.despawn(e0));
        let e0_v2 = ecs.spawn();
        assert_eq!(set.get(e0_v2), None);
        assert!(!set.contains(e0_v2));
    }

    #[test]
    fn query_intersects_smallest_store_deterministically_and_supports_spatial_lookup() {
        let mut ecs = EcsWorld::new();
        let player = ecs.spawn();
        ecs.set_name(player, "player");
        ecs.set_position(player, Coord::new(0, 0));
        ecs.set_player(player, true);
        ecs.set_solid(player, true);

        let gem = ecs.spawn();
        ecs.set_name(gem, "gem1");
        ecs.set_position(gem, Coord::new(3, 2));
        ecs.set_custom(gem, "gem", ScriptValue::Bool(true));

        let crab = ecs.spawn();
        ecs.set_name(crab, "crab1");
        ecs.set_position(crab, Coord::new(3, 2));
        ecs.set_solid(crab, true);
        ecs.set_custom(crab, "crab", ScriptValue::Bool(true));

        assert_eq!(ecs.query(&["pos", "crab"]), vec![crab]);
        assert_eq!(ecs.query(&["pos", "solid"]), vec![player, crab]);
        assert!(ecs.query(&["pos", "missing"]).is_empty());
        assert_eq!(ecs.at(Coord::new(3, 2)), vec![gem, crab]);
        assert_eq!(ecs.find_by_name("gem1"), Some(gem));
        assert_eq!(ecs.player(), Some((player, Coord::new(0, 0))));
    }

    #[test]
    fn renderables_sort_by_layer_and_inspector_formats_core_and_custom_components() {
        let mut ecs = EcsWorld::new();

        let player = ecs.spawn();
        ecs.set_name(player, "player");
        ecs.set_position(player, Coord::new(1, 1));
        ecs.set_sprite(
            player,
            Sprite {
                tile: "@".to_string(),
                layer: 10,
            },
        );
        ecs.set_player(player, true);

        let gem = ecs.spawn();
        ecs.set_name(gem, "gem1");
        ecs.set_position(gem, Coord::new(1, 1));
        ecs.set_sprite(
            gem,
            Sprite {
                tile: "G".to_string(),
                layer: 2,
            },
        );
        let mut stats = BTreeMap::new();
        stats.insert("value".to_string(), ScriptValue::Int(5));
        ecs.set_custom(gem, "loot", ScriptValue::Table(stats));

        let order: Vec<EntityId> = ecs.renderables().into_iter().map(|(id, ..)| id).collect();
        assert_eq!(order, vec![gem, player]);

        let fields = ecs.inspect_components(gem);
        assert_eq!(
            fields,
            vec![
                ("name".to_string(), "\"gem1\"".to_string()),
                ("pos".to_string(), "(1, 1)".to_string()),
                ("sprite".to_string(), "tile=\"G\" layer=2".to_string()),
                ("loot".to_string(), "{ value=5 }".to_string()),
            ]
        );
    }
}
