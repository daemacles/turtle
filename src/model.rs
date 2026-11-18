//! Core world geometry, entity state, camera tracking, and input intent types.
//!
//! Defines the pure Rust data model shared across the Lua host, terminal input layer, frame
//! renderer, and main simulation loop without depending on terminal system calls or the Lua
//! virtual machine. Keeping entity ownership here creates a clean boundary for evolving the store
//! into an entity component architecture later.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use terminal_graphics::Colour;

/// Number of logical tile rows visible inside the world viewport.
pub const VIEWPORT_ROWS: usize = 17;

/// Number of logical tile columns visible when each tile occupies one terminal column.
pub const VIEWPORT_COLS_ONE: usize = 75;

/// Number of logical tile columns visible when each tile occupies two terminal columns.
pub const VIEWPORT_COLS_TWO: usize = 37;

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
    /// Returns the logical viewport column count that fits inside the fixed frame width.
    pub const fn viewport_cols(self) -> usize {
        match self {
            Self::One => VIEWPORT_COLS_ONE,
            Self::Two => VIEWPORT_COLS_TWO,
        }
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

/// Per-entity intent submitted by the host into the Lua world step function.
///
/// Separating intents from entity storage lets both local keyboard actions and future network or
/// mouse commands feed one uniform request queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    /// Identifier of the entity or virtual cursor issuing the action.
    pub entity_id: String,
    /// Action requested for this step.
    pub action: Action,
    /// Entity world position captured at the time the intent was built.
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

/// Authoritative Rust-owned record for one live actor or object in the world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    /// Unique identifier shared between Rust storage and Lua scripts.
    pub id: String,
    /// Current world coordinates.
    pub pos: Coord,
    /// Key into the loaded world palette used to draw this entity.
    pub tile: String,
    /// Whether this entity receives local player input and camera tracking.
    pub is_player: bool,
}

/// Incremental mutation returned by Lua to update, spawn, or remove an entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityPatch {
    /// Target entity identifier to update, insert, or remove.
    pub id: String,
    /// Replacement world position when moving or spawning an entity.
    pub pos: Option<Coord>,
    /// Replacement palette key when changing appearance.
    pub tile: Option<String>,
    /// Replacement player flag when changing camera or input ownership.
    pub is_player: Option<bool>,
    /// Marks the matching entity for removal from the entity store.
    pub remove: bool,
}

/// Ordered registry of all active world entities owned by Rust.
///
/// Preserves insertion order so entity overlay compositing remains deterministic across frames and
/// prepares cleanly for component-based storage in later versions.
#[derive(Clone, Debug, Default)]
pub struct EntityStore {
    /// Live entities ordered by creation time.
    entities: Vec<Entity>,
}

impl EntityStore {
    /// Builds a store from an initial entity list, promoting the default player identifier when unmarked.
    ///
    /// Duplicate identifiers collapse by updating the earlier entry in place so insertion order
    /// stays deterministic.
    ///
    /// - `list`: initial entities emitted by the script's initialization hook.
    pub fn from_entities(list: Vec<Entity>) -> Self {
        let mut store = Self::default();
        for entity in list {
            if let Some(existing) = store.entities.iter_mut().find(|e| e.id == entity.id) {
                *existing = entity;
            } else {
                store.entities.push(entity);
            }
        }

        if !store.entities.iter().any(|e| e.is_player)
            && let Some(player) = store.entities.iter_mut().find(|e| e.id == "player")
        {
            player.is_player = true;
        }

        store
    }

    /// Applies a batch of Lua-produced entity mutations in order.
    ///
    /// Removals filter out matching identifiers, updates patch existing records in place, and
    /// unseen identifiers with a position insert as new entities.
    ///
    /// - `patches`: delta updates emitted by one simulation step.
    pub fn apply_patches(&mut self, patches: &[EntityPatch]) {
        for patch in patches {
            if patch.remove {
                self.entities.retain(|e| e.id != patch.id);
                continue;
            }

            if let Some(existing) = self.entities.iter_mut().find(|e| e.id == patch.id) {
                if let Some(pos) = patch.pos {
                    existing.pos = pos;
                }
                if let Some(tile) = &patch.tile {
                    existing.tile = tile.clone();
                }
                if let Some(is_player) = patch.is_player {
                    existing.is_player = is_player;
                }
            } else if let Some(pos) = patch.pos {
                let is_player = patch.is_player.unwrap_or_else(|| patch.id == "player");
                let tile = patch.tile.clone().unwrap_or_else(|| "?".to_string());
                self.entities.push(Entity {
                    id: patch.id.clone(),
                    pos,
                    tile,
                    is_player,
                });
            }
        }
    }

    /// Returns the active player entity if one exists in the store.
    pub fn player(&self) -> Option<&Entity> {
        self.entities.iter().find(|e| e.is_player)
    }

    /// Iterates over all live entities in deterministic insertion order.
    pub fn iter(&self) -> std::slice::Iter<'_, Entity> {
        self.entities.iter()
    }

    /// Returns the total number of live entities currently stored.
    pub fn len(&self) -> usize {
        self.entities.len()
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
    //! Unit tests verifying camera deadzones, entity registry patches, cadence limiting, and tile styles.

    use super::*;

    #[test]
    fn bounds_center_odd_viewports_on_camera() {
        let cam = Camera {
            center: Coord::new(0, 0),
        };
        let b2 = cam.bounds(VIEWPORT_COLS_TWO, VIEWPORT_ROWS);
        assert_eq!(b2.min, Coord::new(-18, -8));
        assert_eq!(b2.max, Coord::new(18, 8));
        assert_eq!((b2.width(), b2.height()), (37, 17));

        let b1 = cam.bounds(VIEWPORT_COLS_ONE, VIEWPORT_ROWS);
        assert_eq!(b1.min, Coord::new(-37, -8));
        assert_eq!(b1.max, Coord::new(37, 8));
        assert_eq!((b1.width(), b1.height()), (75, 17));
    }

    #[test]
    fn camera_deadzone_holds_inside_inner_box_and_tracks_overshoot() {
        let mut cam = Camera::default();
        // Inside 37x17 deadzone (|dx| <= 9, |dy| <= 4) camera stays still.
        cam.follow_deadzone(Coord::new(9, -4), VIEWPORT_COLS_TWO, VIEWPORT_ROWS);
        assert_eq!(cam.center, Coord::new(0, 0));

        // Stepping one cell past east and south boundaries shifts camera by 1 on each axis.
        cam.follow_deadzone(Coord::new(10, 5), VIEWPORT_COLS_TWO, VIEWPORT_ROWS);
        assert_eq!(cam.center, Coord::new(1, 1));

        cam.pan(-3, 2);
        assert_eq!(cam.center, Coord::new(-2, 3));
    }

    #[test]
    fn entity_store_promotes_player_and_applies_move_spawn_remove_patches() {
        let mut store = EntityStore::from_entities(vec![
            Entity {
                id: "crate1".into(),
                pos: Coord::new(2, 0),
                tile: "B".into(),
                is_player: false,
            },
            Entity {
                id: "player".into(),
                pos: Coord::new(0, 0),
                tile: "@".into(),
                is_player: false,
            },
        ]);

        assert_eq!(store.player().unwrap().id, "player");
        assert_eq!(store.len(), 2);

        store.apply_patches(&[
            EntityPatch {
                id: "player".into(),
                pos: Some(Coord::new(1, 0)),
                tile: None,
                is_player: None,
                remove: false,
            },
            EntityPatch {
                id: "crate1".into(),
                pos: None,
                tile: None,
                is_player: None,
                remove: true,
            },
            EntityPatch {
                id: "gem1".into(),
                pos: Some(Coord::new(4, -2)),
                tile: Some("G".into()),
                is_player: None,
                remove: false,
            },
        ]);

        assert_eq!(store.len(), 2);
        assert_eq!(store.player().unwrap().pos, Coord::new(1, 0));
        let ids: Vec<&str> = store.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["player", "gem1"]);
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
