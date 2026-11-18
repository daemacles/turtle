//! Sandboxed Lua script loader, ABI marshaler, and atomic hot-reload file watcher.
//!
//! Instantiates isolated Lua virtual machines for world scripts, converts between Rust world
//! structs and Lua tables, captures Lua print calls into an off-screen buffer, and swaps virtual
//! machines atomically on reload so script errors freeze the previous valid world instead of
//! crashing or corrupting terminal output.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime};

use mlua::prelude::*;
use terminal_graphics::Colour;

use crate::model::{
    CellWidth, Coord, DEFAULT_IDLE_MS, Entity, EntityPatch, EntityStore, HudUpdate, Intent, Pacing,
    RectBounds, TileStyle, WorldConfig,
};

/// Minimum interval between file metadata checks when polling for script edits.
const WATCH_INTERVAL: Duration = Duration::from_millis(150);

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
    /// Evaluates the source chunk, resolves the exported world table, and validates required hooks.
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

    /// Seeds Lua's random generator and invokes the script's optional initialization hook.
    ///
    /// Returns the starting entity list, initial status and message update, and any lines printed
    /// during startup.
    ///
    /// - `seed`: deterministic integer seed passed to `math.randomseed` and `init`.
    ///
    /// # Errors
    ///
    /// Returns a [LuaError] if seeding Lua fails, if `init` raises an error, or if any returned
    /// entity record lacks required coordinates or palette keys.
    pub fn init(&self, seed: u64) -> LuaResult<(Vec<Entity>, HudUpdate, Vec<String>)> {
        self.print_buf.borrow_mut().clear();
        seed_lua_rng(&self.lua, seed)?;

        let world_tbl: LuaTable = self.lua.registry_value(&self.world_key)?;
        let maybe_init: Option<LuaFunction> = world_tbl.get("init")?;
        let Some(init_fn) = maybe_init else {
            let printed = std::mem::take(&mut *self.print_buf.borrow_mut());
            return Ok((Vec::new(), HudUpdate::default(), printed));
        };

        let results = init_fn.call::<LuaMultiValue>(seed as i64)?;
        let mut iter = results.into_iter();
        let entities = match iter.next() {
            Some(LuaValue::Table(tbl)) => parse_initial_entities(&tbl)?,
            Some(LuaValue::Nil) | None => Vec::new(),
            Some(_) => {
                return Err(LuaError::external(
                    "world.init first return value must be an entity array or nil",
                ));
            }
        };
        let hud = match iter.next() {
            Some(LuaValue::Table(tbl)) => parse_hud(&tbl)?,
            Some(LuaValue::Nil) | None => HudUpdate::default(),
            Some(_) => {
                return Err(LuaError::external(
                    "world.init second return value must be a hud table or nil",
                ));
            }
        };

        let printed = std::mem::take(&mut *self.print_buf.borrow_mut());
        Ok((entities, hud, printed))
    }

    /// Advances world logic by one step with player intents and an entity snapshot.
    ///
    /// Marshals intents and the entity snapshot into Lua, invokes the step hook, and parses patches.
    ///
    /// - `t`: target simulation step counter starting at one after initialization.
    /// - `intents`: external player or cursor actions queued for this step.
    /// - `store`: authoritative entity snapshot read by Lua for collision and AI decisions.
    ///
    /// # Errors
    ///
    /// Returns a [LuaError] if calling `step` raises a script error or returns malformed entity
    /// patch entries.
    pub fn step(
        &self,
        t: u64,
        intents: &[Intent],
        store: &EntityStore,
    ) -> LuaResult<(Vec<EntityPatch>, HudUpdate, Vec<String>)> {
        self.print_buf.borrow_mut().clear();

        let requests_tbl = self.lua.create_table_with_capacity(intents.len(), 0)?;
        for (idx, intent) in intents.iter().enumerate() {
            let (dx, dy) = intent.delta();
            let req = self.lua.create_table_with_capacity(0, 6)?;
            req.set("id", intent.entity_id.as_str())?;
            req.set("action", intent.action_name())?;
            req.set("x", intent.origin.x)?;
            req.set("y", intent.origin.y)?;
            req.set("dx", dx)?;
            req.set("dy", dy)?;
            requests_tbl.raw_set(idx + 1, req)?;
        }

        let entities_tbl = self.lua.create_table_with_capacity(0, store.len())?;
        for entity in store.iter() {
            let entry = self.lua.create_table_with_capacity(0, 5)?;
            entry.set("id", entity.id.as_str())?;
            entry.set("x", entity.pos.x)?;
            entry.set("y", entity.pos.y)?;
            entry.set("tile", entity.tile.as_str())?;
            entry.set("player", entity.is_player)?;
            entities_tbl.set(entity.id.as_str(), entry)?;
        }

        let world_tbl: LuaTable = self.lua.registry_value(&self.world_key)?;
        let step_fn: LuaFunction = world_tbl.get("step")?;
        let results = step_fn.call::<LuaMultiValue>((t as i64, requests_tbl, entities_tbl))?;

        let mut iter = results.into_iter();
        let patches = match iter.next() {
            Some(LuaValue::Table(tbl)) => parse_entity_patches(&tbl)?,
            Some(LuaValue::Nil) | None => Vec::new(),
            Some(_) => {
                return Err(LuaError::external(
                    "world.step first return value must be an update array or nil",
                ));
            }
        };
        let hud = match iter.next() {
            Some(LuaValue::Table(tbl)) => parse_hud(&tbl)?,
            Some(LuaValue::Nil) | None => HudUpdate::default(),
            Some(_) => {
                return Err(LuaError::external(
                    "world.step second return value must be a hud table or nil",
                ));
            }
        };

        let printed = std::mem::take(&mut *self.print_buf.borrow_mut());
        Ok((patches, hud, printed))
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

/// Parses the initial entity sequence returned by script initialization.
fn parse_initial_entities(tbl: &LuaTable) -> LuaResult<Vec<Entity>> {
    let mut entities = Vec::new();
    for val in tbl.sequence_values::<LuaValue>() {
        let LuaValue::Table(entry) = val? else {
            return Err(LuaError::external(
                "each entry returned by world.init must be a table",
            ));
        };
        let id: String = entry.get("id")?;
        let x: i32 = entry.get("x")?;
        let y: i32 = entry.get("y")?;
        let tile: String = entry.get("tile")?;
        let is_player = entry.get::<Option<bool>>("player")?.unwrap_or(false);
        entities.push(Entity {
            id,
            pos: Coord::new(x, y),
            tile,
            is_player,
        });
    }
    Ok(entities)
}

/// Parses the delta update sequence returned by a simulation step.
fn parse_entity_patches(tbl: &LuaTable) -> LuaResult<Vec<EntityPatch>> {
    let mut patches = Vec::new();
    for val in tbl.sequence_values::<LuaValue>() {
        let LuaValue::Table(entry) = val? else {
            return Err(LuaError::external(
                "each entry returned by world.step must be an update table",
            ));
        };
        let id: String = entry.get("id")?;
        let remove = entry.get::<Option<bool>>("remove")?.unwrap_or(false);
        let x: Option<i32> = entry.get("x")?;
        let y: Option<i32> = entry.get("y")?;
        let pos = match (x, y) {
            (Some(px), Some(py)) => Some(Coord::new(px, py)),
            (None, None) => None,
            _ => {
                return Err(LuaError::external(format!(
                    "update for entity '{id}' must specify both x and y when moving or spawning"
                )));
            }
        };
        let tile: Option<String> = entry.get("tile")?;
        let is_player: Option<bool> = entry.get("player")?;
        patches.push(EntityPatch {
            id,
            pos,
            tile,
            is_player,
            remove,
        });
    }
    Ok(patches)
}

/// Parses optional status and event strings from the second return value.
fn parse_hud(tbl: &LuaTable) -> LuaResult<HudUpdate> {
    Ok(HudUpdate {
        status: tbl.get("status")?,
        message: tbl.get("msg")?,
    })
}

#[cfg(test)]
mod tests {
    //! Unit tests covering script loading, entity patching, view queries, and shadow reload isolation.

    use super::*;
    use crate::model::{Action, Camera, VIEWPORT_ROWS};

    #[test]
    fn loads_world_tidekeep_and_steps_into_viewport() {
        let world = LuaWorld::load_from_path(Path::new("world.lua")).unwrap();
        let (init_entities, hud, _) = world.init(42).unwrap();
        assert!(!init_entities.is_empty());
        assert!(hud.status.is_some());

        let mut store = EntityStore::from_entities(init_entities);
        let player = store.player().unwrap().clone();
        let intents = vec![Intent {
            entity_id: player.id,
            action: Action::Move { dx: 1, dy: 0 },
            origin: player.pos,
        }];
        let (patches, _, _) = world.step(1, &intents, &store).unwrap();
        store.apply_patches(&patches);

        let mut cam = Camera::default();
        cam.snap_to(store.player().unwrap().pos);
        let bounds = cam.bounds(world.config().cell_width.viewport_cols(), VIEWPORT_ROWS);
        let (grid, _) = world.view(bounds, 1).unwrap();
        assert_eq!(grid.len(), VIEWPORT_ROWS);
        assert_eq!(grid[0].len(), world.config().cell_width.viewport_cols());
    }

    #[test]
    fn loads_and_steps_life_and_simple_room_examples() {
        for path in ["examples/life.lua", "examples/simple_room.lua"] {
            let world = LuaWorld::load_from_path(Path::new(path)).unwrap();
            let (entities, _, _) = world.init(42).unwrap();
            let mut store = EntityStore::from_entities(entities);
            let (patches, _, _) = world.step(1, &[], &store).unwrap();
            store.apply_patches(&patches);
            let cam = Camera::default();
            let bounds = cam.bounds(world.config().cell_width.viewport_cols(), VIEWPORT_ROWS);
            let (grid, _) = world.view(bounds, 1).unwrap();
            assert_eq!(grid.len(), VIEWPORT_ROWS);
        }
    }

    #[test]
    fn shadow_reload_rejects_broken_script_without_disturbing_active_world() {
        let active = LuaWorld::load_from_path(Path::new("examples/simple_room.lua")).unwrap();
        let (entities, _, _) = active.init(7).unwrap();
        let store = EntityStore::from_entities(entities);

        let broken = LuaWorld::load_from_source("local world = { broken = ", "=broken", "broken");
        assert!(broken.is_err());

        let (patches, _, _) = active.step(1, &[], &store).unwrap();
        assert!(patches.len() <= store.len() + 1);
    }
}
