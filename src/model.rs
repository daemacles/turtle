//! Core world geometry, layout calculation, camera tracking, and input intent types.
//!
//! Defines the pure-Rust data model shared across the entity world, Lua host, terminal input layer,
//! frame renderer, and simulation loop without depending on terminal system calls or the Lua VM.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use terminal_graphics::Colour;

use crate::ecs::EntityId;

/// Default character columns of the rendered frame when terminal dimensions are unavailable.
pub const DEFAULT_FRAME_WIDTH: u32 = 78;

/// Default character rows of the rendered frame when terminal dimensions are unavailable.
pub const DEFAULT_FRAME_HEIGHT: u32 = 23;

/// Number of logical tile rows visible in the default viewport height.
pub const DEFAULT_VIEWPORT_ROWS: usize = 17;

/// Number of logical tile columns visible at single-column tile width in the default viewport.
pub const DEFAULT_VIEWPORT_COLS_ONE: usize = 75;

/// Number of logical tile columns visible at two-column tile width in the default viewport.
pub const DEFAULT_VIEWPORT_COLS_TWO: usize = 37;

/// Minimum terminal columns required to render a playable map viewport without layout collapse.
pub const MIN_TERMINAL_COLS: u16 = 52;

/// Minimum terminal rows required to render a playable map viewport and status footer.
pub const MIN_TERMINAL_ROWS: u16 = 14;

/// Upper bound on terminal frame width so huge panes do not request excessive Lua grid rows.
pub const MAX_FRAME_WIDTH: u16 = 160;

/// Upper bound on terminal frame height so huge panes do not request excessive Lua grid rows.
pub const MAX_FRAME_HEIGHT: u16 = 55;

/// Minimum terminal columns required to dock the inspector drawer beside the map viewport.
pub const INSPECTOR_DRAWER_MIN_COLS: u16 = 106;

/// Character width reserved for the side-by-side entity inspector drawer when docked.
pub const INSPECTOR_DRAWER_WIDTH: usize = 32;

/// Number of terminal rows occupied by frame borders, divider, event log, and controls legend.
pub const CHROME_ROWS: usize = 6;

/// Minimum milliseconds between repeated movement steps in the same direction.
pub const SAME_DIR_REPEAT_MS: u64 = 80;

/// Minimum milliseconds between consecutive movement steps after changing direction.
pub const TURN_CORNER_REPEAT_MS: u64 = 25;

/// Default idle delay in milliseconds before hybrid pacing fires an ambient step.
pub const DEFAULT_IDLE_MS: u64 = 650;

/// Maximum number of recent status messages retained by the event log.
const MAX_LOG_LINES: usize = 16;

/// Signed integer coordinates in the two-dimensional world grid.
///
/// Positive horizontal coordinates point east and positive vertical coordinates point south,
/// centered around the origin without restricting worlds to non-negative quadrants.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Coord {
    /// Horizontal world coordinate growing eastward.
    pub x: i32,
    /// Vertical world coordinate growing southward.
    pub y: i32,
}

impl Coord {
    /// Creates a world coordinate from signed horizontal and vertical offsets.
    ///
    /// - `x`: horizontal grid position growing eastward.
    /// - `y`: vertical grid position growing southward.
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// Inclusive rectangular region in world coordinates used for viewport queries.
///
/// Both minimum and maximum corners belong to the covered region so odd viewport dimensions keep a
/// single unambiguous center coordinate.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RectBounds {
    /// Top-left inclusive corner in world coordinates.
    pub min: Coord,
    /// Bottom-right inclusive corner in world coordinates.
    pub max: Coord,
}

impl RectBounds {
    /// Creates an inclusive bounding rectangle from minimum and maximum corners.
    ///
    /// - `min`: top-left inclusive world coordinate.
    /// - `max`: bottom-right inclusive world coordinate.
    pub const fn new(min: Coord, max: Coord) -> Self {
        Self { min, max }
    }

    /// Returns the number of tile columns spanned by this inclusive rectangle.
    ///
    /// Returns zero when the maximum horizontal coordinate lies west of the minimum.
    pub fn width(&self) -> usize {
        if self.max.x < self.min.x {
            0
        } else {
            (self.max.x as i64 - self.min.x as i64 + 1) as usize
        }
    }

    /// Returns the number of tile rows spanned by this inclusive rectangle.
    ///
    /// Returns zero when the maximum vertical coordinate lies north of the minimum.
    pub fn height(&self) -> usize {
        if self.max.y < self.min.y {
            0
        } else {
            (self.max.y as i64 - self.min.y as i64 + 1) as usize
        }
    }

    /// Reports whether a world position falls inside this inclusive bounding box.
    ///
    /// - `pos`: world coordinate to test against both horizontal and vertical bounds.
    pub fn contains(&self, pos: Coord) -> bool {
        pos.x >= self.min.x && pos.x <= self.max.x && pos.y >= self.min.y && pos.y <= self.max.y
    }
}

/// Simulation clock mode declared by the loaded world script.
///
/// Controls how user inputs and elapsed time drive simulation steps in the main loop.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Pacing {
    /// Advances only when the player issues an action.
    Turn,
    /// Advances on a fixed interval regardless of whether input arrived.
    Tick {
        /// Milliseconds between consecutive simulation steps.
        tick_ms: u64,
    },
    /// Advances immediately on player input and also fires ambient steps when idle.
    Hybrid {
        /// Milliseconds of player inactivity before firing an ambient step.
        idle_ms: u64,
    },
}

impl Default for Pacing {
    fn default() -> Self {
        Self::Hybrid {
            idle_ms: DEFAULT_IDLE_MS,
        }
    }
}

impl Pacing {
    /// Returns the short uppercase tag displayed in the top frame border.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Turn => "TURN",
            Self::Tick { .. } => "TICK",
            Self::Hybrid { .. } => "HYBRID",
        }
    }
}

/// Horizontal terminal column width assigned to each logical world tile.
///
/// Using two columns compensates for roughly two-to-one terminal character height so square rooms
/// appear square on screen, while single-column mode packs dense grids into the viewport.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum CellWidth {
    /// Renders each tile as a single terminal column.
    One,
    /// Renders each tile as two adjacent terminal columns.
    #[default]
    Two,
}

impl CellWidth {
    /// Returns the terminal column multiplier per logical world tile.
    pub const fn columns(self) -> usize {
        match self {
            Self::One => 1,
            Self::Two => 2,
        }
    }

    /// Returns the default viewport column count used when no terminal window size is present.
    pub const fn default_viewport_cols(self) -> usize {
        match self {
            Self::One => DEFAULT_VIEWPORT_COLS_ONE,
            Self::Two => DEFAULT_VIEWPORT_COLS_TWO,
        }
    }
}

/// Computed screen and logical viewport geometry derived from terminal dimensions and inspector mode.
///
/// Ensures both viewport tile counts remain odd so camera bounds stay symmetric around the center
/// coordinate while reserving margin columns and rows to prevent terminal line-wrap scrolling.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FrameLayout {
    /// Total character columns allocated to the display buffer.
    pub frame_width: u32,
    /// Total character rows allocated to the display buffer.
    pub frame_height: u32,
    /// Logical world tile columns requested from the world view hook.
    pub viewport_cols: usize,
    /// Logical world tile rows requested from the world view hook.
    pub viewport_rows: usize,
    /// Width of the docked side-by-side inspector panel, or zero when closed or modal.
    pub inspector_drawer_width: usize,
}

impl FrameLayout {
    /// Computes frame and viewport geometry for optional terminal dimensions and inspector state.
    ///
    /// Falls back to standard default dimensions when no terminal dimensions are supplied. Returns
    /// an error holding the undersized dimensions when the terminal is too small to draw cleanly.
    ///
    /// - `term_size`: detected terminal columns and rows, or none for non-terminal execution.
    /// - `cell_width`: terminal columns occupied by each logical tile.
    /// - `inspector_open`: whether the entity inspector is currently toggled visible.
    ///
    /// # Errors
    ///
    /// Returns the reported column and row pair when the terminal pane is smaller than the minimum
    /// playable size.
    pub fn compute(
        term_size: Option<(u16, u16)>,
        cell_width: CellWidth,
        inspector_open: bool,
    ) -> Result<Self, (u16, u16)> {
        let Some((cols, rows)) = term_size else {
            return Ok(Self::default_for(cell_width));
        };

        if cols < MIN_TERMINAL_COLS || rows < MIN_TERMINAL_ROWS {
            return Err((cols, rows));
        }

        // Leave one spare terminal column and bottom row so edge writes never auto-wrap or scroll.
        let avail_cols = (cols.saturating_sub(1)).min(MAX_FRAME_WIDTH) as usize;
        let avail_rows = (rows.saturating_sub(1)).min(MAX_FRAME_HEIGHT) as usize;

        let inspector_drawer_width = if inspector_open && cols >= INSPECTOR_DRAWER_MIN_COLS {
            INSPECTOR_DRAWER_WIDTH
        } else {
            0
        };

        let raw_rows = avail_rows.saturating_sub(CHROME_ROWS).max(1);
        let viewport_rows = force_odd_at_least_one(raw_rows);

        let map_budget = avail_cols.saturating_sub(inspector_drawer_width);
        let tile_cols_budget = map_budget.saturating_sub(4) / cell_width.columns();
        let viewport_cols = force_odd_at_least_one(tile_cols_budget.max(1));

        let map_width = viewport_cols * cell_width.columns() + 4;
        let frame_width = (map_width + inspector_drawer_width) as u32;
        let frame_height = (viewport_rows + CHROME_ROWS) as u32;

        Ok(Self {
            frame_width,
            frame_height,
            viewport_cols,
            viewport_rows,
            inspector_drawer_width,
        })
    }

    /// Returns the fallback layout matching a standard eighty-by-twenty-four terminal window.
    ///
    /// - `cell_width`: terminal columns occupied by each logical tile.
    pub const fn default_for(cell_width: CellWidth) -> Self {
        let viewport_cols = cell_width.default_viewport_cols();
        let frame_width = match cell_width {
            CellWidth::One => (viewport_cols + 4) as u32,
            CellWidth::Two => DEFAULT_FRAME_WIDTH,
        };
        Self {
            frame_width,
            frame_height: DEFAULT_FRAME_HEIGHT,
            viewport_cols,
            viewport_rows: DEFAULT_VIEWPORT_ROWS,
            inspector_drawer_width: 0,
        }
    }

    /// Returns the total character columns belonging to the map panel including its borders.
    pub const fn map_width(&self) -> usize {
        self.frame_width as usize - self.inspector_drawer_width
    }

    /// Returns the usable inner text columns inside the map panel borders.
    pub const fn map_inner_width(&self) -> usize {
        self.map_width().saturating_sub(4)
    }
}

/// Rounds an even count down by one so viewport extents remain odd and camera-centered.
const fn force_odd_at_least_one(n: usize) -> usize {
    if n <= 1 {
        1
    } else if n.is_multiple_of(2) {
        n - 1
    } else {
        n
    }
}

/// Resolved visual appearance for one terrain tile or entity sprite.
///
/// Stores both a two-character micro-sprite and a single-character glyph so rendering works cleanly
/// in either tile width mode. A background of `None` marks the sprite transparent over terrain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileStyle {
    /// Character pair rendered left to right when tile width is two columns.
    pub glyph: [char; 2],
    /// Single character rendered when tile width is one column.
    pub single: char,
    /// Foreground color applied to the tile characters.
    pub fg: Colour,
    /// Optional cell background color, or transparent inheritance when absent.
    pub bg: Option<Colour>,
}

impl TileStyle {
    /// Builds a tile style from a glyph string and ANSI foreground and background colors.
    ///
    /// Single-character strings pad their second wide column with a space; empty strings render as
    /// blanks. Characters beyond the second Unicode scalar are ignored.
    ///
    /// - `glyph_str`: one or two display characters representing the tile.
    /// - `fg`: foreground color applied to drawn characters.
    /// - `bg`: explicit background color, or `None` for transparent overlay compositing.
    pub fn from_spec(glyph_str: &str, fg: Colour, bg: Option<Colour>) -> Self {
        let mut chars = glyph_str.chars();
        let first = chars.next().unwrap_or(' ');
        let second = chars.next().unwrap_or(' ');
        Self {
            glyph: [first, second],
            single: first,
            fg,
            bg,
        }
    }
}

/// World-level configuration loaded from the Lua script header fields.
#[derive(Clone, Debug)]
pub struct WorldConfig {
    /// Human-readable world name shown in the top status bar.
    pub title: String,
    /// Terminal columns occupied by each logical tile.
    pub cell_width: CellWidth,
    /// Clock behavior governing when simulation steps occur.
    pub pacing: Pacing,
    /// Optional fixed seed declared inside the script, overriding automatic seeding.
    pub declared_seed: Option<u64>,
}

/// Player-level action dispatched into the simulation step or camera controller.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Directional movement or camera pan delta in world tiles.
    Move {
        /// East-west step delta in tiles.
        dx: i32,
        /// North-south step delta in tiles.
        dy: i32,
    },
    /// Explicit wait or pass turn in place.
    Wait,
    /// Context-sensitive interaction at the actor or cursor location.
    Act,
}

/// Identifies whether an input intent applies to a live entity or the virtual camera reticle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IntentTarget {
    /// Action targeting a specific generational entity handle.
    Entity(EntityId),
    /// Action targeting the center reticle in a playerless world.
    Cursor,
}

/// Per-entity or cursor intent submitted by the host into the Lua world step function.
///
/// Separating intents from entity storage lets both local keyboard actions and future network or
/// mouse commands feed one uniform request queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    /// Target entity handle or virtual cursor receiving the action.
    pub target: IntentTarget,
    /// Action requested for this step.
    pub action: Action,
    /// World position captured at the time the intent was built.
    pub origin: Coord,
}

impl Intent {
    /// Returns the protocol string name expected by Lua for this intent's action.
    pub const fn action_name(&self) -> &'static str {
        match self.action {
            Action::Move { .. } => "move",
            Action::Wait => "wait",
            Action::Act => "act",
        }
    }

    /// Returns the horizontal and vertical step offset implied by this intent.
    pub const fn delta(&self) -> (i32, i32) {
        match self.action {
            Action::Move { dx, dy } => (dx, dy),
            Action::Wait | Action::Act => (0, 0),
        }
    }
}

/// Rate limiter that tames terminal OS key-repeat bursts without delaying deliberate taps.
///
/// Tracks the timestamp and direction of the most recent accepted move on the main thread. Single
/// taps always pass immediately, while held key-repeats throttle to distinct same-direction and
/// corner-turn cadences.
#[derive(Debug, Default)]
pub struct RepeatCadence {
    /// Last accepted directional vector and the timestamp it fired.
    last_move: Option<((i32, i32), Instant)>,
}

impl RepeatCadence {
    /// Creates an unprimed cadence limiter ready to accept the next move immediately.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears recorded timing state so the next movement input fires immediately.
    pub fn reset(&mut self) {
        self.last_move = None;
    }

    /// Decides whether a directional move may proceed at the given timestamp.
    ///
    /// Applies [SAME_DIR_REPEAT_MS] when repeating the previous direction and
    /// [TURN_CORNER_REPEAT_MS] when switching to a new direction.
    ///
    /// - `dx`: horizontal step delta.
    /// - `dy`: vertical step delta.
    /// - `now`: monotonic timestamp when the input is evaluated.
    pub fn allow_move(&mut self, dx: i32, dy: i32, now: Instant) -> bool {
        let dir = (dx, dy);
        if let Some((prev_dir, prev_at)) = self.last_move {
            let min_gap = if prev_dir == dir {
                Duration::from_millis(SAME_DIR_REPEAT_MS)
            } else {
                Duration::from_millis(TURN_CORNER_REPEAT_MS)
            };
            if now.duration_since(prev_at) < min_gap {
                return false;
            }
        }
        self.last_move = Some((dir, now));
        true
    }
}

/// Viewport camera tracking either the player deadzone or manual panning.
///
/// Stores the world coordinate located at the center of the visible tile grid so odd viewport
/// dimensions frame the center coordinate symmetrically.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Camera {
    /// World coordinate shown at the center cell of the viewport.
    pub center: Coord,
}

impl Camera {
    /// Centers the viewport directly onto the target world coordinate.
    ///
    /// - `target`: world coordinate to place at the viewport center.
    pub fn snap_to(&mut self, target: Coord) {
        self.center = target;
    }

    /// Shifts the camera only if the target coordinate leaves the central deadzone box.
    ///
    /// The deadzone spans roughly half the viewport width and height around `center`, eliminating
    /// background shimmer during small movements inside the inner region.
    ///
    /// - `target`: player or focal world coordinate to keep comfortably inside the frame.
    /// - `view_cols`: logical tile columns across the viewport.
    /// - `view_rows`: logical tile rows across the viewport.
    pub fn follow_deadzone(&mut self, target: Coord, view_cols: usize, view_rows: usize) {
        let dz_x = (view_cols as i32) / 4;
        let dz_y = (view_rows as i32) / 4;

        let dx = target.x.saturating_sub(self.center.x);
        if dx > dz_x {
            self.center.x = self.center.x.saturating_add(dx - dz_x);
        } else if dx < -dz_x {
            self.center.x = self.center.x.saturating_add(dx + dz_x);
        }

        let dy = target.y.saturating_sub(self.center.y);
        if dy > dz_y {
            self.center.y = self.center.y.saturating_add(dy - dz_y);
        } else if dy < -dz_y {
            self.center.y = self.center.y.saturating_add(dy + dz_y);
        }
    }

    /// Moves the viewport center directly by a signed tile offset.
    ///
    /// - `dx`: eastward tile delta.
    /// - `dy`: southward tile delta.
    pub fn pan(&mut self, dx: i32, dy: i32) {
        self.center.x = self.center.x.saturating_add(dx);
        self.center.y = self.center.y.saturating_add(dy);
    }

    /// Computes inclusive world bounds covered by a viewport of the given logical dimensions.
    ///
    /// - `view_cols`: logical tile columns visible horizontally.
    /// - `view_rows`: logical tile rows visible vertically.
    pub fn bounds(&self, view_cols: usize, view_rows: usize) -> RectBounds {
        let cols = view_cols.max(1) as i32;
        let rows = view_rows.max(1) as i32;
        let min = Coord::new(
            self.center.x.saturating_sub(cols / 2),
            self.center.y.saturating_sub(rows / 2),
        );
        let max = Coord::new(
            min.x.saturating_add(cols - 1),
            min.y.saturating_add(rows - 1),
        );
        RectBounds::new(min, max)
    }
}

/// Optional status header and event message emitted by Lua initialization or step hooks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HudUpdate {
    /// Short persistent summary text displayed in the top bar.
    pub status: Option<String>,
    /// Transient event line appended to the bottom message log.
    pub message: Option<String>,
}

/// Bounded rolling queue of recent event and print lines shown below the viewport.
///
/// Normalizes whitespace and keeps the newest lines up to a fixed cap so the rendered two-line log
/// region remains tidy across long sessions.
#[derive(Clone, Debug, Default)]
pub struct MessageLog {
    /// Recent lines ordered from oldest to newest.
    lines: VecDeque<String>,
}

impl MessageLog {
    /// Appends a non-empty sanitized message line, dropping the oldest line if full.
    ///
    /// Control characters such as tabs and line breaks collapse to spaces so single-row log slots
    /// stay aligned.
    ///
    /// - `line`: message text from Lua events or captured print calls.
    pub fn push_line(&mut self, line: impl AsRef<str>) {
        let clean: String = line
            .as_ref()
            .chars()
            .map(|ch| if ch == '\n' || ch == '\r' || ch == '\t' { ' ' } else { ch })
            .collect();
        let trimmed = clean.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.lines.len() == MAX_LOG_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(trimmed.to_string());
    }

    /// Removes all recorded messages from the log.
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// Returns the two most recent log lines ordered older first, newer second.
    pub fn recent_two(&self) -> (&str, &str) {
        let len = self.lines.len();
        match len {
            0 => ("", ""),
            1 => ("", self.lines.back().map_or("", String::as_str)),
            _ => {
                let older = self.lines.get(len - 2).map_or("", String::as_str);
                let newer = self.lines.get(len - 1).map_or("", String::as_str);
                (older, newer)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests verifying camera deadzones, frame layouts, cadence limiting, and tile styles.

    use super::*;

    #[test]
    fn bounds_center_odd_viewports_on_camera() {
        let cam = Camera {
            center: Coord::new(0, 0),
        };
        let b2 = cam.bounds(DEFAULT_VIEWPORT_COLS_TWO, DEFAULT_VIEWPORT_ROWS);
        assert_eq!(b2.min, Coord::new(-18, -8));
        assert_eq!(b2.max, Coord::new(18, 8));
        assert_eq!((b2.width(), b2.height()), (37, 17));

        let b1 = cam.bounds(DEFAULT_VIEWPORT_COLS_ONE, DEFAULT_VIEWPORT_ROWS);
        assert_eq!(b1.min, Coord::new(-37, -8));
        assert_eq!(b1.max, Coord::new(37, 8));
        assert_eq!((b1.width(), b1.height()), (75, 17));
    }

    #[test]
    fn camera_deadzone_holds_inside_inner_box_and_tracks_overshoot() {
        let mut cam = Camera::default();
        // Inside 37x17 deadzone (|dx| <= 9, |dy| <= 4) camera stays still.
        cam.follow_deadzone(
            Coord::new(9, -4),
            DEFAULT_VIEWPORT_COLS_TWO,
            DEFAULT_VIEWPORT_ROWS,
        );
        assert_eq!(cam.center, Coord::new(0, 0));

        // Stepping one cell past east and south boundaries shifts camera by 1 on each axis.
        cam.follow_deadzone(
            Coord::new(10, 5),
            DEFAULT_VIEWPORT_COLS_TWO,
            DEFAULT_VIEWPORT_ROWS,
        );
        assert_eq!(cam.center, Coord::new(1, 1));

        cam.pan(-3, 2);
        assert_eq!(cam.center, Coord::new(-2, 3));
    }

    #[test]
    fn frame_layout_adapts_odd_viewports_and_inspector_drawer_threshold() {
        let std_two = FrameLayout::compute(Some((80, 24)), CellWidth::Two, true).unwrap();
        assert_eq!(std_two.frame_width, DEFAULT_FRAME_WIDTH);
        assert_eq!(std_two.frame_height, DEFAULT_FRAME_HEIGHT);
        assert_eq!((std_two.viewport_cols, std_two.viewport_rows), (37, 17));
        assert_eq!(std_two.inspector_drawer_width, 0);

        let std_one = FrameLayout::compute(Some((80, 24)), CellWidth::One, false).unwrap();
        assert_eq!((std_one.viewport_cols, std_one.viewport_rows), (75, 17));
        assert_eq!(std_one.frame_width, 79);

        let wide_closed = FrameLayout::compute(Some((120, 34)), CellWidth::Two, false).unwrap();
        assert_eq!(wide_closed.inspector_drawer_width, 0);
        assert_eq!(wide_closed.viewport_cols % 2, 1);
        assert_eq!(wide_closed.viewport_rows % 2, 1);
        assert_eq!(wide_closed.viewport_rows, 27);

        let wide_open = FrameLayout::compute(Some((120, 34)), CellWidth::Two, true).unwrap();
        assert_eq!(wide_open.inspector_drawer_width, INSPECTOR_DRAWER_WIDTH);
        assert!(wide_open.frame_width <= 119);
        assert_eq!(wide_open.viewport_cols % 2, 1);

        assert_eq!(
            FrameLayout::compute(Some((51, 24)), CellWidth::Two, false),
            Err((51, 24))
        );
        assert_eq!(
            FrameLayout::compute(Some((80, 13)), CellWidth::Two, false),
            Err((80, 13))
        );
    }

    #[test]
    fn repeat_cadence_throttles_same_direction_more_than_corner_turns() {
        let mut cadence = RepeatCadence::new();
        let t0 = Instant::now();

        assert!(cadence.allow_move(1, 0, t0));
        assert!(!cadence.allow_move(1, 0, t0 + Duration::from_millis(40)));
        assert!(cadence.allow_move(0, -1, t0 + Duration::from_millis(30)));
        assert!(!cadence.allow_move(0, -1, t0 + Duration::from_millis(70)));
        assert!(cadence.allow_move(0, -1, t0 + Duration::from_millis(120)));
    }

    #[test]
    fn tile_style_and_message_log_handle_padding_and_truncation() {
        let one = TileStyle::from_spec("@", Colour::BrightYellow, None);
        assert_eq!(one.glyph, ['@', ' ']);
        assert_eq!(one.single, '@');

        let two = TileStyle::from_spec("≈~", Colour::BrightCyan, Some(Colour::Blue));
        assert_eq!(two.glyph, ['≈', '~']);
        assert_eq!(two.single, '≈');

        let mut log = MessageLog::default();
        assert_eq!(log.recent_two(), ("", ""));
        log.push_line("  \t\n ");
        assert_eq!(log.recent_two(), ("", ""));
        log.push_line("first\nline");
        assert_eq!(log.recent_two(), ("", "first line"));
        log.push_line("second");
        assert_eq!(log.recent_two(), ("first line", "second"));
    }
}
