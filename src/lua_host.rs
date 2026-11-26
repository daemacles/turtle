//! Sandboxed Lua script loader, scoped ECS userdata bridge, and atomic hot-reload watcher.
//!
//! Instantiates isolated Lua virtual machines for world scripts, registers the host entity world
//! userdata interface, captures Lua print calls into an off-screen buffer, and swaps virtual
//! machines atomically on reload so script errors freeze the previous valid world instead of
//! crashing or corrupting terminal output.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime};

use mlua::prelude::*;
use terminal_graphics::Colour;

use crate::ecs::{EcsWorld, EntityId, ScriptValue, Sprite};
use crate::model::{
    CellWidth, Coord, DEFAULT_IDLE_MS, HudUpdate, Intent, IntentTarget, Pacing, RectBounds,
    TileStyle, WorldConfig,
};

/// Minimum interval between file metadata checks when polling for script edits.
const WATCH_INTERVAL: Duration = Duration::from_millis(150);

/// Maximum nested table depth accepted when marshaling dynamic Lua components into Rust.
const MAX_SCRIPT_DEPTH: usize = 16;

/// Loaded Lua virtual machine bound to one validated world script table.
///
/// Owns its isolated Lua state and parsed static configuration while leaving authoritative entity
/// and frame state in Rust. Reloading constructs a separate candidate instance so failed edits
/// never disturb the active simulation.
pub struct LuaWorld {
    /// Embedded Lua interpreter holding script globals and functions.
    lua: Lua,
    /// Registry handle pointing to the script's world table.
    world_key: LuaRegistryKey,
    /// Per-call buffer collecting lines written through Lua's global print function.
    print_buf: Rc<RefCell<Vec<String>>>,
    /// Parsed visual legend mapping tile keys to terminal styles.
    palette: HashMap<String, TileStyle>,
    /// Static metadata and pacing settings declared by the script.
    config: WorldConfig,
}

impl LuaWorld {
    /// Loads and validates a world script from disk inside a fresh Lua virtual machine.
    ///
    /// Reads the file contents into memory, derives a default world title from the filename stem,
    /// and executes the chunk without modifying any previously running world instance.
    ///
    /// - `path`: filesystem location of the Lua world script to load.
    ///
    /// # Errors
    ///
    /// Returns a [LuaError] if reading the file fails, if Lua compilation or top-level execution
    /// raises an error, or if required world fields fail validation.
    pub fn load_from_path(path: &Path) -> LuaResult<Self> {
        let source = std::fs::read_to_string(path).map_err(LuaError::external)?;
        let default_title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("world");
        let chunk_name = format!("={}", path.display());
        Self::load_from_source(&source, &chunk_name, default_title)
    }

    /// Compiles and validates an in-memory Lua world script inside a fresh virtual machine.
    ///
    /// Evaluates the source chunk, registers host userdata types, resolves the exported world
    /// table, and validates required hooks.
    ///
    /// - `source`: complete Lua chunk text defining the world.
    /// - `chunk_name`: diagnostic chunk name shown in Lua syntax and runtime stack traces.
    /// - `default_title`: fallback display title used when the script omits `title`.
    ///
    /// # Errors
    ///
    /// Returns a [LuaError] if compilation fails, if no world table is found, if required hooks are
    /// absent, or if palette or pacing fields contain invalid values.
    pub fn load_from_source(
        source: &str,
        chunk_name: &str,
        default_title: &str,
    ) -> LuaResult<Self> {
        let lua = Lua::new();
        lua.register_userdata_type::<EcsWorld>(register_ecs_methods)?;
        let print_buf = Rc::new(RefCell::new(Vec::new()));
        install_print_capture(&lua, Rc::clone(&print_buf))?;

        let evaluated = lua.load(source).set_name(chunk_name).eval::<LuaValue>()?;
        let world_tbl = match evaluated {
            LuaValue::Table(tbl) => tbl,
            LuaValue::Nil => lua.globals().get::<LuaTable>("world").map_err(|_| {
                LuaError::external("script must return a world table or define global 'world'")
            })?,
            _ => {
                return Err(LuaError::external(
                    "world script must return a table or define global 'world'",
                ));
            }
        };

        let config = parse_world_config(&world_tbl, default_title)?;
        let palette = parse_palette(&world_tbl.get::<LuaTable>("palette")?)?;

        // Validate required callable hooks eagerly before accepting a hot-reload candidate.
        let _: LuaFunction = world_tbl.get("step")?;
        let _: LuaFunction = world_tbl.get("view")?;

        let world_key = lua.create_registry_value(world_tbl)?;

        Ok(Self {
            lua,
            world_key,
            print_buf,
            palette,
            config,
        })
    }

    /// Returns static configuration settings declared by this world script.
    pub fn config(&self) -> &WorldConfig {
        &self.config
    }

    /// Returns the parsed tile visual palette keyed by short script strings.
    pub fn palette(&self) -> &HashMap<String, TileStyle> {
        &self.palette
    }

    /// Seeds Lua's random generator and populates a fresh entity world via the init hook.
    ///
    /// Passes a scoped mutable handle to the script's initialization function so Lua cannot retain
    /// a dangling reference after initialization returns.
    ///
    /// - `seed`: deterministic integer seed passed to `math.randomseed` and `init`.
    ///
    /// # Errors
    ///
    /// Returns a [LuaError] if seeding Lua fails, if `init` raises an error, or if any spawned
    /// entity component fails validation.
    pub fn init(&self, seed: u64) -> LuaResult<(EcsWorld, HudUpdate, Vec<String>)> {
        self.print_buf.borrow_mut().clear();
        seed_lua_rng(&self.lua, seed)?;

        let world_tbl: LuaTable = self.lua.registry_value(&self.world_key)?;
        let maybe_init: Option<LuaFunction> = world_tbl.get("init")?;
        let mut ecs = EcsWorld::new();
        let Some(init_fn) = maybe_init else {
            let printed = std::mem::take(&mut *self.print_buf.borrow_mut());
            return Ok((ecs, HudUpdate::default(), printed));
        };

        let hud_val = self.lua.scope(|scope| {
            let ecs_ud = scope.create_any_userdata_ref_mut(&mut ecs)?;
            init_fn.call::<LuaValue>((seed as i64, ecs_ud))
        })?;

        let hud = match hud_val {
            LuaValue::Table(tbl) => parse_hud(&tbl)?,
            LuaValue::Nil => HudUpdate::default(),
            _ => {
                return Err(LuaError::external(
                    "world.init return value must be a hud table or nil",
                ));
            }
        };

        let printed = std::mem::take(&mut *self.print_buf.borrow_mut());
        Ok((ecs, hud, printed))
    }

    /// Advances world logic by one step against a transactional copy of the entity world.
    ///
    /// Clones the live entity world before entering the scoped script call so any Lua runtime error
    /// drops the half-mutated working copy and preserves the caller's previous state untouched.
    ///
    /// - `t`: target simulation step counter starting at one after initialization.
    /// - `intents`: external player or cursor actions queued for this step.
    /// - `current_ecs`: authoritative entity world snapshot before this step runs.
    ///
    /// # Errors
    ///
    /// Returns a [LuaError] if calling `step` raises a script error, supplies malformed component
    /// updates, or returns a non-table HUD value.
    pub fn step(
        &self,
        t: u64,
        intents: &[Intent],
        current_ecs: &EcsWorld,
    ) -> LuaResult<(EcsWorld, HudUpdate, Vec<String>)> {
        self.print_buf.borrow_mut().clear();

        let requests_tbl = self.lua.create_table_with_capacity(intents.len(), 0)?;
        for (idx, intent) in intents.iter().enumerate() {
            let (dx, dy) = intent.delta();
            let req = self.lua.create_table_with_capacity(0, 6)?;
            match intent.target {
                IntentTarget::Entity(id) => req.set("id", id.to_lua_int())?,
                IntentTarget::Cursor => req.set("id", "$cursor")?,
            }
            req.set("action", intent.action_name())?;
            req.set("x", intent.origin.x)?;
            req.set("y", intent.origin.y)?;
            req.set("dx", dx)?;
            req.set("dy", dy)?;
            requests_tbl.raw_set(idx + 1, req)?;
        }

        let mut working_ecs = current_ecs.clone();
        let world_tbl: LuaTable = self.lua.registry_value(&self.world_key)?;
        let step_fn: LuaFunction = world_tbl.get("step")?;

        let hud_val = self.lua.scope(|scope| {
            let ecs_ud = scope.create_any_userdata_ref_mut(&mut working_ecs)?;
            step_fn.call::<LuaValue>((t as i64, requests_tbl, ecs_ud))
        })?;

        let hud = match hud_val {
            LuaValue::Table(tbl) => parse_hud(&tbl)?,
            LuaValue::Nil => HudUpdate::default(),
            _ => {
                return Err(LuaError::external(
                    "world.step return value must be a hud table or nil",
                ));
            }
        };

        let printed = std::mem::take(&mut *self.print_buf.borrow_mut());
        Ok((working_ecs, hud, printed))
    }

    /// Queries Lua for the visible tile rows covering the inclusive bounding box.
    ///
    /// Passes inclusive minimum and maximum corner tables and simulation step `t`, then resolves
    /// each returned palette key or inline style override into concrete [TileStyle] cells.
    ///
    /// - `bounds`: inclusive world bounding box sized to the active viewport dimensions.
    /// - `t`: current simulation step counter available for ambient tile animation.
    ///
    /// # Errors
    ///
    /// Returns a [LuaError] if `view` raises a script error, returns mismatched row or column
    /// counts, or references an undefined palette key or color name.
    pub fn view(
        &self,
        bounds: RectBounds,
        t: u64,
    ) -> LuaResult<(Vec<Vec<TileStyle>>, Vec<String>)> {
        self.print_buf.borrow_mut().clear();

        let start_tbl = self.lua.create_table_with_capacity(0, 2)?;
        start_tbl.set("x", bounds.min.x)?;
        start_tbl.set("y", bounds.min.y)?;

        let end_tbl = self.lua.create_table_with_capacity(0, 2)?;
        end_tbl.set("x", bounds.max.x)?;
        end_tbl.set("y", bounds.max.y)?;

        let world_tbl: LuaTable = self.lua.registry_value(&self.world_key)?;
        let view_fn: LuaFunction = world_tbl.get("view")?;
        let rows_tbl = view_fn.call::<LuaTable>((start_tbl, end_tbl, t as i64))?;

        let expected_rows = bounds.height();
        let expected_cols = bounds.width();
        let mut grid = Vec::with_capacity(expected_rows);

        for ry in 1..=expected_rows {
            let row_val: LuaValue = rows_tbl.raw_get(ry)?;
            let row_cells = match row_val {
                LuaValue::Table(row_tbl) => {
                    let mut cells = Vec::with_capacity(expected_cols);
                    for rx in 1..=expected_cols {
                        let cell_val: LuaValue = row_tbl.raw_get(rx)?;
                        cells.push(self.resolve_cell(cell_val, rx, ry)?);
                    }
                    cells
                }
                LuaValue::String(row_str) => {
                    let s = row_str.to_str()?;
                    let mut cells = Vec::with_capacity(expected_cols);
                    let mut chars = s.chars();
                    for rx in 1..=expected_cols {
                        let Some(ch) = chars.next() else {
                            return Err(LuaError::external(format!(
                                "world.view row {ry} ended at column {rx} (expected {expected_cols})"
                            )));
                        };
                        let mut key_buf = [0u8; 4];
                        let key = ch.encode_utf8(&mut key_buf);
                        let style = self.palette.get(key).ok_or_else(|| {
                            LuaError::external(format!(
                                "unknown palette key '{key}' in world.view at ({rx}, {ry})"
                            ))
                        })?;
                        cells.push(style.clone());
                    }
                    cells
                }
                LuaValue::Nil => {
                    return Err(LuaError::external(format!(
                        "world.view returned missing row {ry} (expected {expected_rows} rows)"
                    )));
                }
                _ => {
                    return Err(LuaError::external(format!(
                        "world.view row {ry} must be an array or string"
                    )));
                }
            };
            grid.push(row_cells);
        }

        let printed = std::mem::take(&mut *self.print_buf.borrow_mut());
        Ok((grid, printed))
    }

    /// Resolves one cell value returned inside a row table into a concrete tile style.
    fn resolve_cell(&self, cell: LuaValue, rx: usize, ry: usize) -> LuaResult<TileStyle> {
        match cell {
            LuaValue::String(key) => {
                let k = key.to_str()?;
                self.palette.get(k.as_ref()).cloned().ok_or_else(|| {
                    LuaError::external(format!(
                        "unknown palette key '{}' at viewport ({rx}, {ry})",
                        k
                    ))
                })
            }
            LuaValue::Table(tbl) => parse_tile_style(&tbl, Some(Colour::Black)),
            LuaValue::Nil => Err(LuaError::external(format!(
                "missing cell at viewport ({rx}, {ry})"
            ))),
            _ => Err(LuaError::external(format!(
                "cell at viewport ({rx}, {ry}) must be a palette key string or style table"
            ))),
        }
    }
}

/// Registers the host entity-component methods exposed to Lua on scoped userdata handles.
fn register_ecs_methods(reg: &mut LuaUserDataRegistry<EcsWorld>) {
    reg.add_method_mut("spawn", |_, ecs, components: Option<LuaTable>| {
        let id = ecs.spawn();
        if let Some(tbl) = components {
            for pair in tbl.pairs::<String, LuaValue>() {
                let (key, val) = match pair {
                    Ok(kv) => kv,
                    Err(err) => {
                        ecs.despawn(id);
                        return Err(err);
                    }
                };
                if let Err(err) = set_entity_component(ecs, id, &key, val) {
                    ecs.despawn(id);
                    return Err(err);
                }
            }
        }
        Ok(id.to_lua_int())
    });

    reg.add_method_mut("despawn", |_, ecs, raw_id: i64| {
        let Some(id) = EntityId::from_lua_int(raw_id) else {
            return Ok(false);
        };
        Ok(ecs.despawn(id))
    });

    reg.add_method("alive", |_, ecs, raw_id: i64| {
        Ok(EntityId::from_lua_int(raw_id).is_some_and(|id| ecs.is_alive(id)))
    });

    reg.add_method("get", |lua, ecs, (raw_id, name): (i64, String)| {
        let Some(id) = EntityId::from_lua_int(raw_id).filter(|&id| ecs.is_alive(id)) else {
            return Ok(LuaValue::Nil);
        };
        get_entity_component(lua, ecs, id, &name)
    });

    reg.add_method_mut(
        "set",
        |_, ecs, (raw_id, name, val): (i64, String, LuaValue)| {
            let id = require_live_entity(ecs, raw_id)?;
            set_entity_component(ecs, id, &name, val)
        },
    );

    reg.add_method_mut("remove", |_, ecs, (raw_id, name): (i64, String)| {
        let Some(id) = EntityId::from_lua_int(raw_id).filter(|&id| ecs.is_alive(id)) else {
            return Ok(false);
        };
        Ok(ecs.remove_component(id, &name))
    });

    reg.add_method("has", |_, ecs, (raw_id, name): (i64, String)| {
        let Some(id) = EntityId::from_lua_int(raw_id).filter(|&id| ecs.is_alive(id)) else {
            return Ok(false);
        };
        Ok(ecs.has_component(id, &name))
    });

    reg.add_method("query", |lua, ecs, names: LuaVariadic<String>| {
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let ids = ecs.query(&refs);
        pack_entity_ids(lua, &ids)
    });

    reg.add_method("at", |lua, ecs, (x, y): (i32, i32)| {
        let ids = ecs.at(Coord::new(x, y));
        pack_entity_ids(lua, &ids)
    });

    reg.add_method("find_by_name", |_, ecs, name: String| {
        Ok(ecs.find_by_name(&name).map(EntityId::to_lua_int))
    });

    reg.add_method("count", |_, ecs, ()| Ok(ecs.len() as i64));
}

/// Validates that a Lua integer decodes to a currently live entity handle.
fn require_live_entity(ecs: &EcsWorld, raw_id: i64) -> LuaResult<EntityId> {
    let id = EntityId::from_lua_int(raw_id)
        .ok_or_else(|| LuaError::external(format!("invalid entity handle {raw_id}")))?;
    if !ecs.is_alive(id) {
        return Err(LuaError::external(format!(
            "stale or dead entity handle {id}"
        )));
    }
    Ok(id)
}

/// Packs a slice of Rust entity handles into a one-based Lua array table.
fn pack_entity_ids(lua: &Lua, ids: &[EntityId]) -> LuaResult<LuaTable> {
    let tbl = lua.create_table_with_capacity(ids.len(), 0)?;
    for (idx, id) in ids.iter().enumerate() {
        tbl.raw_set(idx + 1, id.to_lua_int())?;
    }
    Ok(tbl)
}

/// Reads one core or custom component from a live entity and marshals it to Lua.
fn get_entity_component(
    lua: &Lua,
    ecs: &EcsWorld,
    id: EntityId,
    name: &str,
) -> LuaResult<LuaValue> {
    match name {
        "pos" => {
            let Some(pos) = ecs.position(id) else {
                return Ok(LuaValue::Nil);
            };
            let tbl = lua.create_table_with_capacity(0, 2)?;
            tbl.set("x", pos.x)?;
            tbl.set("y", pos.y)?;
            Ok(LuaValue::Table(tbl))
        }
        "sprite" => {
            let Some(sprite) = ecs.sprite(id) else {
                return Ok(LuaValue::Nil);
            };
            let tbl = lua.create_table_with_capacity(0, 2)?;
            tbl.set("tile", sprite.tile.as_str())?;
            tbl.set("layer", sprite.layer)?;
            Ok(LuaValue::Table(tbl))
        }
        "name" => match ecs.name(id) {
            Some(label) => Ok(LuaValue::String(lua.create_string(label)?)),
            None => Ok(LuaValue::Nil),
        },
        "player" => Ok(if ecs.is_player(id) {
            LuaValue::Boolean(true)
        } else {
            LuaValue::Nil
        }),
        "solid" => Ok(if ecs.is_solid(id) {
            LuaValue::Boolean(true)
        } else {
            LuaValue::Nil
        }),
        other => match ecs.custom(id, other) {
            Some(val) => script_value_to_lua(lua, val),
            None => Ok(LuaValue::Nil),
        },
    }
}

/// Writes or detaches one core or custom component on a live entity.
fn set_entity_component(
    ecs: &mut EcsWorld,
    id: EntityId,
    name: &str,
    val: LuaValue,
) -> LuaResult<()> {
    if matches!(val, LuaValue::Nil) {
        ecs.remove_component(id, name);
        return Ok(());
    }

    match name {
        "pos" => {
            let LuaValue::Table(tbl) = val else {
                return Err(LuaError::external(
                    "component 'pos' must be a table { x = integer, y = integer }",
                ));
            };
            let x: i32 = tbl.get("x")?;
            let y: i32 = tbl.get("y")?;
            ecs.set_position(id, Coord::new(x, y));
            Ok(())
        }
        "sprite" => {
            let sprite = match val {
                LuaValue::String(s) => Sprite {
                    tile: s.to_str()?.to_string(),
                    layer: 0,
                },
                LuaValue::Table(tbl) => {
                    let tile = tbl
                        .get::<Option<String>>("tile")?
                        .or(tbl.get::<Option<String>>(1)?)
                        .ok_or_else(|| {
                            LuaError::external(
                                "component 'sprite' table must specify 'tile' or index 1",
                            )
                        })?;
                    let layer = tbl.get::<Option<i32>>("layer")?.unwrap_or(0);
                    Sprite { tile, layer }
                }
                _ => {
                    return Err(LuaError::external(
                        "component 'sprite' must be a palette key string or { tile, layer? } table",
                    ));
                }
            };
            ecs.set_sprite(id, sprite);
            Ok(())
        }
        "name" => {
            let LuaValue::String(s) = val else {
                return Err(LuaError::external("component 'name' must be a string"));
            };
            ecs.set_name(id, s.to_str()?.as_ref());
            Ok(())
        }
        "player" => {
            let LuaValue::Boolean(flag) = val else {
                return Err(LuaError::external("component 'player' must be a boolean"));
            };
            ecs.set_player(id, flag);
            Ok(())
        }
        "solid" => {
            let LuaValue::Boolean(flag) = val else {
                return Err(LuaError::external("component 'solid' must be a boolean"));
            };
            ecs.set_solid(id, flag);
            Ok(())
        }
        other => {
            let script_val = lua_to_script_value(val, 0)?;
            ecs.set_custom(id, other, script_val);
            Ok(())
        }
    }
}

/// Converts a dynamic Lua value into a pure-Rust value tree with bounded recursion depth.
fn lua_to_script_value(val: LuaValue, depth: usize) -> LuaResult<ScriptValue> {
    if depth > MAX_SCRIPT_DEPTH {
        return Err(LuaError::external(format!(
            "component value exceeds maximum nesting depth of {MAX_SCRIPT_DEPTH}"
        )));
    }
    match val {
        LuaValue::Boolean(b) => Ok(ScriptValue::Bool(b)),
        LuaValue::Integer(n) => Ok(ScriptValue::Int(n)),
        LuaValue::Number(f) => Ok(ScriptValue::Float(f)),
        LuaValue::String(s) => Ok(ScriptValue::Str(s.to_str()?.to_string())),
        LuaValue::Table(tbl) => {
            let mut entries = Vec::new();
            for pair in tbl.pairs::<LuaValue, LuaValue>() {
                entries.push(pair?);
            }
            if entries.is_empty() {
                return Ok(ScriptValue::Table(BTreeMap::new()));
            }

            let mut list_items: Vec<(i64, ScriptValue)> = Vec::with_capacity(entries.len());
            let mut is_dense_array = true;
            for (k, _) in &entries {
                if let LuaValue::Integer(idx) = k
                    && *idx >= 1
                {
                    continue;
                }
                is_dense_array = false;
                break;
            }

            if is_dense_array {
                for (k, v) in &entries {
                    if let LuaValue::Integer(idx) = k {
                        list_items.push((*idx, lua_to_script_value(v.clone(), depth + 1)?));
                    }
                }
                list_items.sort_by_key(|(idx, _)| *idx);
                let contiguous = list_items
                    .iter()
                    .enumerate()
                    .all(|(pos, (idx, _))| *idx == (pos as i64) + 1);
                if contiguous {
                    return Ok(ScriptValue::List(
                        list_items.into_iter().map(|(_, v)| v).collect(),
                    ));
                }
            }

            let mut map = BTreeMap::new();
            for (k, v) in entries {
                let key_str = match k {
                    LuaValue::String(s) => s.to_str()?.to_string(),
                    LuaValue::Integer(i) => i.to_string(),
                    _ => {
                        return Err(LuaError::external(
                            "custom component table keys must be strings or integers",
                        ));
                    }
                };
                map.insert(key_str, lua_to_script_value(v, depth + 1)?);
            }
            Ok(ScriptValue::Table(map))
        }
        _ => Err(LuaError::external(
            "custom component values may only contain booleans, numbers, strings, or plain tables",
        )),
    }
}

/// Converts a stored script value tree back into a Lua value inside the active interpreter.
fn script_value_to_lua(lua: &Lua, val: &ScriptValue) -> LuaResult<LuaValue> {
    match val {
        ScriptValue::Bool(b) => Ok(LuaValue::Boolean(*b)),
        ScriptValue::Int(n) => Ok(LuaValue::Integer(*n)),
        ScriptValue::Float(f) => Ok(LuaValue::Number(*f)),
        ScriptValue::Str(s) => Ok(LuaValue::String(lua.create_string(s)?)),
        ScriptValue::List(items) => {
            let tbl = lua.create_table_with_capacity(items.len(), 0)?;
            for (idx, item) in items.iter().enumerate() {
                tbl.raw_set(idx + 1, script_value_to_lua(lua, item)?)?;
            }
            Ok(LuaValue::Table(tbl))
        }
        ScriptValue::Table(map) => {
            let tbl = lua.create_table_with_capacity(0, map.len())?;
            for (k, v) in map {
                tbl.set(k.as_str(), script_value_to_lua(lua, v)?)?;
            }
            Ok(LuaValue::Table(tbl))
        }
    }
}

/// Polling file watcher tracking modification time and byte size for one script path.
///
/// Checking metadata rather than filesystem inode watches handles editors that save files via
/// temporary files and atomic renames, while requiring no external watcher crates.
pub struct FileStampWatcher {
    /// Filesystem path polled for modifications.
    path: PathBuf,
    /// Most recently observed modified timestamp and byte length.
    last_stamp: Option<(SystemTime, u64)>,
    /// Monotonic timestamp of the most recent metadata poll.
    last_checked: Instant,
}

impl FileStampWatcher {
    /// Creates a watcher initialized with the file's current modification stamp if present.
    ///
    /// - `path`: script path whose disk updates trigger hot-reload checks.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let last_stamp = read_file_stamp(&path);
        Self {
            path,
            last_stamp,
            last_checked: Instant::now(),
        }
    }

    /// Checks whether the watched file changed since the last observed stamp.
    ///
    /// Skips filesystem calls until [WATCH_INTERVAL] has elapsed since the previous check. Returns
    /// false if the file is momentarily unavailable mid-rename.
    ///
    /// - `now`: current monotonic timestamp from the simulation loop.
    pub fn poll_changed(&mut self, now: Instant) -> bool {
        if now.duration_since(self.last_checked) < WATCH_INTERVAL {
            return false;
        }
        self.last_checked = now;

        let Some(stamp) = read_file_stamp(&self.path) else {
            return false;
        };
        if Some(stamp) != self.last_stamp {
            self.last_stamp = Some(stamp);
            return true;
        }
        false
    }
}

/// Reads the current modification time and byte length for a script path.
fn read_file_stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    Some((modified, meta.len()))
}

/// Replaces the global print function with a handler that appends formatted lines to the buffer.
fn install_print_capture(lua: &Lua, sink: Rc<RefCell<Vec<String>>>) -> LuaResult<()> {
    let print_fn = lua.create_function(move |lua, args: LuaMultiValue| {
        let tostring: LuaFunction = lua.globals().get("tostring")?;
        let mut parts = Vec::with_capacity(args.len());
        for value in args {
            let text: String = tostring.call(value)?;
            parts.push(text);
        }
        sink.borrow_mut().push(parts.join("\t"));
        Ok(())
    })?;
    lua.globals().set("print", print_fn)?;
    Ok(())
}

/// Seeds Lua's random number generator so procedural generation replays deterministically.
fn seed_lua_rng(lua: &Lua, seed: u64) -> LuaResult<()> {
    let math: LuaTable = lua.globals().get("math")?;
    let randomseed: LuaFunction = math.get("randomseed")?;
    randomseed.call::<()>(seed as i64)?;
    Ok(())
}

/// Parses optional world title, tile width, seed, and pacing configuration from the world table.
fn parse_world_config(world_tbl: &LuaTable, default_title: &str) -> LuaResult<WorldConfig> {
    let title = world_tbl
        .get::<Option<String>>("title")?
        .unwrap_or_else(|| default_title.to_string());

    let cell_width = match world_tbl.get::<Option<i64>>("cell_width")? {
        None | Some(2) => CellWidth::Two,
        Some(1) => CellWidth::One,
        Some(other) => {
            return Err(LuaError::external(format!(
                "invalid cell_width {other}: expected 1 or 2"
            )));
        }
    };

    let declared_seed = match world_tbl.get::<Option<i64>>("seed")? {
        None => None,
        Some(s) if s >= 0 => Some(s as u64),
        Some(other) => {
            return Err(LuaError::external(format!(
                "invalid seed {other}: expected non-negative integer"
            )));
        }
    };

    let pacing = match world_tbl.get::<Option<LuaTable>>("pacing")? {
        None => Pacing::default(),
        Some(pt) => parse_pacing(&pt)?,
    };

    Ok(WorldConfig {
        title,
        cell_width,
        pacing,
        declared_seed,
    })
}

/// Parses a pacing configuration table selecting turn, tick, or hybrid timing.
fn parse_pacing(tbl: &LuaTable) -> LuaResult<Pacing> {
    let mode = tbl
        .get::<Option<String>>("mode")?
        .unwrap_or_else(|| "hybrid".to_string());
    match mode.as_str() {
        "turn" => Ok(Pacing::Turn),
        "tick" => {
            let tick_ms = if let Some(ms) = tbl.get::<Option<u64>>("tick_ms")? {
                ms.max(16)
            } else if let Some(hz) = tbl.get::<Option<f64>>("hz")? {
                (1000.0 / hz.max(0.5)).round() as u64
            } else {
                120
            };
            Ok(Pacing::Tick { tick_ms })
        }
        "hybrid" => {
            let idle_ms = tbl
                .get::<Option<u64>>("idle_ms")?
                .unwrap_or(DEFAULT_IDLE_MS)
                .max(50);
            Ok(Pacing::Hybrid { idle_ms })
        }
        other => Err(LuaError::external(format!(
            "invalid pacing mode '{other}': expected 'turn', 'tick', or 'hybrid'"
        ))),
    }
}

/// Parses the script's palette legend into named tile style definitions.
fn parse_palette(tbl: &LuaTable) -> LuaResult<HashMap<String, TileStyle>> {
    let mut map = HashMap::new();
    for pair in tbl.pairs::<String, LuaValue>() {
        let (key, val) = pair?;
        let LuaValue::Table(entry) = val else {
            return Err(LuaError::external(format!(
                "palette entry '{key}' must be a table {{ glyph, fg, bg? }}"
            )));
        };
        let style = parse_tile_style(&entry, None)?;
        map.insert(key, style);
    }
    Ok(map)
}

/// Parses one tile style table supporting positional indices or named glyph and color keys.
fn parse_tile_style(tbl: &LuaTable, default_bg: Option<Colour>) -> LuaResult<TileStyle> {
    let ch = tbl
        .get::<Option<String>>("ch")?
        .or(tbl.get::<Option<String>>(1)?)
        .ok_or_else(|| LuaError::external("tile style missing glyph string ('ch' or index 1)"))?;

    let fg_name = tbl
        .get::<Option<String>>("fg")?
        .or(tbl.get::<Option<String>>(2)?)
        .ok_or_else(|| LuaError::external("tile style missing foreground color ('fg' or index 2)"))?;
    let fg = parse_colour(&fg_name)?;

    let bg_val: LuaValue = match tbl.get::<LuaValue>("bg")? {
        LuaValue::Nil => tbl.get::<LuaValue>(3)?,
        other => other,
    };
    let bg = match bg_val {
        LuaValue::Nil => default_bg,
        LuaValue::String(s) => Some(parse_colour(s.to_str()?.as_ref())?),
        _ => {
            return Err(LuaError::external(
                "tile style background ('bg' or index 3) must be a color name string or nil",
            ));
        }
    };

    Ok(TileStyle::from_spec(&ch, fg, bg))
}

/// Parses an exact terminal color variant name into a color value.
fn parse_colour(name: &str) -> LuaResult<Colour> {
    Colour::from_str(name).map_err(|_| {
        LuaError::external(format!(
            "unknown Colour '{name}' (examples: Black, Blue, Green, BrightYellow, BrightCyan)"
        ))
    })
}

/// Parses optional status and event strings from the HUD update table.
fn parse_hud(tbl: &LuaTable) -> LuaResult<HudUpdate> {
    Ok(HudUpdate {
        status: tbl.get("status")?,
        message: tbl.get("msg")?,
    })
}

#[cfg(test)]
mod tests {
    //! Unit tests covering script loading, scoped ECS userdata, rollback, and shadow reload.

    use super::*;
    use crate::model::{
        Action, Camera, DEFAULT_VIEWPORT_COLS_TWO, DEFAULT_VIEWPORT_ROWS,
    };

    #[test]
    fn loads_world_tidekeep_and_steps_into_viewport() {
        let world = LuaWorld::load_from_path(Path::new("world.lua")).unwrap();
        let (mut ecs, hud, _) = world.init(42).unwrap();
        assert!(!ecs.is_empty());
        assert!(hud.status.is_some());

        let (player_id, player_pos) = ecs.player().unwrap();
        let intents = vec![Intent {
            target: IntentTarget::Entity(player_id),
            action: Action::Move { dx: 1, dy: 0 },
            origin: player_pos,
        }];
        let (next_ecs, _, _) = world.step(1, &intents, &ecs).unwrap();
        ecs = next_ecs;

        let mut cam = Camera::default();
        let (_, moved_pos) = ecs.player().unwrap();
        cam.snap_to(moved_pos);
        let bounds = cam.bounds(
            world.config().cell_width.default_viewport_cols(),
            DEFAULT_VIEWPORT_ROWS,
        );
        let (grid, _) = world.view(bounds, 1).unwrap();
        assert_eq!(grid.len(), DEFAULT_VIEWPORT_ROWS);
        assert_eq!(grid[0].len(), DEFAULT_VIEWPORT_COLS_TWO);
    }

    #[test]
    fn loads_and_steps_life_and_simple_room_examples() {
        for path in ["examples/life.lua", "examples/simple_room.lua"] {
            let world = LuaWorld::load_from_path(Path::new(path)).unwrap();
            let (ecs, _, _) = world.init(42).unwrap();
            let (next_ecs, _, _) = world.step(1, &[], &ecs).unwrap();
            let cam = Camera::default();
            let bounds = cam.bounds(
                world.config().cell_width.default_viewport_cols(),
                DEFAULT_VIEWPORT_ROWS,
            );
            let (grid, _) = world.view(bounds, 1).unwrap();
            assert_eq!(grid.len(), DEFAULT_VIEWPORT_ROWS);
            assert!(next_ecs.len() <= ecs.len() + 1);
        }
    }

    #[test]
    fn step_rolls_back_mutations_on_runtime_error_and_rejects_escaped_handles() {
        let script = r#"
            local world = {
                palette = { ["."] = { " .", "White", "Black" } },
            }
            local leaked = nil
            function world.init(_seed, ecs)
                ecs:spawn({ name = "hero", pos = { x = 0, y = 0 }, sprite = "@", player = true })
            end
            function world.step(t, _req, ecs)
                if t == 1 then
                    leaked = ecs
                    local hero = ecs:find_by_name("hero")
                    ecs:set(hero, "pos", { x = 99, y = 99 })
                    error("boom mid-step")
                end
                return { status = tostring(leaked:count()) }
            end
            function world.view(start_pos, end_pos, _t)
                local rows = {}
                for y = start_pos.y, end_pos.y do
                    local row = {}
                    for x = start_pos.x, end_pos.x do
                        row[#row + 1] = "."
                    end
                    rows[#rows + 1] = row
                end
                return rows
            end
            return world
        "#;

        let world = LuaWorld::load_from_source(script, "=rollback_test", "test").unwrap();
        let (ecs, _, _) = world.init(1).unwrap();
        let hero = ecs.find_by_name("hero").unwrap();
        assert_eq!(ecs.position(hero), Some(Coord::new(0, 0)));

        // Step 1 mutates hero then errors: caller snapshot must remain untouched.
        assert!(world.step(1, &[], &ecs).is_err());
        assert_eq!(ecs.position(hero), Some(Coord::new(0, 0)));

        // Step 2 tries to call the leaked userdata handle captured in step 1: must fail safely.
        assert!(world.step(2, &[], &ecs).is_err());
    }

    #[test]
    fn shadow_reload_rejects_broken_script_without_disturbing_active_world() {
        let active = LuaWorld::load_from_path(Path::new("examples/simple_room.lua")).unwrap();
        let (ecs, _, _) = active.init(7).unwrap();

        let broken = LuaWorld::load_from_source("local world = { broken = ", "=broken", "broken");
        assert!(broken.is_err());

        let (next_ecs, _, _) = active.step(1, &[], &ecs).unwrap();
        assert_eq!(next_ecs.len(), ecs.len());
    }

    #[test]
    fn tidekeep_v3_ecs_systems_cover_bridges_combat_pickups_status_and_ai() {
        let world = LuaWorld::load_from_path(Path::new("world.lua")).unwrap();
        let (base_ecs, _, _) = world.init(42).unwrap();
        let (player_id, _) = base_ecs.player().unwrap();
        assert!(base_ecs.has_component(player_id, "hp"));
        assert!(base_ecs.has_component(player_id, "combat"));
        assert!(base_ecs.has_component(player_id, "inventory"));
        assert!(!base_ecs.query(&["pickup"]).is_empty());
        assert!(base_ecs.query(&["ai"]).len() >= 3);

        // 1. Walk east twice to push crate1 into (3,0) water, creating a bridge entity.
        let mut ecs = base_ecs.clone();
        for tick in 1..=2 {
            let (_, pos) = ecs.player().unwrap();
            let intent = Intent {
                target: IntentTarget::Entity(player_id),
                action: Action::Move { dx: 1, dy: 0 },
                origin: pos,
            };
            let (next_ecs, _, _) = world.step(tick, &[intent], &ecs).unwrap();
            ecs = next_ecs;
        }
        assert!(ecs.find_by_name("crate1").is_none());
        let bridges = ecs.query(&["bridge"]);
        assert_eq!(bridges.len(), 1);
        assert_eq!(ecs.position(bridges[0]), Some(Coord::new(3, 0)));
        assert_eq!(ecs.position(player_id), Some(Coord::new(2, 0)));

        // Stepping east again stands the player on top of the bridge entity.
        let step_on_bridge = Intent {
            target: IntentTarget::Entity(player_id),
            action: Action::Move { dx: 1, dy: 0 },
            origin: Coord::new(2, 0),
        };
        let (ecs, _, _) = world.step(3, &[step_on_bridge], &ecs).unwrap();
        assert_eq!(ecs.position(player_id), Some(Coord::new(3, 0)));
        assert_eq!(ecs.at(Coord::new(3, 0)).len(), 2);

        // 2. Collect tonic1 at (1,-1) and verify inventory + despawn.
        let mut ecs = base_ecs.clone();
        for (tick, (dx, dy)) in [(1, (1, 0)), (2, (0, -1))] {
            let (_, pos) = ecs.player().unwrap();
            let intent = Intent {
                target: IntentTarget::Entity(player_id),
                action: Action::Move { dx, dy },
                origin: pos,
            };
            let (next_ecs, _, _) = world.step(tick, &[intent], &ecs).unwrap();
            ecs = next_ecs;
        }
        assert!(ecs.find_by_name("tonic1").is_none());
        let ScriptValue::Table(inv) = ecs.custom(player_id, "inventory").unwrap() else {
            panic!("expected inventory table");
        };
        assert_eq!(inv.get("tonics"), Some(&ScriptValue::Int(2)));

        // 3. Salt Flare attaches stun status, ticks it down over 3 steps, then prunes it.
        let mut ecs = base_ecs;
        let crab1 = ecs.find_by_name("crab1").unwrap();
        ecs.set_position(crab1, Coord::new(0, -1));
        let flare_intent = Intent {
            target: IntentTarget::Entity(player_id),
            action: Action::Act,
            origin: Coord::new(0, 0),
        };
        let (mut ecs, _, _) = world.step(1, &[flare_intent], &ecs).unwrap();
        assert!(ecs.has_component(crab1, "status"));
        assert_eq!(ecs.sprite(crab1).unwrap().tile, "Z");
        assert_eq!(ecs.position(crab1), Some(Coord::new(0, -1)));

        for tick in 2..=4 {
            let (next_ecs, _, _) = world.step(tick, &[], &ecs).unwrap();
            ecs = next_ecs;
        }
        assert!(!ecs.has_component(crab1, "status"));
        assert_eq!(ecs.sprite(crab1).unwrap().tile, "C");

        // 4. Melee bumps drop crab1 to 1 HP ("flee") and then despawn it at 0 HP.
        ecs.set_position(crab1, Coord::new(0, -1));
        let bump1 = Intent {
            target: IntentTarget::Entity(player_id),
            action: Action::Move { dx: 0, dy: -1 },
            origin: Coord::new(0, 0),
        };
        let (mut ecs, _, _) = world.step(5, &[bump1], &ecs).unwrap();
        let ScriptValue::Table(hp) = ecs.custom(crab1, "hp").unwrap() else {
            panic!("expected hp table");
        };
        assert_eq!(hp.get("cur"), Some(&ScriptValue::Int(1)));
        let ScriptValue::Table(ai) = ecs.custom(crab1, "ai").unwrap() else {
            panic!("expected ai table");
        };
        assert_eq!(ai.get("state"), Some(&ScriptValue::Str("flee".to_string())));

        ecs.set_position(crab1, Coord::new(0, -1));
        let bump2 = Intent {
            target: IntentTarget::Entity(player_id),
            action: Action::Move { dx: 0, dy: -1 },
            origin: Coord::new(0, 0),
        };
        let (ecs, _, _) = world.step(6, &[bump2], &ecs).unwrap();
        assert!(!ecs.is_alive(crab1));
    }
}
