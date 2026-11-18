//! Interactive terminal game and simulation engine driven by hot-reloaded Lua world scripts.
//!
//! Launches a fixed-size character-grid viewport around an embedded Lua world controller, polls
//! raw keyboard input without blocking timed updates, manages authoritative entity storage, and
//! atomically reloads modified scripts back to time zero without losing terminal stability on
//! mid-edit syntax errors.

mod lua_host;
mod model;
mod render;
mod tty;

use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mlua::prelude::*;
use terminal_graphics::{Colour, Display};

use crate::lua_host::{FileStampWatcher, LuaWorld};
use crate::model::{
    Action, Camera, CellWidth, EntityStore, Intent, MessageLog, Pacing, RepeatCadence, TileStyle, VIEWPORT_ROWS,
};
use crate::render::{FRAME_HEIGHT, FRAME_WIDTH, FrameState};
use crate::tty::{RawInput, TerminalGuard};

/// Duration that the outer border stays highlighted green after a successful hot-reload.
const RELOAD_FLASH_DURATION: Duration = Duration::from_millis(300);

/// Maximum poll wait in milliseconds so file-modification checks run promptly while idle.
const WATCH_POLL_CAP_MS: i32 = 150;

/// Parsed command-line options controlling which world script runs and optional headless steps.
struct CliOptions {
    /// Path to the Lua world script loaded at startup and watched for edits.
    script_path: PathBuf,
    /// Optional bounded step count for non-interactive smoke runs.
    max_steps: Option<u64>,
}

/// Complete mutable runtime state owned by the simulation loop.
struct EngineState {
    /// Path to the Lua world script being executed and watched.
    script_path: PathBuf,
    /// Active validated Lua world instance, or nothing when initial startup failed interactively.
    world: Option<LuaWorld>,
    /// Most recent script-declared seed observed, used to detect deliberate seed edits on reload.
    last_declared_seed: Option<u64>,
    /// Metadata watcher detecting editor saves to the world script.
    watcher: FileStampWatcher,
    /// Authoritative entity store shared across simulation steps and rendering.
    store: EntityStore,
    /// Viewport camera following the player deadzone or manual panning.
    camera: Camera,
    /// Key-repeat throttle protecting exploration movement from terminal repeat floods.
    cadence: RepeatCadence,
    /// Bounded queue of recent event and print messages shown below the viewport.
    log: MessageLog,
    /// Optional status summary string returned by the active world script.
    hud_status: String,
    /// Current random seed passed to world initialization.
    seed: u64,
    /// Current simulation step counter starting at zero after initialization or reset.
    tick: u64,
    /// Whether timed or hybrid automatic stepping is currently paused.
    paused: bool,
    /// Timestamp until which the reload confirmation highlight stays active.
    reload_flash_until: Option<Instant>,
    /// Next scheduled monotonic deadline for tick or hybrid idle stepping.
    next_step_at: Option<Instant>,
    /// Latest player intent queued in tick mode until the next clock beat fires.
    pending_tick_intent: Option<Intent>,
    /// Latest terrain rows returned by the active world script for the current camera bounds.
    terrain: Vec<Vec<TileStyle>>,
    /// Non-fatal script error shown inside an overlay while keeping the last valid state frozen.
    error_overlay: Option<String>,
    /// Last terminal dimensions seen when smaller than the required canvas, for redraw detection.
    last_too_small: Option<(u16, u16)>,
    /// Flag indicating that the frame buffer needs to be re-composed and printed.
    dirty: bool,
}

impl EngineState {
    /// Creates an initial unpopulated engine state ready to load a world script.
    fn new(script_path: PathBuf) -> Self {
        let watcher = FileStampWatcher::new(&script_path);
        Self {
            script_path,
            world: None,
            last_declared_seed: None,
            watcher,
            store: EntityStore::default(),
            camera: Camera::default(),
            cadence: RepeatCadence::new(),
            log: MessageLog::default(),
            hud_status: String::new(),
            seed: roll_random_seed(),
            tick: 0,
            paused: false,
            reload_flash_until: None,
            next_step_at: None,
            pending_tick_intent: None,
            terrain: default_blank_terrain(CellWidth::Two),
            error_overlay: None,
            last_too_small: None,
            dirty: true,
        }
    }

    /// Compiles and initializes the script in an isolated candidate virtual machine before swapping.
    ///
    /// Preserves the previous world, entities, and terrain intact if compilation, initialization, or
    /// the initial viewport query fails, recording the diagnostic in `error_overlay`.
    fn reload_from_disk(
        &mut self,
        now: Instant,
        fresh_seed: bool,
        announce_reload: bool,
    ) -> LuaResult<()> {
        let candidate = LuaWorld::load_from_path(&self.script_path)?;
        let declared = candidate.config().declared_seed;

        let next_seed = if fresh_seed {
            roll_random_seed()
        } else if declared.is_some() && (self.world.is_none() || declared != self.last_declared_seed)
        {
            declared.unwrap_or(self.seed)
        } else {
            self.seed
        };

        let (entities, hud, init_prints) = candidate.init(next_seed)?;
        let store = EntityStore::from_entities(entities);
        let mut camera = Camera::default();
        if let Some(player) = store.player() {
            camera.snap_to(player.pos);
        }

        let bounds = camera.bounds(candidate.config().cell_width.viewport_cols(), VIEWPORT_ROWS);
        let (terrain, view_prints) = candidate.view(bounds, 0)?;

        self.world = Some(candidate);
        self.last_declared_seed = declared;
        self.seed = next_seed;
        self.store = store;
        self.camera = camera;
        self.terrain = terrain;
        self.tick = 0;
        self.paused = false;
        self.pending_tick_intent = None;
        self.cadence.reset();
        self.error_overlay = None;
        self.hud_status = hud.status.unwrap_or_default();
        self.log.clear();

        for line in init_prints.into_iter().chain(view_prints) {
            self.log.push_line(line);
        }
        if announce_reload {
            self.log.push_line(format!(
                "Reloaded {} -> t=0 (seed {:04X})",
                self.script_path.display(),
                self.seed & 0xffff
            ));
        }
        if let Some(msg) = hud.message {
            self.log.push_line(msg);
        }

        if announce_reload {
            self.reload_flash_until = Some(now + RELOAD_FLASH_DURATION);
        }
        self.schedule_next_step_from(now);
        self.dirty = true;
        Ok(())
    }

    /// Re-initializes the currently loaded world back to step zero without recompiling from disk.
    fn reset_loaded_world(&mut self, now: Instant, fresh_seed: bool) {
        if fresh_seed {
            self.seed = roll_random_seed();
        }
        let Some(world) = self.world.as_ref() else {
            if let Err(err) = self.reload_from_disk(now, fresh_seed, true) {
                self.set_error(err.to_string());
            }
            return;
        };

        let init_outcome = world.init(self.seed).and_then(|(entities, hud, init_prints)| {
            let store = EntityStore::from_entities(entities);
            let mut camera = Camera::default();
            if let Some(player) = store.player() {
                camera.snap_to(player.pos);
            }
            let bounds = camera.bounds(world.config().cell_width.viewport_cols(), VIEWPORT_ROWS);
            let (terrain, view_prints) = world.view(bounds, 0)?;
            Ok((store, camera, terrain, hud, init_prints, view_prints))
        });

        match init_outcome {
            Ok((store, camera, terrain, hud, init_prints, view_prints)) => {
                self.store = store;
                self.camera = camera;
                self.terrain = terrain;
                self.tick = 0;
                self.paused = false;
                self.pending_tick_intent = None;
                self.cadence.reset();
                self.error_overlay = None;
                self.hud_status = hud.status.unwrap_or_default();
                self.log.clear();
                for line in init_prints.into_iter().chain(view_prints) {
                    self.log.push_line(line);
                }
                self.log
                    .push_line(format!("Reset -> t=0 (seed {:04X})", self.seed & 0xffff));
                if let Some(msg) = hud.message {
                    self.log.push_line(msg);
                }
                self.schedule_next_step_from(now);
                self.dirty = true;
            }
            Err(err) => self.set_error(err.to_string()),
        }
    }

    /// Executes one simulation step and refreshes terrain for the active camera.
    fn execute_step(&mut self, intents: Vec<Intent>, now: Instant) -> LuaResult<()> {
        let Some(world) = self.world.as_ref() else {
            return Ok(());
        };

        let next_tick = self.tick.saturating_add(1);
        let (patches, hud, step_prints) = world.step(next_tick, &intents, &self.store)?;

        self.tick = next_tick;
        self.store.apply_patches(&patches);

        let cell_width = world.config().cell_width;
        if let Some(player) = self.store.player() {
            self.camera
                .follow_deadzone(player.pos, cell_width.viewport_cols(), VIEWPORT_ROWS);
        }

        for line in step_prints {
            self.log.push_line(line);
        }
        if let Some(status) = hud.status {
            self.hud_status = status;
        }
        if let Some(msg) = hud.message {
            self.log.push_line(msg);
        }

        let bounds = self
            .camera
            .bounds(cell_width.viewport_cols(), VIEWPORT_ROWS);
        let (terrain, view_prints) = world.view(bounds, self.tick)?;
        for line in view_prints {
            self.log.push_line(line);
        }
        self.terrain = terrain;
        self.schedule_next_step_from(now);
        self.dirty = true;
        Ok(())
    }

    /// Refreshes terrain for the current camera without advancing simulation time.
    fn refresh_view_only(&mut self) {
        let Some(world) = self.world.as_ref() else {
            return;
        };
        let bounds = self
            .camera
            .bounds(world.config().cell_width.viewport_cols(), VIEWPORT_ROWS);
        match world.view(bounds, self.tick) {
            Ok((terrain, view_prints)) => {
                for line in view_prints {
                    self.log.push_line(line);
                }
                self.terrain = terrain;
                self.dirty = true;
            }
            Err(err) => self.set_error(err.to_string()),
        }
    }

    /// Records a script failure into the error modal and pauses automatic stepping.
    fn set_error(&mut self, message: String) {
        self.error_overlay = Some(message);
        self.paused = true;
        self.next_step_at = None;
        self.dirty = true;
    }

    /// Schedules the next timed step deadline based on pacing mode and pause state.
    fn schedule_next_step_from(&mut self, now: Instant) {
        if self.paused || self.error_overlay.is_some() {
            self.next_step_at = None;
            return;
        }
        let pacing = self
            .world
            .as_ref()
            .map_or(Pacing::default(), |w| w.config().pacing);
        self.next_step_at = match pacing {
            Pacing::Turn => None,
            Pacing::Tick { tick_ms } => Some(now + Duration::from_millis(tick_ms)),
            Pacing::Hybrid { idle_ms } => Some(now + Duration::from_millis(idle_ms)),
        };
    }

    /// Computes the poll sleep duration in milliseconds capped for prompt file-watch checks.
    fn poll_timeout_ms(&self, now: Instant) -> i32 {
        let mut wait_ms = WATCH_POLL_CAP_MS;

        if let Some(deadline) = self.next_step_at {
            let rem = deadline.saturating_duration_since(now).as_millis();
            wait_ms = wait_ms.min(rem.min(i32::MAX as u128) as i32);
        }
        if let Some(flash_end) = self.reload_flash_until {
            let rem = flash_end.saturating_duration_since(now).as_millis();
            wait_ms = wait_ms.min(rem.min(i32::MAX as u128) as i32);
        }

        wait_ms.max(0)
    }

    /// Paints the current state onto the display buffer and flushes it if dirty.
    fn render_if_dirty(&mut self, display: &mut Display, now: Instant) -> io::Result<()> {
        let too_small = tty::terminal_size()
            .filter(|&(cols, rows)| cols < FRAME_WIDTH as u16 || rows < FRAME_HEIGHT as u16 + 1);
        if too_small != self.last_too_small {
            self.last_too_small = too_small;
            self.dirty = true;
        }

        if !self.dirty {
            return Ok(());
        }

        let empty_palette = HashMap::new();
        let (title, pacing, cell_width, palette) = match &self.world {
            Some(world) => (
                world.config().title.as_str(),
                world.config().pacing,
                world.config().cell_width,
                world.palette(),
            ),
            None => ("turtle", Pacing::Turn, CellWidth::Two, &empty_palette),
        };

        let reload_flash = self.reload_flash_until.is_some_and(|end| now < end);
        let bounds = self
            .camera
            .bounds(cell_width.viewport_cols(), VIEWPORT_ROWS);

        let frame = FrameState {
            title,
            pacing,
            cell_width,
            tick: self.tick,
            seed: self.seed,
            paused: self.paused,
            reload_flash,
            bounds,
            terrain: &self.terrain,
            entities: &self.store,
            palette,
            camera_center: self.camera.center,
            hud_status: &self.hud_status,
            log: &self.log,
            error_overlay: self.error_overlay.as_deref(),
            terminal_too_small: too_small,
        };

        render::compose_frame(display, &frame);
        display.print()?;
        self.dirty = false;
        Ok(())
    }
}

/// Parses command-line arguments selecting the script path and optional headless step count.
///
/// # Errors
///
/// Returns a descriptive usage message if an unknown flag or invalid step count is supplied.
fn parse_cli_args() -> Result<CliOptions, String> {
    let mut script_path: Option<PathBuf> = None;
    let mut max_steps: Option<u64> = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                return Err(
                    "Usage: turtle [script.lua] [--steps N]\nDefaults to world.lua when no script path is given."
                        .to_string(),
                );
            }
            "--steps" => {
                let raw = args
                    .next()
                    .ok_or_else(|| "--steps requires a non-negative integer".to_string())?;
                let parsed: u64 = raw
                    .parse()
                    .map_err(|_| format!("invalid --steps value '{raw}': expected integer"))?;
                max_steps = Some(parsed);
            }
            flag if flag.starts_with("--") => {
                return Err(format!("unknown option '{flag}' (pass --help for usage)"));
            }
            path_arg => {
                if script_path.is_none() {
                    script_path = Some(PathBuf::from(path_arg));
                } else {
                    return Err(format!("unexpected extra argument '{path_arg}'"));
                }
            }
        }
    }

    Ok(CliOptions {
        script_path: script_path.unwrap_or_else(|| PathBuf::from("world.lua")),
        max_steps,
    })
}

/// Collapses batched raw events so repeated directional keys keep only the latest direction.
fn coalesce_player_actions(events: Vec<RawInput>) -> (Vec<RawInput>, Option<Action>) {
    let mut controls = Vec::new();
    let mut latest_move: Option<Action> = None;
    let mut explicit_action: Option<Action> = None;

    for ev in events {
        match ev {
            RawInput::Action(action @ Action::Move { .. }) => latest_move = Some(action),
            RawInput::Action(action) => explicit_action = Some(action),
            other => controls.push(other),
        }
    }

    // Prefer a deliberate Wait or Act over an accidental earlier movement, unless Move came too.
    let chosen = latest_move.or(explicit_action);
    (controls, chosen)
}

/// Applies one player or cursor action according to entity presence and the active pacing mode.
fn dispatch_player_action(state: &mut EngineState, action: Action, now: Instant) {
    if state.error_overlay.is_some() {
        return;
    }

    if let Some(player) = state.store.player().cloned() {
        if let Action::Move { dx, dy } = action {
            let pacing = state
                .world
                .as_ref()
                .map_or(Pacing::default(), |w| w.config().pacing);
            if !matches!(pacing, Pacing::Tick { .. }) && !state.cadence.allow_move(dx, dy, now) {
                return;
            }
        }

        let intent = Intent {
            entity_id: player.id,
            action,
            origin: player.pos,
        };
        let pacing = state
            .world
            .as_ref()
            .map_or(Pacing::default(), |w| w.config().pacing);

        match pacing {
            Pacing::Tick { .. } => state.pending_tick_intent = Some(intent),
            Pacing::Turn | Pacing::Hybrid { .. } => {
                if let Err(err) = state.execute_step(vec![intent], now) {
                    state.set_error(err.to_string());
                }
            }
        }
        return;
    }

    // Playerless worlds (such as cellular automata) pan the camera directly and toggle at center.
    match action {
        Action::Move { dx, dy } => {
            state.camera.pan(dx, dy);
            state.refresh_view_only();
        }
        Action::Act => {
            let intent = Intent {
                entity_id: "$cursor".to_string(),
                action: Action::Act,
                origin: state.camera.center,
            };
            if let Err(err) = state.execute_step(vec![intent], now) {
                state.set_error(err.to_string());
            }
        }
        Action::Wait => {
            if let Err(err) = state.execute_step(Vec::new(), now) {
                state.set_error(err.to_string());
            }
        }
    }
}

/// Generates a compact random seed from monotonic clock nanoseconds.
fn roll_random_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(42, |d| d.as_nanos());
    ((nanos ^ (nanos >> 16)) & 0xffff) as u64
}

/// Creates a blank black terrain grid matching the active tile width.
fn default_blank_terrain(cell_width: CellWidth) -> Vec<Vec<TileStyle>> {
    let style = TileStyle::from_spec(" ", Colour::White, Some(Colour::Black));
    vec![vec![style; cell_width.viewport_cols()]; VIEWPORT_ROWS]
}

/// Runs fixed-step non-interactive smoke frames and exits cleanly.
fn run_smoke_steps(state: &mut EngineState, display: &mut Display, steps: u64) -> LuaResult<()> {
    let now = Instant::now();
    state.reload_from_disk(now, false, false)?;
    if let Err(err) = state.render_if_dirty(display, now)
        && err.kind() != ErrorKind::BrokenPipe
    {
        return Err(LuaError::external(err));
    }

    for _ in 0..steps {
        let step_now = Instant::now();
        state.execute_step(Vec::new(), step_now)?;
        if let Err(err) = state.render_if_dirty(display, step_now) {
            if err.kind() == ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(LuaError::external(err));
        }
    }

    Ok(())
}

/// Runs the interactive raw-mode event loop until the user quits or closes input.
fn run_interactive(state: &mut EngineState, display: &mut Display) -> io::Result<()> {
    let start = Instant::now();
    if let Err(err) = state.reload_from_disk(start, false, false) {
        state.set_error(err.to_string());
    }
    state.render_if_dirty(display, start)?;

    loop {
        let now = Instant::now();

        if let Some(flash_end) = state.reload_flash_until
            && now >= flash_end
        {
            state.reload_flash_until = None;
            state.dirty = true;
        }

        if state.watcher.poll_changed(now)
            && let Err(err) = state.reload_from_disk(now, false, true)
        {
            state.set_error(err.to_string());
        }

        let timeout_ms = state.poll_timeout_ms(now);
        let events = tty::poll_and_drain_inputs(timeout_ms)?;
        let step_now = Instant::now();
        let (controls, player_action) = coalesce_player_actions(events);

        for cmd in controls {
            match cmd {
                RawInput::Quit => return Ok(()),
                RawInput::DismissOverlay => {
                    if state.error_overlay.take().is_some() {
                        state.dirty = true;
                    }
                }
                RawInput::TogglePause => {
                    state.paused = !state.paused;
                    state.schedule_next_step_from(step_now);
                    state.dirty = true;
                }
                RawInput::StepOnce => {
                    state.paused = true;
                    state.next_step_at = None;
                    let intent = state.pending_tick_intent.take().into_iter().collect();
                    if let Err(err) = state.execute_step(intent, step_now) {
                        state.set_error(err.to_string());
                    }
                }
                RawInput::ResetSameSeed => state.reset_loaded_world(step_now, false),
                RawInput::ResetNewSeed => state.reset_loaded_world(step_now, true),
                RawInput::Action(_) => {}
            }
        }

        if let Some(action) = player_action {
            dispatch_player_action(state, action, step_now);
        }

        if let Some(deadline) = state.next_step_at
            && !state.paused
            && state.error_overlay.is_none()
            && step_now >= deadline
        {
            let intent = state.pending_tick_intent.take().into_iter().collect();
            if let Err(err) = state.execute_step(intent, step_now) {
                state.set_error(err.to_string());
            }
        }

        if let Err(err) = state.render_if_dirty(display, Instant::now()) {
            if err.kind() == ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(err);
        }
    }
}

/// Parses arguments, prepares the terminal grid, runs the simulation, and restores shell state.
///
/// # Errors
///
/// Returns a [LuaError] if command-line parsing fails, if terminal initialization fails, or if a
/// non-interactive smoke run fails to load or step its script.
fn main() -> LuaResult<()> {
    let options = parse_cli_args().map_err(LuaError::external)?;
    let _guard = TerminalGuard::enter().map_err(LuaError::external)?;

    let mut display = Display::new(FRAME_WIDTH, FRAME_HEIGHT);
    if let Err(err) = display.reserve_space() {
        if err.kind() == ErrorKind::BrokenPipe {
            return Ok(());
        }
        return Err(LuaError::external(err));
    }

    let mut state = EngineState::new(options.script_path);
    let result = match options.max_steps {
        Some(steps) => run_smoke_steps(&mut state, &mut display, steps),
        None => run_interactive(&mut state, &mut display).map_err(|err| {
            if err.kind() == ErrorKind::BrokenPipe {
                return LuaError::external("broken pipe");
            }
            LuaError::external(err)
        }),
    };

    drop(_guard);
    println!();
    result
}

#[cfg(test)]
mod tests {
    //! Unit tests verifying CLI option parsing and batched input coalescing.

    use super::*;

    #[test]
    fn coalesces_repeated_directional_keys_to_latest_move() {
        let events = vec![
            RawInput::Action(Action::Move { dx: 1, dy: 0 }),
            RawInput::Action(Action::Move { dx: 1, dy: 0 }),
            RawInput::Action(Action::Move { dx: 0, dy: -1 }),
            RawInput::TogglePause,
        ];
        let (controls, action) = coalesce_player_actions(events);
        assert_eq!(controls, vec![RawInput::TogglePause]);
        assert_eq!(action, Some(Action::Move { dx: 0, dy: -1 }));
    }
}
