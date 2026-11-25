//! Interactive terminal game and simulation engine driven by hot-reloaded Lua world scripts.
//!
//! Launches a dynamically sized character-grid viewport around an embedded Lua world controller
//! backed by a hand-rolled entity-component store, polls raw keyboard input without blocking timed
//! updates, presents frames inside an alternate screen buffer with live resize and entity
//! inspection, and atomically reloads modified scripts back to time zero without losing terminal
//! stability on mid-edit syntax errors.

mod ecs;
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

use crate::ecs::EcsWorld;
use crate::lua_host::{FileStampWatcher, LuaWorld};
use crate::model::{
    Action, Camera, CellWidth, DEFAULT_FRAME_WIDTH, FrameLayout, Intent, IntentTarget, MessageLog,
    Pacing, RepeatCadence, TileStyle,
};
use crate::render::FrameState;
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
    /// Authoritative entity-component store shared across simulation steps, rendering, and inspection.
    ecs: EcsWorld,
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
    /// Active screen and viewport geometry derived from terminal dimensions and inspector state.
    layout: FrameLayout,
    /// Whether the live entity-component inspector view is currently displayed.
    inspector_open: bool,
    /// Zero-based index into live entities currently highlighted by the inspector.
    inspector_selected: usize,
    /// Non-fatal script error shown inside an overlay while keeping the last valid state frozen.
    error_overlay: Option<String>,
    /// Last terminal dimensions seen when smaller than the playable minimum, for redraw detection.
    last_too_small: Option<(u16, u16)>,
    /// Marks that the full screen must be wiped before presenting after a geometry change.
    needs_clear: bool,
    /// Flag indicating that the frame buffer needs to be re-composed and printed.
    dirty: bool,
}

impl EngineState {
    /// Creates an initial unpopulated engine state ready to load a world script.
    fn new(script_path: PathBuf, layout: FrameLayout) -> Self {
        let watcher = FileStampWatcher::new(&script_path);
        Self {
            script_path,
            world: None,
            last_declared_seed: None,
            watcher,
            ecs: EcsWorld::new(),
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
            terrain: default_blank_terrain(layout),
            layout,
            inspector_open: false,
            inspector_selected: 0,
            error_overlay: None,
            last_too_small: None,
            needs_clear: true,
            dirty: true,
        }
    }

    /// Compiles and initializes the script in an isolated candidate virtual machine before swapping.
    ///
    /// Preserves the previous world, entities, layout, and terrain intact if compilation,
    /// initialization, or the initial viewport query fails.
    fn reload_from_disk(
        &mut self,
        display: &mut Display,
        is_tty: bool,
        now: Instant,
        fresh_seed: bool,
        announce_reload: bool,
    ) -> LuaResult<()> {
        let candidate = LuaWorld::load_from_path(&self.script_path)?;
        let declared = candidate.config().declared_seed;
        let cell_width = candidate.config().cell_width;

        let next_seed = if fresh_seed {
            roll_random_seed()
        } else if declared.is_some() && (self.world.is_none() || declared != self.last_declared_seed)
        {
            declared.unwrap_or(self.seed)
        } else {
            self.seed
        };

        let term_size = if is_tty { tty::terminal_size() } else { None };
        let target_layout = FrameLayout::compute(term_size, cell_width, self.inspector_open)
            .unwrap_or_else(|_| FrameLayout::default_for(cell_width));

        let (ecs, hud, init_prints) = candidate.init(next_seed)?;
        let mut camera = Camera::default();
        if let Some((_, pos)) = ecs.player() {
            camera.snap_to(pos);
        }

        let bounds = camera.bounds(target_layout.viewport_cols, target_layout.viewport_rows);
        let (terrain, view_prints) = candidate.view(bounds, 0)?;

        self.world = Some(candidate);
        self.last_declared_seed = declared;
        self.seed = next_seed;
        self.ecs = ecs;
        self.clamp_inspector_selection();
        self.camera = camera;
        self.terrain = terrain;
        if target_layout != self.layout {
            self.layout = target_layout;
            *display = Display::new(target_layout.frame_width, target_layout.frame_height);
            self.needs_clear = true;
        }
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
    fn reset_loaded_world(
        &mut self,
        display: &mut Display,
        is_tty: bool,
        now: Instant,
        fresh_seed: bool,
    ) {
        if fresh_seed {
            self.seed = roll_random_seed();
        }
        let Some(world) = self.world.as_ref() else {
            if let Err(err) = self.reload_from_disk(display, is_tty, now, fresh_seed, true) {
                self.set_error(err.to_string());
            }
            return;
        };

        let init_outcome = world.init(self.seed).and_then(|(ecs, hud, init_prints)| {
            let mut camera = Camera::default();
            if let Some((_, pos)) = ecs.player() {
                camera.snap_to(pos);
            }
            let bounds = camera.bounds(self.layout.viewport_cols, self.layout.viewport_rows);
            let (terrain, view_prints) = world.view(bounds, 0)?;
            Ok((ecs, camera, terrain, hud, init_prints, view_prints))
        });

        match init_outcome {
            Ok((ecs, camera, terrain, hud, init_prints, view_prints)) => {
                self.ecs = ecs;
                self.clamp_inspector_selection();
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

    /// Executes one simulation step transactionally and refreshes terrain for the active camera.
    fn execute_step(&mut self, intents: Vec<Intent>, now: Instant) -> LuaResult<()> {
        let Some(world) = self.world.as_ref() else {
            return Ok(());
        };

        let next_tick = self.tick.saturating_add(1);
        let (next_ecs, hud, step_prints) = world.step(next_tick, &intents, &self.ecs)?;

        self.tick = next_tick;
        self.ecs = next_ecs;
        clamp_index(self.ecs.len(), &mut self.inspector_selected);

        if let Some((_, pos)) = self.ecs.player() {
            self.camera.follow_deadzone(
                pos,
                self.layout.viewport_cols,
                self.layout.viewport_rows,
            );
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
            .bounds(self.layout.viewport_cols, self.layout.viewport_rows);
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
            self.terrain = default_blank_terrain(self.layout);
            return;
        };
        let bounds = self
            .camera
            .bounds(self.layout.viewport_cols, self.layout.viewport_rows);
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

    /// Keeps the inspector selection index inside the current live entity range.
    fn clamp_inspector_selection(&mut self) {
        clamp_index(self.ecs.len(), &mut self.inspector_selected);
    }

    /// Recomputes display dimensions and viewport bounds when terminal size or drawer state changes.
    fn sync_layout(&mut self, display: &mut Display, is_tty: bool) {
        let cell_width = self
            .world
            .as_ref()
            .map_or(CellWidth::Two, |w| w.config().cell_width);
        let term_size = if is_tty { tty::terminal_size() } else { None };

        match FrameLayout::compute(term_size, cell_width, self.inspector_open) {
            Ok(new_layout) => {
                if self.last_too_small.take().is_some() {
                    self.needs_clear = true;
                    self.dirty = true;
                }
                if new_layout != self.layout {
                    self.layout = new_layout;
                    *display = Display::new(new_layout.frame_width, new_layout.frame_height);
                    self.needs_clear = true;
                    if let Some((_, pos)) = self.ecs.player() {
                        self.camera.follow_deadzone(
                            pos,
                            new_layout.viewport_cols,
                            new_layout.viewport_rows,
                        );
                    }
                    self.refresh_view_only();
                    self.dirty = true;
                }
            }
            Err((cols, rows)) => {
                if self.last_too_small != Some((cols, rows)) {
                    self.last_too_small = Some((cols, rows));
                    let w = (cols.max(12) as u32).min(DEFAULT_FRAME_WIDTH);
                    let h = rows.saturating_sub(1).clamp(5, 10) as u32;
                    *display = Display::new(w, h);
                    self.needs_clear = true;
                    self.dirty = true;
                }
            }
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

    /// Syncs layout, paints the frame buffer, and flushes output when dirty.
    fn render_if_dirty(
        &mut self,
        display: &mut Display,
        is_tty: bool,
        now: Instant,
    ) -> io::Result<()> {
        self.sync_layout(display, is_tty);

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
            .bounds(self.layout.viewport_cols, self.layout.viewport_rows);

        let frame = FrameState {
            title,
            pacing,
            cell_width,
            layout: self.layout,
            tick: self.tick,
            seed: self.seed,
            paused: self.paused,
            reload_flash,
            bounds,
            terrain: &self.terrain,
            ecs: &self.ecs,
            palette,
            camera_center: self.camera.center,
            hud_status: &self.hud_status,
            log: &self.log,
            inspector_open: self.inspector_open,
            inspector_selected: self.inspector_selected,
            error_overlay: self.error_overlay.as_deref(),
            terminal_too_small: self.last_too_small,
        };

        render::compose_frame(display, &frame);
        let clear = std::mem::take(&mut self.needs_clear);
        tty::present_frame(display, is_tty, clear)?;
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

    if let Some((player_id, player_pos)) = state.ecs.player() {
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
            target: IntentTarget::Entity(player_id),
            action,
            origin: player_pos,
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
                target: IntentTarget::Cursor,
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

/// Clamps a selection cursor so it remains within the live element count.
fn clamp_index(count: usize, selected: &mut usize) {
    if count == 0 {
        *selected = 0;
    } else if *selected >= count {
        *selected = count - 1;
    }
}

/// Generates a compact random seed from monotonic clock nanoseconds.
fn roll_random_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(42, |d| d.as_nanos());
    ((nanos ^ (nanos >> 16)) & 0xffff) as u64
}

/// Creates a blank black terrain grid matching the active layout dimensions.
fn default_blank_terrain(layout: FrameLayout) -> Vec<Vec<TileStyle>> {
    let style = TileStyle::from_spec(" ", Colour::White, Some(Colour::Black));
    vec![vec![style; layout.viewport_cols]; layout.viewport_rows]
}

/// Runs fixed-step non-interactive smoke frames and exits cleanly.
fn run_smoke_steps(state: &mut EngineState, display: &mut Display, steps: u64) -> LuaResult<()> {
    let now = Instant::now();
    state.reload_from_disk(display, false, now, false, false)?;
    if let Err(err) = state.render_if_dirty(display, false, now)
        && err.kind() != ErrorKind::BrokenPipe
    {
        return Err(LuaError::external(err));
    }

    for _ in 0..steps {
        let step_now = Instant::now();
        state.execute_step(Vec::new(), step_now)?;
        if let Err(err) = state.render_if_dirty(display, false, step_now) {
            if err.kind() == ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(LuaError::external(err));
        }
    }

    Ok(())
}

/// Runs the interactive raw-mode event loop until the user quits or closes input.
fn run_interactive(state: &mut EngineState, display: &mut Display, is_tty: bool) -> io::Result<()> {
    let start = Instant::now();
    if let Err(err) = state.reload_from_disk(display, is_tty, start, false, false) {
        state.set_error(err.to_string());
    }
    state.render_if_dirty(display, is_tty, start)?;

    loop {
        let now = Instant::now();

        if tty::take_resize_pending() {
            state.dirty = true;
        }

        if let Some(flash_end) = state.reload_flash_until
            && now >= flash_end
        {
            state.reload_flash_until = None;
            state.dirty = true;
        }

        if state.watcher.poll_changed(now)
            && let Err(err) = state.reload_from_disk(display, is_tty, now, false, true)
        {
            state.set_error(err.to_string());
        }

        let timeout_ms = state.poll_timeout_ms(now);
        let events = tty::poll_and_drain_inputs(timeout_ms)?;
        if tty::take_resize_pending() {
            state.dirty = true;
        }

        let step_now = Instant::now();
        let (controls, player_action) = coalesce_player_actions(events);

        for cmd in controls {
            match cmd {
                RawInput::Quit => return Ok(()),
                RawInput::DismissOverlay => {
                    if state.error_overlay.take().is_some() {
                        state.dirty = true;
                    } else if state.inspector_open {
                        state.inspector_open = false;
                        state.dirty = true;
                    }
                }
                RawInput::ToggleInspector => {
                    state.inspector_open = !state.inspector_open;
                    state.clamp_inspector_selection();
                    state.dirty = true;
                }
                RawInput::InspectorPrev => {
                    if state.inspector_open && !state.ecs.is_empty() {
                        let count = state.ecs.len();
                        state.inspector_selected = (state.inspector_selected + count - 1) % count;
                        state.dirty = true;
                    }
                }
                RawInput::InspectorNext => {
                    if state.inspector_open && !state.ecs.is_empty() {
                        let count = state.ecs.len();
                        state.inspector_selected = (state.inspector_selected + 1) % count;
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
                RawInput::ResetSameSeed => state.reset_loaded_world(display, is_tty, step_now, false),
                RawInput::ResetNewSeed => state.reset_loaded_world(display, is_tty, step_now, true),
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

        if let Err(err) = state.render_if_dirty(display, is_tty, Instant::now()) {
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
    let guard = TerminalGuard::enter().map_err(LuaError::external)?;
    let is_tty = guard.is_interactive_tty();

    let term_size = if is_tty { tty::terminal_size() } else { None };
    let initial_layout = FrameLayout::compute(term_size, CellWidth::Two, false)
        .unwrap_or_else(|_| FrameLayout::default_for(CellWidth::Two));
    let mut display = Display::new(initial_layout.frame_width, initial_layout.frame_height);

    if !is_tty
        && let Err(err) = display.reserve_space()
    {
        if err.kind() == ErrorKind::BrokenPipe {
            return Ok(());
        }
        return Err(LuaError::external(err));
    }

    let mut state = EngineState::new(options.script_path, initial_layout);
    let result = match options.max_steps {
        Some(steps) => run_smoke_steps(&mut state, &mut display, steps),
        None => run_interactive(&mut state, &mut display, is_tty).map_err(|err| {
            if err.kind() == ErrorKind::BrokenPipe {
                return LuaError::external("broken pipe");
            }
            LuaError::external(err)
        }),
    };

    drop(guard);
    if !is_tty {
        println!();
    }
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
