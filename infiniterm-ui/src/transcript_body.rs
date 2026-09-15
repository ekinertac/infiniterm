//! The transcript card: an agent session's turns, read from its JSONL.
//! Port of `TranscriptCard.svelte`; parsing and formatting are core's
//! `transcript.rs`, this is the drawing and the keys.
//!
//! The turns on the left, one per prompt or answer, and the chosen turn on
//! the right with its text and every tool call under it, results folded
//! behind their summary line until Enter opens them. The list follows the
//! end while the session runs: the file is re-read whenever its mtime
//! moves (2 s poll, like the editor's reload), and if you were on the last
//! turn you stay on the last turn. `card.path` is the session file, which
//! arrived on the agent card with its first hook event.
use crate::body::{BodyAction, CardBody};
use crate::terminal_body::Metrics;
use gpui::{fill, font, point, px, size, App, Bounds, Hsla, Keystroke, Pixels, Window};
use infiniterm_core::files::file_mtime;
use infiniterm_core::grid::{Point, Size};
use infiniterm_core::transcript::{
    tool_line, transcript_read, turn_preview, turn_time, Role, Turn,
};
use infiniterm_editor::wrap::wrap_line;

const PAD_X: f64 = 8.;
const PAD_Y: f64 = 6.;
const DISK_POLL_MS: f64 = 2000.;
/// The reference's list rows read at line-height 1.5.
const LINE: f64 = 1.5;
/// Columns reserved for the "you"/"claude" role label in a list row.
const WHO_COL_WIDTH_CELLS: f64 = 7.;
/// Columns reserved for a list row's timestamp, right-aligned.
const TIME_COL_WIDTH_CELLS: f64 = 6.;
/// Columns the row's preview text gives up to the role label, the
/// timestamp and the gaps around them.
const PREVIEW_RESERVED_COLS: usize = 14;
/// Page up/down moves the detail pane by this many lines.
const PAGE_SCROLL_LINES: usize = 10;

/// The card's colours, resolved by `editors.rs` from the theme and chrome.
#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptColors {
    pub background: Hsla,
    pub foreground: Hsla,
    pub faint: Hsla,
    pub user: Hsla,
    pub assistant: Hsla,
    pub sel_bg: Hsla,
    pub sel_fg: Hsla,
}

pub struct TranscriptBody {
    pub path: Option<String>,
    turns: Vec<Turn>,
    cursor: usize,
    list_scroll: usize,
    detail_scroll: usize,
    /// Whether the current turn's tool inputs and results show.
    expanded: bool,
    error: Option<String>,
    mtime: Option<u64>,
    last_check: f64,
    pub sidebar_w: f64,
    pub sidebar_top: bool,
    pub colors: TranscriptColors,
    pub metrics: Metrics,
    pub inactive_dim: f64,
    world: Size,
    dirty: bool,
}

impl TranscriptBody {
    pub fn new(path: Option<String>, metrics: &Metrics, world: Size) -> TranscriptBody {
        let c = |v: u32| -> Hsla { gpui::rgb(v).into() };
        TranscriptBody {
            path,
            turns: vec![],
            cursor: 0,
            list_scroll: 0,
            detail_scroll: 0,
            expanded: false,
            error: None,
            mtime: None,
            last_check: 0.,
            sidebar_w: 0.,
            sidebar_top: false,
            colors: TranscriptColors {
                background: c(0x0e101a),
                foreground: c(0xb9c4d2),
                faint: c(0x5a6472),
                user: c(0xe5c07b),
                assistant: c(0x61afef),
                sel_bg: c(0xe39500),
                sel_fg: c(0x0e101a),
            },
            metrics: Metrics {
                family: metrics.family.clone(),
                font_px: metrics.font_px,
                line_height: metrics.line_height,
                cell_w: metrics.cell_w,
            },
            inactive_dim: crate::chrome::INACTIVE_DIM_DEFAULT,
            world,
            dirty: true,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn line_h(&self) -> f64 {
        self.metrics.font_px * LINE
    }

    fn load(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let at_end = self.cursor + 1 >= self.turns.len();
        match transcript_read(&path) {
            Ok(next) => {
                let n = next.len();
                self.turns = next;
                if at_end || self.cursor >= n {
                    self.cursor = n.saturating_sub(1);
                    self.detail_scroll = 0;
                }
                self.error = None;
            }
            Err(e) => {
                eprintln!("[infiniterm/warn] transcript {path}: {e}");
                self.error = Some(e);
            }
        }
        self.dirty = true;
    }

    /// Re-reads the file when its mtime moved; every two seconds.
    pub fn idle(&mut self, now: f64) {
        if now - self.last_check < DISK_POLL_MS {
            return;
        }
        self.last_check = now;
        let Some(path) = self.path.clone() else {
            return;
        };
        let m = file_mtime(&path);
        if m.is_none() || m == self.mtime {
            return;
        }
        self.mtime = m;
        self.load();
    }

    fn set_cursor(&mut self, i: usize) {
        let i = i.min(self.turns.len().saturating_sub(1));
        if i != self.cursor {
            self.cursor = i;
            self.detail_scroll = 0;
            self.expanded = false;
        }
        self.dirty = true;
    }

    fn list_area(&self, world: Size) -> (Point, Size) {
        if self.sidebar_top {
            (
                Point { x: 0., y: 0. },
                Size {
                    w: world.w,
                    h: self.sidebar_w,
                },
            )
        } else {
            (
                Point { x: 0., y: 0. },
                Size {
                    w: self.sidebar_w,
                    h: world.h,
                },
            )
        }
    }

    fn detail_area(&self, world: Size) -> (Point, Size) {
        if self.sidebar_top {
            (
                Point {
                    x: 0.,
                    y: self.sidebar_w,
                },
                Size {
                    w: world.w,
                    h: world.h - self.sidebar_w,
                },
            )
        } else {
            (
                Point {
                    x: self.sidebar_w,
                    y: 0.,
                },
                Size {
                    w: world.w - self.sidebar_w,
                    h: world.h,
                },
            )
        }
    }

    /// The detail as lines to draw: the head, the text wrapped, each tool's
    /// summary and, when expanded, its input and result.
    fn detail_lines(&self, cols: usize) -> Vec<(String, Hsla)> {
        let Some(t) = self.turns.get(self.cursor) else {
            return vec![];
        };
        let who = match t.role {
            Role::User => ("you", self.colors.user),
            Role::Assistant => ("claude", self.colors.assistant),
        };
        let mut out = vec![(format!("{}  {}", who.0, turn_time(&t.at)), who.1)];
        out.push((String::new(), self.colors.foreground));
        for line in t.text.lines() {
            for (a, b) in wrap_line(line, cols) {
                out.push((
                    line.chars().skip(a).take(b - a).collect(),
                    self.colors.foreground,
                ));
            }
        }
        for tool in &t.tools {
            out.push((String::new(), self.colors.foreground));
            let marker = if self.expanded { "▾ " } else { "▸ " };
            out.push((format!("{marker}{}", tool_line(tool)), self.colors.faint));
            if self.expanded {
                for line in tool
                    .input
                    .lines()
                    .chain(["".into()].iter().map(String::as_str))
                {
                    for (a, b) in wrap_line(line, cols.saturating_sub(2)) {
                        out.push((
                            format!("  {}", line.chars().skip(a).take(b - a).collect::<String>()),
                            self.colors.faint,
                        ));
                    }
                }
                for line in tool.result.lines() {
                    for (a, b) in wrap_line(line, cols.saturating_sub(2)) {
                        out.push((
                            format!("  {}", line.chars().skip(a).take(b - a).collect::<String>()),
                            self.colors.foreground,
                        ));
                    }
                }
            }
        }
        out
    }
}

impl CardBody for TranscriptBody {
    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        scale: f64,
        focused: bool,
        _now: f64,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.dirty = false;
        let world = Size {
            w: f32::from(bounds.size.width) as f64 / scale,
            h: f32::from(bounds.size.height) as f64 / scale,
        };
        self.world = world;
        let s = |v: f64| px((v * scale) as f32);
        window.paint_quad(fill(bounds, self.colors.background));
        let font_size = px((self.metrics.font_px * scale) as f32);
        if font_size < px(crate::chrome::LEGIBLE_FONT_PX as f32) {
            return;
        }
        let line_h = s(self.line_h());
        let f = font(self.metrics.family.clone());
        let pad = s(PAD_X);
        // The list.
        let (lo, ls) = self.list_area(world);
        let list = Bounds::new(
            point(bounds.origin.x + s(lo.x), bounds.origin.y + s(lo.y)),
            size(s(ls.w), s(ls.h)),
        );
        let rows_visible = ((ls.h - PAD_Y * 2.) / self.line_h()).floor().max(1.) as usize;
        if self.cursor < self.list_scroll {
            self.list_scroll = self.cursor;
        } else if self.cursor >= self.list_scroll + rows_visible {
            self.list_scroll = self.cursor + 1 - rows_visible;
        }
        let who_w = px((self.metrics.cell_w * WHO_COL_WIDTH_CELLS * scale) as f32);
        let time_w = px((self.metrics.cell_w * TIME_COL_WIDTH_CELLS * scale) as f32);
        let mut y = list.origin.y + s(PAD_Y);
        if self.turns.is_empty() {
            let text = self.error.clone().unwrap_or_else(|| "no turns yet".into());
            let l = crate::text::shape(window, &text, font_size, &f, self.colors.faint);
            let _ = l.paint(point(list.origin.x + pad, y), line_h, window, cx);
        }
        let preview_cols = (((ls.w - PAD_X * 2.) / self.metrics.cell_w) as usize)
            .saturating_sub(PREVIEW_RESERVED_COLS);
        for (i, t) in self
            .turns
            .iter()
            .enumerate()
            .skip(self.list_scroll)
            .take(rows_visible)
        {
            let is_cursor = i == self.cursor;
            let (mut who_c, mut fg, mut time_c) = (
                match t.role {
                    Role::User => self.colors.user,
                    Role::Assistant => self.colors.assistant,
                },
                self.colors.foreground,
                self.colors.faint,
            );
            if is_cursor {
                let row = Bounds::new(point(list.origin.x, y), size(list.size.width, line_h));
                if focused {
                    window.paint_quad(fill(row, self.colors.sel_bg));
                    who_c = self.colors.sel_fg;
                    fg = self.colors.sel_fg;
                    time_c = self.colors.sel_fg;
                } else {
                    window.paint_quad(fill(
                        row,
                        crate::chrome::with_alpha(
                            self.colors.foreground,
                            crate::chrome::TREE_CURSOR_UNFOCUSED_ALPHA,
                        ),
                    ));
                }
            }
            let who = if t.role == Role::User {
                "you"
            } else {
                "claude"
            };
            let l = crate::text::shape(window, who, font_size, &f, who_c);
            let _ = l.paint(point(list.origin.x + pad, y), line_h, window, cx);
            let l = crate::text::shape(
                window,
                &turn_preview(t, preview_cols.max(4)),
                font_size,
                &f,
                fg,
            );
            let _ = l.paint(point(list.origin.x + pad + who_w, y), line_h, window, cx);
            let time = turn_time(&t.at);
            let l = crate::text::shape(window, &time, font_size, &f, time_c);
            let _ = l.paint(
                point(
                    list.origin.x + list.size.width - pad - l.width.min(time_w),
                    y,
                ),
                line_h,
                window,
                cx,
            );
            y += line_h;
        }
        // The edge between list and detail.
        let hairline = px(crate::chrome::HAIRLINE_PX as f32);
        let edge = if self.sidebar_top {
            Bounds::new(
                point(list.origin.x, list.origin.y + list.size.height - hairline),
                size(list.size.width, hairline),
            )
        } else {
            Bounds::new(
                point(list.origin.x + list.size.width - hairline, list.origin.y),
                size(hairline, list.size.height),
            )
        };
        window.paint_quad(fill(
            edge,
            crate::chrome::with_alpha(self.colors.faint, crate::chrome::HAIRLINE_ALPHA),
        ));
        // The detail.
        let (d_o, d_s) = self.detail_area(world);
        let detail = Bounds::new(
            point(bounds.origin.x + s(d_o.x), bounds.origin.y + s(d_o.y)),
            size(s(d_s.w), s(d_s.h)),
        );
        let cols = (((d_s.w - PAD_X * 2.) / self.metrics.cell_w).floor()).max(4.) as usize;
        let lines = self.detail_lines(cols);
        let visible = ((d_s.h - PAD_Y * 2.) / self.line_h()).floor().max(1.) as usize;
        let max_scroll = lines.len().saturating_sub(visible);
        self.detail_scroll = self.detail_scroll.min(max_scroll);
        let mut y = detail.origin.y + s(PAD_Y);
        for (text, color) in lines.iter().skip(self.detail_scroll).take(visible) {
            if !text.is_empty() {
                let l = crate::text::shape(window, text, font_size, &f, *color);
                let _ = l.paint(point(detail.origin.x + pad, y), line_h, window, cx);
            }
            y += line_h;
        }
        if !focused && self.inactive_dim > 0. {
            window.paint_quad(fill(
                bounds,
                crate::chrome::with_alpha(self.colors.background, self.inactive_dim as f32),
            ));
        }
    }

    fn resized(&mut self, world: Size) {
        self.world = world;
        self.dirty = true;
    }

    fn key(&mut self, k: &Keystroke, _now: f64, _cx: &mut App) -> BodyAction {
        if k.modifiers.platform || k.modifiers.alt || k.modifiers.control {
            return BodyAction::None;
        }
        match k.key.as_str() {
            "down" => self.set_cursor(self.cursor + 1),
            "up" => self.set_cursor(self.cursor.saturating_sub(1)),
            "home" => self.set_cursor(0),
            "end" => self.set_cursor(self.turns.len().saturating_sub(1)),
            "enter" | "space" => {
                self.expanded = !self.expanded;
                self.dirty = true;
            }
            "pagedown" => {
                self.detail_scroll += PAGE_SCROLL_LINES;
                self.dirty = true;
            }
            "pageup" => {
                self.detail_scroll = self.detail_scroll.saturating_sub(PAGE_SCROLL_LINES);
                self.dirty = true;
            }
            _ => {}
        }
        BodyAction::None
    }

    fn mouse_down(
        &mut self,
        local: Point,
        button: gpui::MouseButton,
        _modifiers: &gpui::Modifiers,
        _clicks: usize,
    ) -> BodyAction {
        if button != gpui::MouseButton::Left {
            return BodyAction::None;
        }
        let (lo, ls) = self.list_area(self.world);
        let in_list =
            local.x >= lo.x && local.x < lo.x + ls.w && local.y >= lo.y && local.y < lo.y + ls.h;
        if in_list {
            let row = ((local.y - lo.y - PAD_Y) / self.line_h()).floor().max(0.) as usize;
            self.set_cursor(row + self.list_scroll);
        } else {
            // A click on the detail: fold or unfold the tools.
            self.expanded = !self.expanded;
            self.dirty = true;
        }
        BodyAction::None
    }

    fn wheel(&mut self, local: Point, _dx: f64, dy: f64, _modifiers: &gpui::Modifiers) {
        let lines = (dy / self.line_h() * crate::chrome::WHEEL_LINES_PER_TICK).round() as i64;
        if lines == 0 {
            return;
        }
        let (lo, ls) = self.list_area(self.world);
        let in_list =
            local.x >= lo.x && local.x < lo.x + ls.w && local.y >= lo.y && local.y < lo.y + ls.h;
        if in_list {
            let max = self.turns.len().saturating_sub(1) as i64;
            self.list_scroll = (self.list_scroll as i64 - lines).clamp(0, max) as usize;
        } else {
            self.detail_scroll = (self.detail_scroll as i64 - lines).max(0) as usize;
        }
        self.dirty = true;
    }

    fn wants_frame(&self, _now: f64) -> bool {
        self.dirty
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
