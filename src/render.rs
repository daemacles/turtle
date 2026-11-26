//! Adaptive terminal grid compositor using sixteen ANSI colors and live inspection views.
//!
//! Renders terrain tiles, z-ordered entities with background inheritance, dynamic status bars,
//! event logs, a live entity-component inspector drawer or modal, and non-fatal script error
//! modals into a dynamically sized [Display] buffer without performing terminal input or executing
//! Lua code directly.

use std::collections::HashMap;

use terminal_graphics::shapes::Rect;
use terminal_graphics::{Colour, Container, Display, Graphic, Text};

use crate::ecs::{EcsWorld, EntityId};
use crate::model::{CellWidth, Coord, FrameLayout, MessageLog, Pacing, RectBounds, TileStyle};

/// Borrowed snapshot of all state required to paint one complete terminal frame.
pub struct FrameState<'a> {
    /// Display title declared by the active world script.
    pub title: &'a str,
    /// Active simulation clock mode shown in the header bar.
    pub pacing: Pacing,
    /// Terminal columns per logical world tile.
    pub cell_width: CellWidth,
    /// Dynamic screen and viewport geometry for this frame.
    pub layout: FrameLayout,
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
    /// Authoritative entity-component world rendered on top of terrain.
    pub ecs: &'a EcsWorld,
    /// Tile visual legend used to look up entity sprites.
    pub palette: &'a HashMap<String, TileStyle>,
    /// Current world coordinate at the center of the viewport.
    pub camera_center: Coord,
    /// Script-supplied status string shown in the bottom divider bar.
    pub hud_status: &'a str,
    /// Recent event and print message queue rendered below the viewport.
    pub log: &'a MessageLog,
    /// Whether the live entity-component inspector is currently open.
    pub inspector_open: bool,
    /// Zero-based index of the currently selected entity in creation order.
    pub inspector_selected: usize,
    /// Optional syntax or runtime diagnostic rendered as a modal overlay over the frozen world.
    pub error_overlay: Option<&'a str>,
    /// Detected terminal size when smaller than the minimum playable frame dimensions.
    pub terminal_too_small: Option<(u16, u16)>,
}

/// Multi-line modal block drawn inside a bordered overlay container.
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

/// Prepared entity list and component detail rows shared by inspector drawer and modal renderers.
struct InspectorSnapshot {
    /// Single-line entity count, slot capacity, and store count summary.
    summary: String,
    /// Windowed entity rows paired with selection highlight flags.
    entity_rows: Vec<(String, bool)>,
    /// Header identifying the currently selected entity handle and name, if any.
    selected_header: Option<String>,
    /// Wrapped component key-value lines for the selected entity.
    component_lines: Vec<String>,
}

/// Paints the entire frame buffer from state without flushing to standard output.
///
/// Clears all cells to black first so default white backgrounds never leak through, then draws the
/// outer frame, terrain tiles, z-ordered entity sprites with background inheritance, status bars,
/// event log, controls legend, optional inspector view, and any active error modal.
///
/// - `display`: target off-screen grid sized to the active frame layout.
/// - `state`: immutable simulation and UI snapshot for the current frame.
pub fn compose_frame(display: &mut Display, state: &FrameState<'_>) {
    display.clear();
    Rect::new(
        0,
        0,
        display.width() as isize,
        display.height() as isize,
        Colour::Black,
    )
    .draw(display);

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

    draw_outer_chrome(display, state.layout, border_colour);
    draw_header_row(display, state, border_colour);

    let mut cell_bgs = draw_terrain(display, state);
    if state.ecs.player().is_none() {
        draw_center_cursor(display, state, &cell_bgs);
    }
    draw_entities(display, state, &mut cell_bgs);

    draw_divider_status(display, state, border_colour);
    draw_event_log(display, state.layout, state.log);
    draw_controls_row(
        display,
        state.layout,
        state.ecs.player().is_some(),
        state.inspector_open,
    );

    if state.inspector_open {
        if state.layout.inspector_drawer_width > 0 {
            draw_inspector_drawer(display, state, border_colour);
        } else {
            draw_inspector_modal(display, state);
        }
    }

    if let Some(err) = state.error_overlay {
        draw_error_overlay(display, state.layout, err);
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

/// Draws a compact notice when the terminal pane is narrower or shorter than the playable minimum.
fn draw_small_terminal_notice(display: &mut Display, cols: u16, rows: u16) {
    let w = (cols as isize).min(display.width() as isize).max(4);
    let h = (rows.saturating_sub(1) as isize)
        .min(display.height() as isize)
        .clamp(4, 9);
    let msg = format!("Need >=52x14 pane (got {cols}x{rows})");
    let block = OverlayBlock {
        pos_x: 1,
        pos_y: 0,
        lines: vec![
            (truncate_chars(&msg, (w - 4).max(1) as usize), Colour::BrightYellow),
            (
                truncate_chars("Resize pane or press Q", (w - 4).max(1) as usize),
                Colour::White,
            ),
        ],
    };
    draw_opaque_modal(display, 0, 0, w, h, Colour::Yellow, block);
}

/// Draws the outer frame borders, map footer divider, and optional side-by-side drawer separator.
fn draw_outer_chrome(display: &mut Display, layout: FrameLayout, border: Colour) {
    let map_right = layout.map_width() as isize - 1;
    let frame_right = layout.frame_width as isize - 1;
    let bottom = layout.frame_height as isize - 1;
    let divider_y = layout.viewport_rows as isize + 1;
    let has_drawer = layout.inspector_drawer_width > 0;

    for x in 1..frame_right {
        display.set_pixel(x, 0, '─', border, Colour::Black);
        display.set_pixel(x, bottom, '─', border, Colour::Black);
    }
    for x in 1..map_right {
        display.set_pixel(x, divider_y, '─', border, Colour::Black);
    }
    for y in 1..bottom {
        display.set_pixel(0, y, '│', border, Colour::Black);
        display.set_pixel(frame_right, y, '│', border, Colour::Black);
        if has_drawer {
            display.set_pixel(map_right, y, '│', border, Colour::Black);
        }
    }

    display.set_pixel(0, 0, '┌', border, Colour::Black);
    display.set_pixel(frame_right, 0, '┐', border, Colour::Black);
    display.set_pixel(0, divider_y, '├', border, Colour::Black);
    display.set_pixel(map_right, divider_y, '┤', border, Colour::Black);
    display.set_pixel(0, bottom, '└', border, Colour::Black);
    display.set_pixel(frame_right, bottom, '┘', border, Colour::Black);

    if has_drawer {
        display.set_pixel(map_right, 0, '┬', border, Colour::Black);
        display.set_pixel(map_right, bottom, '┴', border, Colour::Black);
    }
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
    let inner_w = state.layout.map_inner_width();
    let max_left = inner_w.saturating_sub(right_tag.chars().count());
    let left_raw = format!(" {} [{}] ", state.title, badge);
    let left_tag = truncate_chars(&left_raw, max_left);

    Text::new(2, 0, &left_tag, Colour::BrightWhite, Colour::Black).draw(display);
    let right_x = state
        .layout
        .map_width()
        .saturating_sub(2)
        .saturating_sub(right_tag.chars().count())
        .max(2) as isize;
    let right_colour = if state.error_overlay.is_some() {
        Colour::BrightRed
    } else if state.paused {
        Colour::BrightYellow
    } else {
        border
    };
    Text::new(right_x, 0, &right_tag, right_colour, Colour::Black).draw(display);

    if state.layout.inspector_drawer_width > 0 {
        let drawer_x = state.layout.map_width() as isize + 1;
        let max_title = state.layout.inspector_drawer_width.saturating_sub(4);
        let title = truncate_chars(" ECS Inspector ", max_title);
        Text::new(drawer_x, 0, &title, Colour::BrightCyan, Colour::Black).draw(display);
    }
}

/// Paints the terrain rows into the viewport and returns resolved cell backgrounds for entities.
fn draw_terrain(display: &mut Display, state: &FrameState<'_>) -> Vec<Vec<Colour>> {
    let view_cols = state.layout.viewport_cols;
    let view_rows = state.layout.viewport_rows;
    let mut backgrounds = vec![vec![Colour::Black; view_cols]; view_rows];

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
    let rx = state.layout.viewport_cols / 2;
    let ry = state.layout.viewport_rows / 2;
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

/// Composites visible entities over terrain in z-layer and creation order.
fn draw_entities(display: &mut Display, state: &FrameState<'_>, cell_bgs: &mut [Vec<Colour>]) {
    let fallback = TileStyle::from_spec("?", Colour::BrightMagenta, None);
    for (_id, pos, sprite, _is_player) in state.ecs.renderables() {
        draw_one_entity(display, state, pos, &sprite.tile, cell_bgs, &fallback);
    }
}

/// Draws a single visible entity using underlying background inheritance and contrast checking.
fn draw_one_entity(
    display: &mut Display,
    state: &FrameState<'_>,
    pos: Coord,
    tile_key: &str,
    cell_bgs: &mut [Vec<Colour>],
    fallback: &TileStyle,
) {
    if !state.bounds.contains(pos) {
        return;
    }
    let rx = (pos.x - state.bounds.min.x) as usize;
    let ry = (pos.y - state.bounds.min.y) as usize;
    let style = state.palette.get(tile_key).unwrap_or(fallback);
    let under_bg = cell_bgs
        .get(ry)
        .and_then(|r| r.get(rx))
        .copied()
        .unwrap_or(Colour::Black);
    let bg = style.bg.unwrap_or(under_bg);
    if let Some(slot) = cell_bgs.get_mut(ry).and_then(|r| r.get_mut(rx)) {
        *slot = bg;
    }
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
    let divider_y = state.layout.viewport_rows as isize + 1;
    let pos_tag = if let Some((_, pos)) = state.ecs.player() {
        format!(" @({},{}) ", pos.x, pos.y)
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
        format!("entities:{}", state.ecs.len())
    } else {
        state.hud_status.to_string()
    };

    let left_raw = format!("{pos_tag} {mid} ");
    let inner_w = state.layout.map_inner_width();
    let max_left = inner_w.saturating_sub(reload_tag.chars().count());
    let left_trimmed = truncate_chars(&left_raw, max_left);

    Text::new(2, divider_y, &left_trimmed, Colour::BrightWhite, Colour::Black).draw(display);
    let right_x = state
        .layout
        .map_width()
        .saturating_sub(2)
        .saturating_sub(reload_tag.chars().count())
        .max(2) as isize;
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
fn draw_event_log(display: &mut Display, layout: FrameLayout, log: &MessageLog) {
    let base_y = layout.viewport_rows as isize + 2;
    let inner_w = layout.map_inner_width();
    let (older, newer) = log.recent_two();
    if !older.is_empty() {
        let line = truncate_chars(&format!("  {older}"), inner_w);
        Text::new(2, base_y, &line, Colour::White, Colour::Black).draw(display);
    }
    if !newer.is_empty() {
        let line = truncate_chars(&format!("> {newer}"), inner_w);
        Text::new(2, base_y + 1, &line, Colour::BrightWhite, Colour::Black).draw(display);
    }
}

/// Renders the single-line keybinding legend tailored to entity or camera-pan mode.
fn draw_controls_row(
    display: &mut Display,
    layout: FrameLayout,
    has_player: bool,
    inspector_open: bool,
) {
    let y = layout.viewport_rows as isize + 4;
    let legend = match (has_player, inspector_open) {
        (true, true) => {
            "[WASD] Move [.] Wait [E] Act [P] Pause [N] Step [Tab] ECS [[/]] Sel [Q]"
        }
        (true, false) => {
            "[WASD] Move [Space] Wait [E] Act [P] Pause [N] Step [Tab/I] ECS [r/R] [Q]"
        }
        (false, true) => {
            "[WASD] Pan [E] Toggle [P] Pause [N] Step [Tab/I] ECS [[/]] Sel [Q] Quit"
        }
        (false, false) => {
            "[WASD] Pan [E] Toggle [.] Wait [P] Pause [N] Step [Tab/I] ECS [r/R] [Q]"
        }
    };
    let clipped = truncate_chars(legend, layout.map_inner_width());
    Text::new(2, y, &clipped, Colour::BrightBlack, Colour::Black).draw(display);
}

/// Builds a windowed snapshot of live entities and component descriptions for inspector views.
fn build_inspector_snapshot(
    ecs: &EcsWorld,
    selected_idx: usize,
    max_entities: usize,
    text_width: usize,
) -> InspectorSnapshot {
    let summary = format!(
        "{} alive  {} slots  {} stores",
        ecs.len(),
        ecs.slot_capacity(),
        ecs.store_count()
    );
    let live = ecs.live_entities();
    if live.is_empty() {
        return InspectorSnapshot {
            summary,
            entity_rows: vec![("(no live entities)".to_string(), false)],
            selected_header: None,
            component_lines: vec!["World has no entities.".to_string()],
        };
    }

    let clamped = selected_idx.min(live.len() - 1);
    let window = max_entities.max(1).min(live.len());
    let half = window / 2;
    let start = clamped
        .saturating_sub(half)
        .min(live.len().saturating_sub(window));
    let end = (start + window).min(live.len());

    let mut entity_rows = Vec::with_capacity(end - start);
    for (idx, &eid) in live.iter().enumerate().take(end).skip(start) {
        let selected = idx == clamped;
        let marker = if selected { '>' } else { ' ' };
        let label = ecs.name(eid).unwrap_or("-");
        let pos_tag = ecs
            .position(eid)
            .map_or_else(String::new, |p| format!(" ({},{})", p.x, p.y));
        let line = truncate_chars(&format!("{marker} {eid} {label}{pos_tag}"), text_width);
        entity_rows.push((line, selected));
    }

    let selected_id = live[clamped];
    let selected_header = Some(truncate_chars(
        &format!(
            "Selected: {selected_id} {}",
            ecs.name(selected_id).unwrap_or("(unnamed)")
        ),
        text_width,
    ));

    let mut component_lines = Vec::new();
    let components = ecs.inspect_components(selected_id);
    if components.is_empty() {
        component_lines.push("(no components)".to_string());
    } else {
        for (key, val) in components {
            let wrapped = wrap_overlay_text(&format!("{key} = {val}"), text_width);
            for (line_idx, segment) in wrapped.into_iter().enumerate() {
                if line_idx == 0 {
                    component_lines.push(segment);
                } else {
                    component_lines.push(truncate_chars(&format!("  {segment}"), text_width));
                }
            }
        }
    }

    InspectorSnapshot {
        summary,
        entity_rows,
        selected_header,
        component_lines,
    }
}

/// Formats one concise inspector header string for a selected entity handle.
fn format_selected_tag(ecs: &EcsWorld, id: EntityId) -> String {
    match ecs.name(id) {
        Some(name) => format!("{id} \"{name}\""),
        None => format!("{id}"),
    }
}

/// Renders the docked right-hand entity-component inspector drawer on wide terminal panes.
fn draw_inspector_drawer(display: &mut Display, state: &FrameState<'_>, border: Colour) {
    let sep_x = state.layout.map_width() as isize - 1;
    let frame_right = state.layout.frame_width as isize - 1;
    let text_x = sep_x + 2;
    let text_w = state.layout.inspector_drawer_width.saturating_sub(4).max(8);
    let inner_top: isize = 1;
    let inner_bottom = state.layout.frame_height as isize - 2;
    let total_rows = (inner_bottom - inner_top + 1).max(6) as usize;

    // Reserve header, dividers, selected entity banner, and footer before splitting list and fields.
    let list_budget = ((total_rows.saturating_sub(6)) * 2 / 5).clamp(3, 12);
    let snap = build_inspector_snapshot(state.ecs, state.inspector_selected, list_budget, text_w);

    // 1. Summary stats row at the top of the drawer.
    let summary = truncate_chars(&snap.summary, text_w);
    Text::new(text_x, inner_top, &summary, Colour::BrightCyan, Colour::Black).draw(display);

    // 2. Thin separator beneath summary stats.
    let stats_sep_y = inner_top + 1;
    draw_drawer_divider(display, sep_x, frame_right, stats_sep_y, border);

    // 3. Scrollable entity list.
    let mut cursor_y = stats_sep_y + 1;
    for (line, selected) in &snap.entity_rows {
        if cursor_y >= inner_bottom - 2 {
            break;
        }
        let fg = if *selected {
            Colour::BrightYellow
        } else {
            Colour::White
        };
        Text::new(text_x, cursor_y, line, fg, Colour::Black).draw(display);
        cursor_y += 1;
    }

    // 4. Component pane header and fields for the selected entity.
    if cursor_y < inner_bottom - 2 {
        draw_drawer_divider(display, sep_x, frame_right, cursor_y, border);
        let title = truncate_chars(" Components ", text_w);
        Text::new(text_x, cursor_y, &title, Colour::BrightCyan, Colour::Black).draw(display);
        cursor_y += 1;
    }

    let live = state.ecs.live_entities();
    if let Some(&selected_id) = live
        .get(state.inspector_selected.min(live.len().saturating_sub(1)))
        && cursor_y < inner_bottom
    {
        let banner = truncate_chars(&format_selected_tag(state.ecs, selected_id), text_w);
        Text::new(text_x, cursor_y, &banner, Colour::BrightYellow, Colour::Black).draw(display);
        cursor_y += 1;
    }

    for line in snap.component_lines {
        if cursor_y >= inner_bottom {
            break;
        }
        let clipped = truncate_chars(&line, text_w);
        Text::new(text_x, cursor_y, &clipped, Colour::White, Colour::Black).draw(display);
        cursor_y += 1;
    }

    // 5. Footer hint pinned to the bottom row inside the drawer.
    let hint = truncate_chars("[Tab/I] Close  [[/]] Sel", text_w);
    Text::new(text_x, inner_bottom, &hint, Colour::BrightBlack, Colour::Black).draw(display);
}

/// Draws a horizontal divider inside the docked inspector drawer with matching border junctions.
fn draw_drawer_divider(
    display: &mut Display,
    sep_x: isize,
    frame_right: isize,
    y: isize,
    border: Colour,
) {
    display.set_pixel(sep_x, y, '├', border, Colour::Black);
    for x in (sep_x + 1)..frame_right {
        display.set_pixel(x, y, '─', border, Colour::Black);
    }
    display.set_pixel(frame_right, y, '┤', border, Colour::Black);
}

/// Renders the entity-component inspector as a bordered modal inside narrow map viewports.
fn draw_inspector_modal(display: &mut Display, state: &FrameState<'_>) {
    let map_w = state.layout.map_width() as isize;
    let box_w = (map_w - 4).clamp(40, 72);
    let max_h = (state.layout.viewport_rows as isize).clamp(7, 17);
    let text_w = (box_w - 4).max(12) as usize;

    let inner_cap = (max_h - 2).max(5) as usize;
    let list_budget = (inner_cap / 3).clamp(2, 6);
    let snap = build_inspector_snapshot(state.ecs, state.inspector_selected, list_budget, text_w);

    let mut lines: Vec<(String, Colour)> = vec![(
        truncate_chars(&format!("ECS Inspector — {}", snap.summary), text_w),
        Colour::BrightCyan,
    )];

    for (row, selected) in snap.entity_rows {
        let fg = if selected {
            Colour::BrightYellow
        } else {
            Colour::White
        };
        lines.push((row, fg));
    }

    if let Some(header) = snap.selected_header
        && lines.len() + 2 < inner_cap
    {
        lines.push((header, Colour::BrightCyan));
    }

    for comp_line in snap.component_lines {
        if lines.len() + 1 >= inner_cap {
            break;
        }
        lines.push((truncate_chars(&comp_line, text_w), Colour::White));
    }

    lines.push((
        truncate_chars("[Tab/I/Esc] Close | [[/]] Select | [N] Step", text_w),
        Colour::BrightYellow,
    ));

    let height = (lines.len() as isize + 2).min(max_h);
    let box_x = ((map_w - box_w) / 2).max(1);
    let box_y = 1 + ((state.layout.viewport_rows as isize - height) / 2).max(0);
    let block = OverlayBlock {
        pos_x: 1,
        pos_y: 0,
        lines,
    };
    draw_opaque_modal(display, box_x, box_y, box_w, height, Colour::BrightCyan, block);
}

/// Renders a non-fatal script error overlay box centered inside the map viewport frame.
fn draw_error_overlay(display: &mut Display, layout: FrameLayout, err: &str) {
    let map_w = layout.map_width() as isize;
    let box_width: isize = (map_w - 4).clamp(26, 72);
    let content_width = (box_width - 4).max(10) as usize;
    let max_body_lines = (layout.viewport_rows as isize - 4).clamp(2, 11) as usize;

    let mut lines: Vec<(String, Colour)> = vec![(
        truncate_chars(
            "Lua script error (previous valid state frozen):",
            content_width,
        ),
        Colour::BrightRed,
    )];

    let wrapped = wrap_overlay_text(err, content_width);
    for line in wrapped.into_iter().take(max_body_lines) {
        lines.push((line, Colour::White));
    }
    lines.push((
        truncate_chars(
            "Save script to hot-reload | [r] retry t=0 | [Esc] hide | [Q] quit",
            content_width,
        ),
        Colour::BrightYellow,
    ));

    let height = (lines.len() as isize + 2).min(layout.viewport_rows as isize);
    let box_x = ((map_w - box_width) / 2).max(1);
    let box_y = 2.min(layout.viewport_rows as isize);
    let block = OverlayBlock {
        pos_x: 1,
        pos_y: 0,
        lines,
    };
    draw_opaque_modal(display, box_x, box_y, box_width, height, Colour::BrightRed, block);
}

/// Paints an opaque modal dialog box without letting underlying map glyphs bleed through.
///
/// Clears all interior cells to black spaces before drawing the container border and text lines.
fn draw_opaque_modal(
    display: &mut Display,
    x: isize,
    y: isize,
    width: isize,
    height: isize,
    border: Colour,
    content: OverlayBlock,
) {
    // Container::draw only updates cell backgrounds and leaves existing glyphs intact, so fill
    // an opaque black rectangle first to erase underlying terrain and entity sprites.
    Rect::new(x, y, width, height, Colour::Black).draw(display);
    Container::new(x, y, width, height, border, Colour::Black)
        .with_content(content)
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
    //! Unit tests verifying dynamic layouts, layer sorting, inspector views, and error modals.

    use super::*;
    use crate::ecs::{ScriptValue, Sprite};
    use crate::model::{Camera, Coord, MessageLog, Pacing};

    fn empty_terrain(cols: usize, rows: usize, bg: Option<Colour>) -> Vec<Vec<TileStyle>> {
        vec![vec![TileStyle::from_spec("≈~", Colour::BrightCyan, bg); cols]; rows]
    }

    #[test]
    fn compose_frame_inherits_terrain_bg_and_respects_sprite_z_layer() {
        let mut palette = HashMap::new();
        palette.insert(
            "@".to_string(),
            TileStyle::from_spec("@", Colour::BrightYellow, None),
        );
        palette.insert(
            "G".to_string(),
            TileStyle::from_spec("<>", Colour::BrightCyan, None),
        );

        let mut ecs = EcsWorld::new();
        let player = ecs.spawn();
        ecs.set_name(player, "player");
        ecs.set_position(player, Coord::new(0, 0));
        ecs.set_sprite(
            player,
            Sprite {
                tile: "@".into(),
                layer: 10,
            },
        );
        ecs.set_player(player, true);

        let gem = ecs.spawn();
        ecs.set_name(gem, "gem1");
        ecs.set_position(gem, Coord::new(0, 0));
        ecs.set_sprite(
            gem,
            Sprite {
                tile: "G".into(),
                layer: 2,
            },
        );

        let layout = FrameLayout::default_for(CellWidth::Two);
        let cam = Camera::default();
        let bounds = cam.bounds(layout.viewport_cols, layout.viewport_rows);
        let terrain = empty_terrain(layout.viewport_cols, layout.viewport_rows, Some(Colour::Blue));
        let mut log = MessageLog::default();
        log.push_line("Splash! Built a bridge.");

        let state = FrameState {
            title: "Tidekeep",
            pacing: Pacing::Hybrid { idle_ms: 650 },
            cell_width: CellWidth::Two,
            layout,
            tick: 3,
            seed: 0x2a,
            paused: false,
            reload_flash: false,
            bounds,
            terrain: &terrain,
            ecs: &ecs,
            palette: &palette,
            camera_center: cam.center,
            hud_status: "GEMS 1/5",
            log: &log,
            inspector_open: false,
            inspector_selected: 0,
            error_overlay: None,
            terminal_too_small: None,
        };

        let mut display = Display::new(layout.frame_width, layout.frame_height);
        compose_frame(&mut display, &state);

        // No default white backgrounds anywhere in gutters or log rows.
        assert_eq!(display.pixel(1, 1).unwrap().background(), Colour::Black);
        assert_eq!(display.pixel(76, 1).unwrap().background(), Colour::Black);
        assert_eq!(display.pixel(10, 20).unwrap().background(), Colour::Black);

        // Player (layer 10) renders over Gem (layer 2) at center viewport cell (rx=18, ry=8).
        let left_cell = display.pixel(38, 9).unwrap();
        let right_cell = display.pixel(39, 9).unwrap();
        assert_eq!(left_cell.character(), '@');
        assert_eq!(right_cell.character(), ' ');
        assert_eq!(left_cell.background(), Colour::Blue);
        assert_eq!(right_cell.background(), Colour::Blue);
        assert_eq!(left_cell.colour(), Colour::BrightYellow);
    }

    #[test]
    fn renders_side_drawer_on_wide_terminal_and_modal_on_narrow_terminal() {
        let mut ecs = EcsWorld::new();
        let crab = ecs.spawn();
        ecs.set_name(crab, "crab1");
        ecs.set_position(crab, Coord::new(2, 1));
        ecs.set_sprite(
            crab,
            Sprite {
                tile: "C".into(),
                layer: 6,
            },
        );
        ecs.set_custom(crab, "crab", ScriptValue::Bool(true));

        let mut palette = HashMap::new();
        palette.insert("C".into(), TileStyle::from_spec("><", Colour::BrightRed, None));
        let log = MessageLog::default();
        let cam = Camera::default();

        // Wide 120x34 terminal opens docked drawer with top tee junction.
        let wide_layout = FrameLayout::compute(Some((120, 34)), CellWidth::Two, true).unwrap();
        let wide_terrain = empty_terrain(
            wide_layout.viewport_cols,
            wide_layout.viewport_rows,
            Some(Colour::Black),
        );
        let wide_state = FrameState {
            title: "Tidekeep",
            pacing: Pacing::Hybrid { idle_ms: 650 },
            cell_width: CellWidth::Two,
            layout: wide_layout,
            tick: 1,
            seed: 42,
            paused: false,
            reload_flash: false,
            bounds: cam.bounds(wide_layout.viewport_cols, wide_layout.viewport_rows),
            terrain: &wide_terrain,
            ecs: &ecs,
            palette: &palette,
            camera_center: cam.center,
            hud_status: "",
            log: &log,
            inspector_open: true,
            inspector_selected: 0,
            error_overlay: None,
            terminal_too_small: None,
        };
        let mut wide_display = Display::new(wide_layout.frame_width, wide_layout.frame_height);
        compose_frame(&mut wide_display, &wide_state);
        let sep_x = wide_layout.map_width() as isize - 1;
        assert_eq!(wide_display.pixel(sep_x, 0).unwrap().character(), '┬');
        assert!(wide_display.render().contains("ECS Inspector"));
        assert!(wide_display.render().contains("crab = true"));

        // Narrow 80x24 terminal renders inspector inside a modal container.
        let narrow_layout = FrameLayout::compute(Some((80, 24)), CellWidth::Two, true).unwrap();
        let narrow_terrain = empty_terrain(
            narrow_layout.viewport_cols,
            narrow_layout.viewport_rows,
            Some(Colour::Black),
        );
        let narrow_state = FrameState {
            layout: narrow_layout,
            bounds: cam.bounds(narrow_layout.viewport_cols, narrow_layout.viewport_rows),
            terrain: &narrow_terrain,
            ..wide_state
        };
        let mut narrow_display =
            Display::new(narrow_layout.frame_width, narrow_layout.frame_height);
        compose_frame(&mut narrow_display, &narrow_state);
        let out = narrow_display.render();
        assert!(out.contains("ECS Inspector"));
        assert!(out.contains("crab = true"));

        // Underlying terrain glyphs must not bleed through modal padding or short lines.
        let (box_x, box_y, box_w, box_h) = (3, 4, 72, 10);
        assert_eq!(narrow_display.pixel(box_x, box_y).unwrap().character(), '┌');
        for y in (box_y + 1)..(box_y + box_h - 1) {
            let margin = narrow_display.pixel(box_x + 1, y).unwrap();
            assert_eq!((margin.character(), margin.background()), (' ', Colour::Black));
            let tail = narrow_display.pixel(box_x + box_w - 2, y).unwrap();
            assert_eq!((tail.character(), tail.background()), (' ', Colour::Black));
            for x in (box_x + 1)..(box_x + box_w - 1) {
                let p = narrow_display.pixel(x, y).unwrap();
                assert_eq!(p.background(), Colour::Black);
                assert!(!matches!(p.character(), '≈' | '~'));
            }
        }
    }

    #[test]
    fn contrast_guard_and_error_overlay_render_cleanly() {
        assert_eq!(
            contrast_safe_fg(Colour::BrightYellow, Colour::BrightYellow),
            Colour::Black
        );
        assert_eq!(contrast_safe_fg(Colour::Blue, Colour::Blue), Colour::BrightWhite);

        let palette = HashMap::new();
        let ecs = EcsWorld::new();
        let cam = Camera::default();
        let layout = FrameLayout::default_for(CellWidth::One);
        let bounds = cam.bounds(layout.viewport_cols, layout.viewport_rows);
        let terrain = empty_terrain(layout.viewport_cols, layout.viewport_rows, Some(Colour::Black));
        let log = MessageLog::default();

        let state = FrameState {
            title: "Life",
            pacing: Pacing::Turn,
            cell_width: CellWidth::One,
            layout,
            tick: 0,
            seed: 1,
            paused: true,
            reload_flash: false,
            bounds,
            terrain: &terrain,
            ecs: &ecs,
            palette: &palette,
            camera_center: cam.center,
            hud_status: "",
            log: &log,
            inspector_open: false,
            inspector_selected: 0,
            error_overlay: Some("world.lua:5: unexpected symbol near '='"),
            terminal_too_small: None,
        };

        let mut display = Display::new(layout.frame_width, layout.frame_height);
        compose_frame(&mut display, &state);

        assert_eq!(display.size(), (layout.frame_width, layout.frame_height));
        assert!(display.render().contains("Lua script error"));

        let (box_x, box_y, box_w, box_h) = (3, 2, 72, 5);
        assert_eq!(display.pixel(box_x, box_y).unwrap().character(), '┌');
        for y in (box_y + 1)..(box_y + box_h - 1) {
            let margin = display.pixel(box_x + 1, y).unwrap();
            assert_eq!((margin.character(), margin.background()), (' ', Colour::Black));
            for x in (box_x + 1)..(box_x + box_w - 1) {
                let p = display.pixel(x, y).unwrap();
                assert_eq!(p.background(), Colour::Black);
                assert_ne!(p.character(), '≈');
            }
        }
    }

    #[test]
    fn stacked_opaque_entity_propagates_background_to_transparent_actor() {
        let mut palette = HashMap::new();
        palette.insert(
            "=".to_string(),
            TileStyle::from_spec("==", Colour::BrightYellow, Some(Colour::Green)),
        );
        palette.insert(
            "@".to_string(),
            TileStyle::from_spec("@ ", Colour::BrightYellow, None),
        );

        let mut ecs = EcsWorld::new();
        let bridge = ecs.spawn();
        ecs.set_position(bridge, Coord::new(0, 0));
        ecs.set_sprite(
            bridge,
            Sprite {
                tile: "=".into(),
                layer: 1,
            },
        );

        let player = ecs.spawn();
        ecs.set_position(player, Coord::new(0, 0));
        ecs.set_sprite(
            player,
            Sprite {
                tile: "@".into(),
                layer: 10,
            },
        );
        ecs.set_player(player, true);

        let layout = FrameLayout::default_for(CellWidth::Two);
        let cam = Camera::default();
        let bounds = cam.bounds(layout.viewport_cols, layout.viewport_rows);
        let terrain = empty_terrain(layout.viewport_cols, layout.viewport_rows, Some(Colour::Blue));
        let log = MessageLog::default();

        let state = FrameState {
            title: "Tidekeep",
            pacing: Pacing::Hybrid { idle_ms: 650 },
            cell_width: CellWidth::Two,
            layout,
            tick: 1,
            seed: 42,
            paused: false,
            reload_flash: false,
            bounds,
            terrain: &terrain,
            ecs: &ecs,
            palette: &palette,
            camera_center: cam.center,
            hud_status: "",
            log: &log,
            inspector_open: false,
            inspector_selected: 0,
            error_overlay: None,
            terminal_too_small: None,
        };

        let mut display = Display::new(layout.frame_width, layout.frame_height);
        compose_frame(&mut display, &state);

        // Player at world (0,0) inherits Green from the layer-1 bridge instead of Blue terrain.
        let left = display.pixel(38, 9).unwrap();
        let right = display.pixel(39, 9).unwrap();
        assert_eq!(left.character(), '@');
        assert_eq!(right.character(), ' ');
        assert_eq!(left.background(), Colour::Green);
        assert_eq!(right.background(), Colour::Green);
        assert_eq!(left.colour(), Colour::BrightYellow);
    }
}
