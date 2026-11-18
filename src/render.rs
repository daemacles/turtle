//! Fixed-size terminal grid compositor using sixteen ANSI colors and transparent entity overlays.
//!
//! Renders terrain tiles, overlaid entities, camera reticles, status bars, event logs, and
//! non-fatal script error modals into an off-screen [Display] buffer without performing terminal
//! input or running Lua scripts directly.

use std::collections::HashMap;

use terminal_graphics::shapes::Rect;
use terminal_graphics::{Colour, Container, Display, Graphic, Text};

use crate::model::{
    CellWidth, Coord, Entity, EntityStore, MessageLog, Pacing, RectBounds, TileStyle, VIEWPORT_ROWS,
};

/// Total character columns of the rendered terminal frame, chosen to avoid edge wrapping.
pub const FRAME_WIDTH: u32 = 78;

/// Total character rows of the rendered terminal frame, leaving one cursor row inside standard panes.
pub const FRAME_HEIGHT: u32 = 23;

/// Maximum text columns available inside the left and right outer frame borders.
const INNER_WIDTH: usize = 74;

/// Borrowed snapshot of all state required to paint one complete terminal frame.
pub struct FrameState<'a> {
    /// Display title declared by the active world script.
    pub title: &'a str,
    /// Active simulation clock mode shown in the header bar.
    pub pacing: Pacing,
    /// Terminal columns per logical world tile.
    pub cell_width: CellWidth,
    /// Current simulation step counter starting from zero after initialization or reload.
    pub tick: u64,
    /// Active random seed shown in the header for reproducibility.
    pub seed: u64,
    /// Whether timed stepping is currently paused.
    pub paused: bool,
    /// Whether the reload confirmation highlight is currently active.
    pub reload_flash: bool,
    /// Inclusive world bounding rectangle matching the terrain grid dimensions.
    pub bounds: RectBounds,
    /// Two-dimensional row-major terrain grid returned by the world script.
    pub terrain: &'a [Vec<TileStyle>],
    /// Authoritative live entities overlaid on top of terrain.
    pub entities: &'a EntityStore,
    /// Tile visual legend used to look up entity sprites.
    pub palette: &'a HashMap<String, TileStyle>,
    /// Current world coordinate at the center of the viewport.
    pub camera_center: Coord,
    /// Script-supplied status string shown in the bottom divider bar.
    pub hud_status: &'a str,
    /// Recent event and print message queue rendered below the viewport.
    pub log: &'a MessageLog,
    /// Optional syntax or runtime diagnostic rendered as a modal overlay over the frozen world.
    pub error_overlay: Option<&'a str>,
    /// Detected terminal size when smaller than the required minimum frame dimensions.
    pub terminal_too_small: Option<(u16, u16)>,
}

/// Multi-line modal block drawn inside a bordered error container.
struct OverlayBlock {
    /// Horizontal origin relative to the container interior.
    pos_x: isize,
    /// Vertical origin relative to the container interior.
    pos_y: isize,
    /// Colored text rows rendered top to bottom.
    lines: Vec<(String, Colour)>,
}

impl Graphic for OverlayBlock {
    fn draw(&self, display: &mut Display) {
        for (row, (text, fg)) in self.lines.iter().enumerate() {
            Text::new(
                self.pos_x,
                self.pos_y.saturating_add(row as isize),
                text,
                *fg,
                Colour::Black,
            )
            .draw(display);
        }
    }

    fn position(&self) -> (isize, isize) {
        (self.pos_x, self.pos_y)
    }

    fn set_position(&mut self, pos_x: isize, pos_y: isize) {
        self.pos_x = pos_x;
        self.pos_y = pos_y;
    }
}

/// Paints the entire frame buffer from state without flushing to standard output.
///
/// Clears all cells to black first so default white backgrounds never leak through, then draws the
/// outer frame, terrain tiles, entity sprites with background inheritance, status dividers, event
/// log, controls legend, and any active error modal.
///
/// - `display`: target off-screen grid sized to [FRAME_WIDTH] by [FRAME_HEIGHT].
/// - `state`: immutable simulation and UI snapshot for the current frame.
pub fn compose_frame(display: &mut Display, state: &FrameState<'_>) {
    display.clear();
    Rect::new(0, 0, FRAME_WIDTH as isize, FRAME_HEIGHT as isize, Colour::Black).draw(display);

    if let Some((cols, rows)) = state.terminal_too_small {
        draw_small_terminal_notice(display, cols, rows);
        return;
    }

    let border_colour = if state.error_overlay.is_some() {
        Colour::BrightRed
    } else if state.reload_flash {
        Colour::BrightGreen
    } else if state.paused {
        Colour::Yellow
    } else {
        Colour::Cyan
    };

    draw_outer_chrome(display, border_colour);
    draw_header_row(display, state, border_colour);

    let terrain_bgs = draw_terrain(display, state);
    if state.entities.player().is_none() {
        draw_center_cursor(display, state, &terrain_bgs);
    }
    draw_entities(display, state, &terrain_bgs);

    draw_divider_status(display, state, border_colour);
    draw_event_log(display, state.log);
    draw_controls_row(display, state.entities.player().is_some());

    if let Some(err) = state.error_overlay {
        draw_error_overlay(display, err);
    }
}

/// Picks a legible foreground color whenever a transparent actor shares the tile background color.
///
/// - `fg`: desired sprite foreground color.
/// - `bg`: resolved cell background color underneath the sprite.
pub fn contrast_safe_fg(fg: Colour, bg: Colour) -> Colour {
    if fg != bg {
        return fg;
    }
    match bg {
        Colour::White
        | Colour::BrightWhite
        | Colour::BrightYellow
        | Colour::BrightCyan
        | Colour::BrightGreen
        | Colour::Yellow => Colour::Black,
        _ => Colour::BrightWhite,
    }
}

/// Draws a compact notice when the terminal pane is narrower or shorter than the required canvas.
fn draw_small_terminal_notice(display: &mut Display, cols: u16, rows: u16) {
    let w = (cols as isize).clamp(22, FRAME_WIDTH as isize - 2);
    let h = (rows.saturating_sub(1) as isize).clamp(5, 9);
    let msg = format!("Need {FRAME_WIDTH}x24 pane (got {cols}x{rows})");
    let block = OverlayBlock {
        pos_x: 1,
        pos_y: 0,
        lines: vec![
            (truncate_chars(&msg, (w - 4).max(1) as usize), Colour::BrightYellow),
            ("Resize pane or press Q".to_string(), Colour::White),
        ],
    };
    Container::new(0, 0, w, h, Colour::Yellow, Colour::Black)
        .with_content(block)
        .draw(display);
}

/// Draws the outer box borders and the horizontal divider separating viewport and HUD rows.
fn draw_outer_chrome(display: &mut Display, border: Colour) {
    let right = FRAME_WIDTH as isize - 1;
    let bottom = FRAME_HEIGHT as isize - 1;
    let divider_y = VIEWPORT_ROWS as isize + 1;

    for x in 1..right {
        display.set_pixel(x, 0, '─', border, Colour::Black);
        display.set_pixel(x, divider_y, '─', border, Colour::Black);
        display.set_pixel(x, bottom, '─', border, Colour::Black);
    }
    for y in 1..bottom {
        display.set_pixel(0, y, '│', border, Colour::Black);
        display.set_pixel(right, y, '│', border, Colour::Black);
    }
    display.set_pixel(0, 0, '┌', border, Colour::Black);
    display.set_pixel(right, 0, '┐', border, Colour::Black);
    display.set_pixel(0, divider_y, '├', border, Colour::Black);
    display.set_pixel(right, divider_y, '┤', border, Colour::Black);
    display.set_pixel(0, bottom, '└', border, Colour::Black);
    display.set_pixel(right, bottom, '┘', border, Colour::Black);
}

/// Renders the world title, execution badge, pacing mode, tick counter, and seed on the top border.
fn draw_header_row(display: &mut Display, state: &FrameState<'_>, border: Colour) {
    let badge = if state.error_overlay.is_some() {
        "ERROR"
    } else if state.paused {
        "PAUSED"
    } else {
        "LIVE"
    };
    let right_tag = format!(
        " {} t:{:04} seed:{:04X} ",
        state.pacing.label(),
        state.tick,
        state.seed & 0xffff
    );
    let max_left = INNER_WIDTH.saturating_sub(right_tag.chars().count());
    let left_raw = format!(" {} [{}] ", state.title, badge);
    let left_tag = truncate_chars(&left_raw, max_left);

    Text::new(2, 0, &left_tag, Colour::BrightWhite, Colour::Black).draw(display);
    let right_x = (FRAME_WIDTH as usize - 2 - right_tag.chars().count()) as isize;
    let right_colour = if state.error_overlay.is_some() {
        Colour::BrightRed
    } else if state.paused {
        Colour::BrightYellow
    } else {
        border
    };
    Text::new(right_x, 0, &right_tag, right_colour, Colour::Black).draw(display);
}

/// Paints the terrain rows into the viewport and returns resolved cell backgrounds for entities.
fn draw_terrain(display: &mut Display, state: &FrameState<'_>) -> Vec<Vec<Colour>> {
    let view_cols = state.cell_width.viewport_cols();
    let mut backgrounds = vec![vec![Colour::Black; view_cols]; VIEWPORT_ROWS];

    for (ry, bg_row) in backgrounds.iter_mut().enumerate() {
        let Some(tile_row) = state.terrain.get(ry) else {
            continue;
        };
        let screen_y = 1 + ry as isize;
        for (rx, bg_slot) in bg_row.iter_mut().enumerate() {
            let Some(style) = tile_row.get(rx) else {
                continue;
            };
            let bg = style.bg.unwrap_or(Colour::Black);
            *bg_slot = bg;
            match state.cell_width {
                CellWidth::One => {
                    let sx = 2 + rx as isize;
                    display.set_pixel(sx, screen_y, style.single, style.fg, bg);
                }
                CellWidth::Two => {
                    let sx = 2 + (rx * 2) as isize;
                    display.set_pixel(sx, screen_y, style.glyph[0], style.fg, bg);
                    display.set_pixel(sx + 1, screen_y, style.glyph[1], style.fg, bg);
                }
            }
        }
    }

    backgrounds
}

/// Marks the center cell in playerless worlds so the user sees where act toggles happen.
fn draw_center_cursor(display: &mut Display, state: &FrameState<'_>, terrain_bgs: &[Vec<Colour>]) {
    let rx = state.cell_width.viewport_cols() / 2;
    let ry = VIEWPORT_ROWS / 2;
    let screen_y = 1 + ry as isize;
    let under_bg = terrain_bgs
        .get(ry)
        .and_then(|r| r.get(rx))
        .copied()
        .unwrap_or(Colour::Black);
    let cursor_bg = if under_bg == Colour::Blue {
        Colour::Cyan
    } else {
        Colour::Blue
    };

    match state.cell_width {
        CellWidth::One => {
            let sx = 2 + rx as isize;
            let ch = display.pixel(sx, screen_y).map_or('+', |p| {
                let c = p.character();
                if c == ' ' || c == '·' { '+' } else { c }
            });
            let fg = if ch == '█' {
                Colour::BrightCyan
            } else {
                Colour::BrightWhite
            };
            display.set_pixel(sx, screen_y, ch, fg, cursor_bg);
        }
        CellWidth::Two => {
            let sx = 2 + (rx * 2) as isize;
            display.set_pixel(sx, screen_y, '[', Colour::BrightWhite, cursor_bg);
            display.set_pixel(sx + 1, screen_y, ']', Colour::BrightWhite, cursor_bg);
        }
    }
}

/// Composites visible entities over terrain, keeping player entities drawn last on top.
fn draw_entities(display: &mut Display, state: &FrameState<'_>, terrain_bgs: &[Vec<Colour>]) {
    let fallback = TileStyle::from_spec("?", Colour::BrightMagenta, None);
    for pass_player in [false, true] {
        for entity in state.entities.iter() {
            if entity.is_player != pass_player {
                continue;
            }
            draw_one_entity(display, state, entity, terrain_bgs, &fallback);
        }
    }
}

/// Draws a single visible entity using terrain background inheritance and contrast checking.
fn draw_one_entity(
    display: &mut Display,
    state: &FrameState<'_>,
    entity: &Entity,
    terrain_bgs: &[Vec<Colour>],
    fallback: &TileStyle,
) {
    if !state.bounds.contains(entity.pos) {
        return;
    }
    let rx = (entity.pos.x - state.bounds.min.x) as usize;
    let ry = (entity.pos.y - state.bounds.min.y) as usize;
    let style = state.palette.get(&entity.tile).unwrap_or(fallback);
    let under_bg = terrain_bgs
        .get(ry)
        .and_then(|r| r.get(rx))
        .copied()
        .unwrap_or(Colour::Black);
    let bg = style.bg.unwrap_or(under_bg);
    let fg = contrast_safe_fg(style.fg, bg);
    let screen_y = 1 + ry as isize;

    match state.cell_width {
        CellWidth::One => {
            let sx = 2 + rx as isize;
            display.set_pixel(sx, screen_y, style.single, fg, bg);
        }
        CellWidth::Two => {
            let sx = 2 + (rx * 2) as isize;
            display.set_pixel(sx, screen_y, style.glyph[0], fg, bg);
            display.set_pixel(sx + 1, screen_y, style.glyph[1], fg, bg);
        }
    }
}

/// Renders position, script HUD summary, entity count, and reload state on the divider row.
fn draw_divider_status(display: &mut Display, state: &FrameState<'_>, border: Colour) {
    let divider_y = VIEWPORT_ROWS as isize + 1;
    let pos_tag = if let Some(player) = state.entities.player() {
        format!(" @({},{}) ", player.pos.x, player.pos.y)
    } else {
        format!(" cam({},{}) ", state.camera_center.x, state.camera_center.y)
    };
    let reload_tag = if state.error_overlay.is_some() {
        " FROZEN "
    } else if state.reload_flash {
        " RELOADED -> t=0 "
    } else {
        " OK "
    };

    let mid = if state.hud_status.is_empty() {
        format!("entities:{}", state.entities.len())
    } else {
        state.hud_status.to_string()
    };

    let left_raw = format!("{pos_tag} {mid} ");
    let max_left = INNER_WIDTH.saturating_sub(reload_tag.chars().count());
    let left_trimmed = truncate_chars(&left_raw, max_left);

    Text::new(2, divider_y, &left_trimmed, Colour::BrightWhite, Colour::Black).draw(display);
    let right_x = (FRAME_WIDTH as usize - 2 - reload_tag.chars().count()) as isize;
    let right_fg = if state.error_overlay.is_some() {
        Colour::BrightRed
    } else if state.reload_flash {
        Colour::BrightGreen
    } else {
        border
    };
    Text::new(right_x, divider_y, reload_tag, right_fg, Colour::Black).draw(display);
}

/// Renders the two most recent sanitized event lines directly beneath the divider bar.
fn draw_event_log(display: &mut Display, log: &MessageLog) {
    let (older, newer) = log.recent_two();
    if !older.is_empty() {
        let line = truncate_chars(&format!("  {older}"), INNER_WIDTH);
        Text::new(2, 19, &line, Colour::White, Colour::Black).draw(display);
    }
    if !newer.is_empty() {
        let line = truncate_chars(&format!("> {newer}"), INNER_WIDTH);
        Text::new(2, 20, &line, Colour::BrightWhite, Colour::Black).draw(display);
    }
}

/// Renders the single-line keybinding legend tailored to entity or camera-pan mode.
fn draw_controls_row(display: &mut Display, has_player: bool) {
    let legend = if has_player {
        "[WASD/Arrows] Move [Space] Wait [E] Act [P] Pause [N] Step [r/R] [Q] Quit"
    } else {
        "[WASD/Arrows] Pan [E] Toggle [.] Wait [P] Pause [N] Step [r/R] [Q] Quit"
    };
    let clipped = truncate_chars(legend, INNER_WIDTH);
    Text::new(2, 21, &clipped, Colour::BrightBlack, Colour::Black).draw(display);
}

/// Renders a non-fatal script error overlay box centered inside the fixed viewport frame.
fn draw_error_overlay(display: &mut Display, err: &str) {
    let box_width: isize = 72;
    let content_width = (box_width - 4) as usize;
    let max_body_lines = 9;

    let mut lines: Vec<(String, Colour)> = vec![(
        "Lua script error (previous valid state frozen):".to_string(),
        Colour::BrightRed,
    )];

    let wrapped = wrap_overlay_text(err, content_width);
    for line in wrapped.into_iter().take(max_body_lines) {
        lines.push((line, Colour::White));
    }
    lines.push((
        "Save script to hot-reload | [r] retry t=0 | [Esc] hide | [Q] quit".to_string(),
        Colour::BrightYellow,
    ));

    let height = (lines.len() as isize + 2).min(VIEWPORT_ROWS as isize);
    let block = OverlayBlock {
        pos_x: 1,
        pos_y: 0,
        lines,
    };
    Container::new(3, 2, box_width, height, Colour::BrightRed, Colour::Black)
        .with_content(block)
        .draw(display);
}

/// Wraps diagnostic text to fit within the allowed column width while stripping control characters.
fn wrap_overlay_text(text: &str, max_width: usize) -> Vec<String> {
    let width = max_width.max(1);
    let normalized = text.replace('\r', "").replace('\t', "  ");
    let mut out = Vec::new();
    for raw in normalized.split('\n') {
        if raw.is_empty() {
            continue;
        }
        let mut cur = String::new();
        let mut count = 0;
        for ch in raw.chars() {
            if count == width {
                out.push(cur);
                cur = String::new();
                count = 0;
            }
            cur.push(ch);
            count += 1;
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

/// Truncates text cleanly at a maximum Unicode scalar count.
fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    //! Unit tests verifying frame dimensions, background fills, transparent entities, and error modals.

    use super::*;
    use crate::model::{Camera, Coord, Entity, MessageLog, Pacing};

    fn empty_terrain(cols: usize, bg: Option<Colour>) -> Vec<Vec<TileStyle>> {
        vec![vec![TileStyle::from_spec("≈~", Colour::BrightCyan, bg); cols]; VIEWPORT_ROWS]
    }

    #[test]
    fn compose_frame_keeps_fixed_dimensions_and_inherits_terrain_bg() {
        let mut palette = HashMap::new();
        palette.insert(
            "@".to_string(),
            TileStyle::from_spec("@", Colour::BrightYellow, None),
        );

        let store = EntityStore::from_entities(vec![Entity {
            id: "player".into(),
            pos: Coord::new(0, 0),
            tile: "@".into(),
            is_player: true,
        }]);
        let cam = Camera::default();
        let bounds = cam.bounds(CellWidth::Two.viewport_cols(), VIEWPORT_ROWS);
        let terrain = empty_terrain(CellWidth::Two.viewport_cols(), Some(Colour::Blue));
        let mut log = MessageLog::default();
        log.push_line("Splash! Built a bridge.");

        let state = FrameState {
            title: "Tidekeep",
            pacing: Pacing::Hybrid { idle_ms: 650 },
            cell_width: CellWidth::Two,
            tick: 3,
            seed: 0x2a,
            paused: false,
            reload_flash: false,
            bounds,
            terrain: &terrain,
            entities: &store,
            palette: &palette,
            camera_center: cam.center,
            hud_status: "GEMS 1/5",
            log: &log,
            error_overlay: None,
            terminal_too_small: None,
        };

        let mut display = Display::new(FRAME_WIDTH, FRAME_HEIGHT);
        compose_frame(&mut display, &state);

        // No default white backgrounds anywhere in gutters or log rows.
        assert_eq!(display.pixel(1, 1).unwrap().background(), Colour::Black);
        assert_eq!(display.pixel(76, 1).unwrap().background(), Colour::Black);
        assert_eq!(display.pixel(10, 20).unwrap().background(), Colour::Black);

        // Player at world (0,0) maps to center viewport cell (rx=18, ry=8) -> screen x=38,39, y=9.
        let left_cell = display.pixel(38, 9).unwrap();
        let right_cell = display.pixel(39, 9).unwrap();
        assert_eq!(left_cell.character(), '@');
        assert_eq!(right_cell.character(), ' ');
        assert_eq!(left_cell.background(), Colour::Blue);
        assert_eq!(right_cell.background(), Colour::Blue);
        assert_eq!(left_cell.colour(), Colour::BrightYellow);
    }

    #[test]
    fn contrast_guard_and_error_overlay_render_cleanly() {
        assert_eq!(
            contrast_safe_fg(Colour::BrightYellow, Colour::BrightYellow),
            Colour::Black
        );
        assert_eq!(contrast_safe_fg(Colour::Blue, Colour::Blue), Colour::BrightWhite);

        let palette = HashMap::new();
        let store = EntityStore::default();
        let cam = Camera::default();
        let bounds = cam.bounds(CellWidth::One.viewport_cols(), VIEWPORT_ROWS);
        let terrain = empty_terrain(CellWidth::One.viewport_cols(), Some(Colour::Black));
        let log = MessageLog::default();

        let state = FrameState {
            title: "Life",
            pacing: Pacing::Turn,
            cell_width: CellWidth::One,
            tick: 0,
            seed: 1,
            paused: true,
            reload_flash: false,
            bounds,
            terrain: &terrain,
            entities: &store,
            palette: &palette,
            camera_center: cam.center,
            hud_status: "",
            log: &log,
            error_overlay: Some("world.lua:5: unexpected symbol near '='"),
            terminal_too_small: None,
        };

        let mut display = Display::new(FRAME_WIDTH, FRAME_HEIGHT);
        compose_frame(&mut display, &state);

        assert_eq!(display.size(), (FRAME_WIDTH, FRAME_HEIGHT));
        assert_eq!(display.pixel(3, 2).unwrap().character(), '┌');
        assert_eq!(display.pixel(3, 2).unwrap().colour(), Colour::BrightRed);
    }
}
