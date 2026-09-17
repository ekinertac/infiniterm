//! The terminal inside a card: one grid bound to one PTY, painted as shaped
//! lines at the zoomed font size. Port of `TerminalCard.svelte` on the
//! spike's painter (`spikes/canvas/src/terminal.rs`).
//!
//! Owns the emulator and the encoders and nothing else: the frame, the
//! handles and the focus ring are the card frame's, zoom is the canvas's.
//! Bytes arrive through `feed` from the scheduler (one budget per frame,
//! the term-zoom spike's shape); keys the app did not claim are encoded
//! here (`infiniterm-term::keys`); the mouse goes to the program when it
//! asked for it and scrolls the scrollback otherwise. URLs and confirmed
//! paths in the output underline under a Cmd+hover and open with Cmd+click, the same
//! path as `ift <path>`. A plain click only focuses the card.
//!
//! Size follows the card's rect in world units at scale 1: the grid is
//! however many cells fit, and the PTY is told, so programs reflow. Zooming
//! changes nothing about the grid, only the pixel size it is drawn at.
//!
//! A drag selects text (two clicks a word, three a line) and Cmd+C copies
//! it; the selection lives in the grid, on the text, so it survives a
//! scroll. The cursor blinks only in the focused card and only when the
//! settings say so; an unfocused card shows a hollow cursor under a scrim
//! the colour of its ground, which is the reference's dimming. A shell that
//! could not start leaves the card showing why, and a click on that retries.
use crate::body::{BodyAction, CardBody};
use gpui::{
    fill, font, outline, point, px, size, App, Bounds, ClipboardItem, FontStyle, FontWeight, Hsla,
    Keystroke, Pixels, SharedString, TextRun, UnderlineStyle, Window,
};
use infiniterm_core::backend::PaneId;
use infiniterm_core::grid::{Point, Size};
use infiniterm_core::ift::{open_plan, url_plan, PathKind};
use infiniterm_core::links::{find_links, Found, LinkKind};
use infiniterm_core::links_fs::path_kinds;
use infiniterm_term::grid::{CursorKind, Frame, Grid, SelectKind, TermEvent, SPACER};
use infiniterm_term::keys::{encode, encode_with, paste, Key};
use infiniterm_term::mouse::{self, Mods, MouseButton};
use infiniterm_term::palette::Palette;

/// Inset from the card's edge to the first cell, in world units.
const PAD: f64 = 6.;

thread_local! {
    /// Milliseconds spent this second in frame building, link scanning,
    /// shaping and glyph painting, summed over every body; `paint.rs`
    /// reports and resets it under `INFINITERM_KEYLOG`.
    static TIMING: std::cell::Cell<[f64; 4]> = const { std::cell::Cell::new([0.; 4]) };
}

pub fn timing_take() -> [f64; 4] {
    TIMING.with(|t| t.replace([0.; 4]))
}

fn timing_add(slot: usize, since: std::time::Instant) {
    TIMING.with(|t| {
        let mut v = t.get();
        v[slot] += since.elapsed().as_secs_f64() * 1000.;
        t.set(v);
    });
}

/// Half a blink, xterm.js's interval. The cursor is solid for this long
/// after any key, so it never blinks away while you type.
const BLINK_MS: f64 = 600.;

/// The spawn-error text's font size.
const ERROR_FONT_PX: f64 = 12.;
/// The spawn-error block's inset from the card's edge.
const ERROR_PAD_PX: f64 = 16.;
/// The spawn-error lines are spaced looser than the terminal's own rows.
const ERROR_LINE_HEIGHT_RATIO: f64 = 1.5;
/// Half a line between the spawn-error's three blocks (the message, the
/// error, and the retry hint).
const ERROR_BLOCK_GAP_RATIO: f64 = 0.5;
/// The beam and underline cursors' stroke: thicker than a hairline so they
/// still read as a cursor rather than a line in the grid.
const TERM_CURSOR_STROKE_PX: f32 = 2.;
/// A link's underline sits just clear of the glyphs' descenders.
const LINK_UNDERLINE_OFFSET_PX: f32 = 1.5;
/// Dim text (SGR faint) is drawn at this alpha rather than a different
/// weight, since the grid has no font-weight axis for dimness.
const TERM_DIM_ALPHA: f32 = 0.6;
/// A single wheel tick never sends more than this many scrolled lines to a
/// mouse-aware program or the alternate screen: a fast trackpad fling must
/// not flood it.
const WHEEL_MAX_LINES_PER_EVENT: u32 = 10;

pub struct TerminalBody {
    pub pane: Option<PaneId>,
    pub grid: Grid,
    pub palette: Palette,
    pub font_family: String,
    pub weight: FontWeight,
    pub bold_weight: FontWeight,
    pub font_px: f64,
    pub line_height: f64,
    /// The cell width at `font_px`, measured once from the font.
    pub cell_w: f64,
    pub cwd: String,
    /// The card's directory, for resolving paths in the output.
    pub error: Option<String>,
    /// Bytes to write to the pty: the ui drains them after each event.
    pub outgoing: Vec<Vec<u8>>,
    /// What the EMULATOR answered a program with: a colour query, device
    /// attributes, a cursor report. Kept apart from `outgoing`, which is
    /// what the person typed, because under tmux these must not be sent at
    /// all: tmux is the terminal from the program's point of view and
    /// answers them itself, and a second answer arrives as keystrokes.
    pub replies: Vec<Vec<u8>>,
    /// Output arrived since the last paint.
    pub dirty: bool,
    /// The wheel's fraction of a line, carried between events.
    wheel_carry: crate::chrome::WheelCarry,
    pub title: Option<String>,
    /// OSC 52: text a program put on the clipboard, taken by the ui.
    pub clipboard_out: Option<String>,
    /// `terminal.cursorBlink`, `ui.inactiveDim`; the ui keeps them current.
    pub blink: bool,
    pub inactive_dim: f64,
    /// When the blink clock last restarted (a key), and the phase and focus
    /// of the last paint, so `wants_frame` can say when the next flip is due.
    blink_epoch: f64,
    painted_phase: bool,
    painted_focused: bool,
    /// A drag selecting text.
    selecting: bool,
    /// The rows as last built; `Grid::update_frame` touches only the
    /// damaged ones.
    frame: Frame,
    /// The last frame's links, per row, with whether the filesystem said yes.
    links: Vec<Vec<(Found, Option<PathKind>)>>,
    link_texts: Vec<String>,
    hover: Option<(usize, usize)>,
    /// A button held for a drag the program is following.
    dragging: Option<MouseButton>,
    scale: f64,
    cols: usize,
    rows: usize,
    /// Shaped chunks per row, keyed by what they were shaped from, so a row
    /// that did not change is not shaped again. htop repaints every second;
    /// the other 39 rows of the other 24 cards do not.
    shaped: Vec<(u64, Vec<(usize, gpui::ShapedLine)>)>,
}

/// How many cells one shaped chunk covers. Glyph advances at a fractional
/// pixel size round per glyph, so a 200-cell row shaped as one line drifts
/// tens of pixels from the grid by its end; positioning every chunk at its
/// cell's x bounds the drift to a chunk.
const CHUNK: usize = 24;

#[derive(Clone, Debug, PartialEq)]
pub struct Metrics {
    pub family: String,
    pub font_px: f64,
    pub line_height: f64,
    pub cell_w: f64,
    /// `terminal.fontWeight` and `fontWeightBold`, resolved.
    pub weight: FontWeight,
    pub bold_weight: FontWeight,
}

impl Metrics {
    /// The font at the configured weight.
    pub fn font(&self) -> gpui::Font {
        let mut f = font(self.family.clone());
        f.weight = self.weight;
        f
    }
}

/// `"normal"`, `"bold"`, or a number 100 to 900, as CSS spells weights.
pub fn weight_of(s: &str) -> FontWeight {
    match s.trim() {
        "bold" => FontWeight::BOLD,
        "normal" | "" => FontWeight::NORMAL,
        n => n
            .parse::<f32>()
            .ok()
            .filter(|w| (100. ..=900.).contains(w))
            .map(FontWeight)
            .unwrap_or(FontWeight::NORMAL),
    }
}

impl TerminalBody {
    pub fn new(
        metrics: &Metrics,
        palette: Palette,
        world: Size,
        scrollback: usize,
        cwd: String,
    ) -> TerminalBody {
        let (cols, rows) =
            Self::cells_for(world, metrics.cell_w, metrics.font_px * metrics.line_height);
        TerminalBody {
            pane: None,
            grid: Grid::new(cols, rows, scrollback),
            palette,
            font_family: metrics.family.clone(),
            weight: metrics.weight,
            bold_weight: metrics.bold_weight,
            font_px: metrics.font_px,
            line_height: metrics.line_height,
            cell_w: metrics.cell_w,
            cwd,
            error: None,
            outgoing: vec![],
            replies: vec![],
            dirty: true,
            wheel_carry: crate::chrome::WheelCarry::default(),
            title: None,
            clipboard_out: None,
            blink: true,
            inactive_dim: crate::chrome::INACTIVE_DIM_DEFAULT,
            blink_epoch: 0.,
            painted_phase: true,
            painted_focused: false,
            selecting: false,
            frame: Frame::default(),
            links: vec![],
            link_texts: vec![],
            hover: None,
            dragging: None,
            scale: 1.,
            cols,
            rows,
            shaped: vec![],
        }
    }

    fn cells_for(world: Size, cell_w: f64, line_h: f64) -> (usize, usize) {
        let cols = ((world.w - PAD * 2.) / cell_w).floor().max(2.) as usize;
        let rows = ((world.h - PAD * 2.) / line_h).floor().max(1.) as usize;
        (cols, rows)
    }

    pub fn cols(&self) -> u16 {
        self.cols as u16
    }

    pub fn rows(&self) -> u16 {
        self.rows as u16
    }

    /// One budget's worth of output.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.grid.advance(bytes);
        self.dirty = true;
        self.drain_events();
    }

    /// History from a daemon's ring, on adopt. The bytes go to the grid
    /// like live output; the EVENTS do not. A query in the history (the
    /// kitty keyboard question, a colour request, a cursor report) was
    /// answered live, back when the program asked it, by whoever was
    /// attached then. Answering it again now sends the reply into the
    /// program's input as if it had just asked, which is the same mistake
    /// tmux made by answering beside us (bug 9). An OSC 52 in the history
    /// would put months-old text on the clipboard. The title is kept: the
    /// last one set is still the program's name for itself.
    pub fn feed_replay(&mut self, bytes: &[u8]) {
        self.grid.advance(bytes);
        self.dirty = true;
        for event in self.grid.take_events() {
            if let TermEvent::Title(t) = event {
                self.title = Some(t);
            }
        }
    }

    fn drain_events(&mut self) {
        for event in self.grid.take_events() {
            match event {
                TermEvent::Write(s) => self.replies.push(s.into_bytes()),
                TermEvent::Title(t) => self.title = Some(t),
                // The reference does nothing on a bell either.
                TermEvent::Bell => {}
                TermEvent::Clipboard(text) => self.clipboard_out = Some(text),
            }
        }
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
        self.grid.set_palette_changed();
        self.dirty = true;
    }

    /// Metrics changed (a settings edit): the grid is re-counted, which the
    /// reference's `refit` rule insists on, and the caller resizes the PTY.
    pub fn set_metrics(&mut self, metrics: &Metrics, world: Size) -> bool {
        self.font_family = metrics.family.clone();
        self.weight = metrics.weight;
        self.bold_weight = metrics.bold_weight;
        self.font_px = metrics.font_px;
        self.line_height = metrics.line_height;
        self.cell_w = metrics.cell_w;
        self.refit(world)
    }

    /// The card's rect changed: re-count the grid. True when the PTY needs
    /// telling.
    pub fn refit_to(&mut self, world: Size) -> bool {
        self.refit(world)
    }

    fn refit(&mut self, world: Size) -> bool {
        let (cols, rows) = Self::cells_for(world, self.cell_w, self.font_px * self.line_height);
        if cols == self.cols && rows == self.rows {
            return false;
        }
        self.cols = cols;
        self.rows = rows;
        self.grid.resize(cols, rows)
    }

    fn write(&mut self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            self.outgoing.push(bytes);
        }
    }

    /// Text put into the shell as if pasted: bracketed when the program
    /// asked for it, so a path with a newline in its name cannot run as a
    /// command in a shell that reads pastes as typing.
    pub fn paste_text(&mut self, text: &str) {
        let bracketed = self.grid.bracketed_paste();
        self.write(paste(text, bracketed));
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Whether the cursor is on at `now`: always, unless it blinks.
    fn blink_on(&self, now: f64) -> bool {
        !self.blink || (((now - self.blink_epoch) / BLINK_MS) as u64).is_multiple_of(2)
    }

    /// The pointer is in the right half of its cell: a drag ending there
    /// takes the character.
    fn right_half(&self, local: Point) -> bool {
        ((local.x - PAD) / self.cell_w).fract() > 0.5
    }

    fn cell_at(&self, local: Point) -> (usize, usize) {
        let col = ((local.x - PAD) / self.cell_w).floor().max(0.) as usize;
        let row = ((local.y - PAD) / (self.font_px * self.line_height))
            .floor()
            .max(0.) as usize;
        (
            col.min(self.cols.saturating_sub(1)),
            row.min(self.rows.saturating_sub(1)),
        )
    }

    fn mods(m: &gpui::Modifiers) -> Mods {
        Mods {
            shift: m.shift,
            alt: m.alt,
            ctrl: m.control,
        }
    }

    /// The link under a cell, if the row has one there.
    fn link_at(&self, col: usize, row: usize) -> Option<&(Found, Option<PathKind>)> {
        let text = self.link_texts.get(row)?;
        // Byte offset of the cell's character.
        let byte = text.char_indices().nth(col).map(|(i, _)| i)?;
        self.links
            .get(row)?
            .iter()
            .find(|(f, _)| f.start <= byte && byte < f.end)
    }

    /// Re-scans the rows that were rebuilt for links and asks the
    /// filesystem about the paths, so a version number is never underlined.
    /// Only rebuilt rows are looked at: trimming every row of every card
    /// each frame was 10 ms by itself. A row without a slash or a dot
    /// cannot hold a link and is not scanned.
    fn refresh_links(&mut self, frame: &Frame, rebuilt: &[usize]) {
        self.link_texts.resize(frame.rows.len(), String::new());
        self.links.resize(frame.rows.len(), vec![]);
        for &r in rebuilt {
            let row = &frame.rows[r];
            let line = row.text.trim_end();
            if line == self.link_texts[r] {
                continue;
            }
            self.link_texts[r].clear();
            self.link_texts[r].push_str(line);
            self.links[r] = if line.contains(['/', '.']) {
                Self::scan_links(line, &self.cwd)
            } else {
                vec![]
            };
        }
    }

    fn scan_links(line: &str, cwd: &str) -> Vec<(Found, Option<PathKind>)> {
        {
            {
                let found = find_links(line);
                let paths: Vec<&str> = found
                    .iter()
                    .filter(|f| f.kind == LinkKind::Path)
                    .map(|f| f.target.as_str())
                    .collect();
                let kinds = if paths.is_empty() {
                    vec![]
                } else {
                    path_kinds(cwd, &paths)
                };
                let mut ki = kinds.into_iter();
                found
                    .into_iter()
                    .filter_map(|f| match f.kind {
                        LinkKind::Url => Some((f, None)),
                        LinkKind::Path => match ki.next().flatten() {
                            Some(infiniterm_core::links_fs::PathKind::Dir) => {
                                Some((f, Some(PathKind::Directory)))
                            }
                            Some(infiniterm_core::links_fs::PathKind::File) => {
                                Some((f, Some(PathKind::File)))
                            }
                            None => None,
                        },
                    })
                    .collect()
            }
        }
    }

    fn resolve_against(&self, path: &str, home: &str) -> String {
        let home = home.trim_end_matches('/');
        if path.starts_with('/') {
            path.to_string()
        } else if path == "~" {
            home.to_string()
        } else if let Some(rest) = path.strip_prefix("~/") {
            format!("{home}/{rest}")
        } else {
            format!("{}/{path}", self.cwd.trim_end_matches('/'))
        }
    }
}

impl TerminalBody {
    /// The reference's `.scrim`: the card's ground over the text at the
    /// dim amount, so an unfocused card reads as further away.
    fn paint_scrim(&self, bounds: Bounds<Pixels>, focused: bool, window: &mut Window) {
        if focused || self.inactive_dim <= 0. {
            return;
        }
        window.paint_quad(fill(
            bounds,
            crate::chrome::with_alpha(rgb(self.palette.background), self.inactive_dim as f32),
        ));
    }

    /// "could not start a shell in <cwd>", the error, and that a click
    /// retries: the reference's `.spawn-error` block and its button.
    fn paint_error(
        &self,
        error: &str,
        bounds: Bounds<Pixels>,
        scale: f64,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.paint_quad(fill(bounds, gpui::rgb(0x1a0f0f)));
        let size_px = px((ERROR_FONT_PX * scale) as f32);
        if size_px < px(crate::chrome::LEGIBLE_FONT_PX as f32) {
            return;
        }
        let pad = px((ERROR_PAD_PX * scale) as f32);
        let line_h = size_px * ERROR_LINE_HEIGHT_RATIO as f32;
        let f = font(self.font_family.clone());
        let mut y = bounds.origin.y + pad;
        let lines = [
            (format!("could not start a shell in {}", self.cwd), 0xfca5a5),
            (error.to_string(), 0xf87171),
            ("click to retry".to_string(), 0xfca5a5),
        ];
        for (text, color) in lines {
            for part in text.split('\n') {
                let line = crate::text::shape(window, part, size_px, &f, gpui::rgb(color).into());
                let _ = line.paint(point(bounds.origin.x + pad, y), line_h, window, cx);
                y += line_h;
            }
            y += line_h * ERROR_BLOCK_GAP_RATIO as f32;
        }
    }
}

fn rgb(c: [u8; 3]) -> Hsla {
    gpui::rgb(((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32).into()
}

impl CardBody for TerminalBody {
    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        scale: f64,
        focused: bool,
        now: f64,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.scale = scale;
        self.dirty = false;
        self.painted_focused = focused;
        self.painted_phase = self.blink_on(now);
        if let Some(error) = self.error.clone() {
            self.paint_error(&error, bounds, scale, window, cx);
            return;
        }
        let t = std::time::Instant::now();
        let mut frame = std::mem::take(&mut self.frame);
        let rebuilt = self.grid.update_frame(&self.palette, &mut frame);
        timing_add(0, t);
        let t = std::time::Instant::now();
        self.refresh_links(&frame, &rebuilt);
        timing_add(1, t);
        window.paint_quad(fill(bounds, rgb(self.palette.background)));
        let font_size = px((self.font_px * scale) as f32);
        let line_h = px((self.font_px * self.line_height * scale) as f32);
        let cell_w = px((self.cell_w * scale) as f32);
        let pad = px((PAD * scale) as f32);
        let origin = point(bounds.origin.x + pad, bounds.origin.y + pad);
        let mut base = font(self.font_family.clone());
        base.weight = self.weight;
        let bold_weight = self.bold_weight;
        // Too small to read: skip the glyphs, keep the ground. The mid-zoom
        // label names the card instead.
        let legible = font_size >= px(crate::chrome::LEGIBLE_FONT_PX as f32);
        // The cursor under the text: solid when focused and on, hollow when
        // the card is not focused, nothing while scrolled into history.
        if frame.cursor_kind != CursorKind::Hidden
            && frame.display_offset == 0
            && (!focused || self.painted_phase)
        {
            let (col, row) = frame.cursor;
            let x = origin.x + cell_w * col as f32;
            let y = origin.y + line_h * row as f32;
            let rect = match frame.cursor_kind {
                CursorKind::Beam => Bounds::new(
                    point(x, y),
                    size(
                        px(TERM_CURSOR_STROKE_PX * scale as f32)
                            .max(px(crate::chrome::HAIRLINE_PX as f32)),
                        line_h,
                    ),
                ),
                CursorKind::Underline => Bounds::new(
                    point(x, y + line_h - px(TERM_CURSOR_STROKE_PX)),
                    size(cell_w, px(TERM_CURSOR_STROKE_PX)),
                ),
                _ => Bounds::new(point(x, y), size(cell_w, line_h)),
            };
            let color = rgb(self.palette.cursor);
            if focused || frame.cursor_kind != CursorKind::Block {
                window.paint_quad(fill(rect, color));
            } else {
                window.paint_quad(
                    outline(rect, color, gpui::BorderStyle::Solid)
                        .border_widths(px((scale as f32).max(crate::chrome::HAIRLINE_PX as f32))),
                );
            }
        }
        if !legible {
            // Too small for glyphs, not for texture: each run of text is a
            // faint bar the width of its characters, so a full card reads
            // as full from across the canvas and an empty one as empty.
            let ink = crate::chrome::with_alpha(
                rgb(self.palette.foreground),
                crate::chrome::TEXTURE_BAR_ALPHA,
            );
            let bar_h = (line_h * crate::chrome::TEXTURE_BAR_HEIGHT_RATIO)
                .max(px(crate::chrome::HAIRLINE_PX as f32));
            for (r, row) in frame.rows.iter().enumerate() {
                if row_paints_nothing(row) {
                    continue;
                }
                let y = origin.y + line_h * r as f32 + (line_h - bar_h) / 2.;
                let mut col = 0usize;
                for run in &row.runs {
                    let mut start: Option<usize> = None;
                    for (i, ch) in run.text.chars().enumerate() {
                        let blank = ch == ' ' || ch == SPACER;
                        match (blank, start) {
                            (false, None) => start = Some(col + i),
                            (true, Some(s)) => {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(origin.x + cell_w * s as f32, y),
                                        size(cell_w * (col + i - s) as f32, bar_h),
                                    ),
                                    run.bg.map(rgb).unwrap_or(ink),
                                ));
                                start = None;
                            }
                            _ => {}
                        }
                    }
                    let n = run.text.chars().count();
                    if let Some(s) = start {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(origin.x + cell_w * s as f32, y),
                                size(cell_w * (col + n - s) as f32, bar_h),
                            ),
                            run.bg.map(rgb).unwrap_or(ink),
                        ));
                    }
                    col += n;
                }
            }
            self.frame = frame;
            self.paint_scrim(bounds, focused, window);
            return;
        }
        let hover = self.hover;
        for (r, row) in frame.rows.iter().enumerate() {
            if row_paints_nothing(row) {
                continue;
            }
            let y = origin.y + line_h * r as f32;
            // Backgrounds first, per run, so a full-width highlight is a quad
            // and not a shaped-line property.
            let mut x = origin.x;
            for run in &row.runs {
                let w = cell_w * run.text.chars().count() as f32;
                if let Some(bg) = run.bg {
                    window.paint_quad(fill(Bounds::new(point(x, y), size(w, line_h)), rgb(bg)));
                }
                x += w;
            }
            // The underline is the Cmd+hover affordance and nothing else.
            // Underlining every confirmed link marked up most of the output
            // of anything that prints paths, which is most things.
            if let Some(links) = self.links.get(r) {
                for (f, _) in links {
                    let start = row.text[..f.start.min(row.text.len())].chars().count();
                    let len = row.text[f.start.min(row.text.len())..f.end.min(row.text.len())]
                        .chars()
                        .count();
                    if !under_hover(hover, r, start, len) {
                        continue;
                    }
                    let ux = origin.x + cell_w * start as f32;
                    window.paint_quad(fill(
                        Bounds::new(
                            point(ux, y + line_h - px(LINK_UNDERLINE_OFFSET_PX)),
                            size(cell_w * len as f32, px(crate::chrome::HAIRLINE_PX as f32)),
                        ),
                        rgb(self.palette.selection),
                    ));
                }
            }
            // Shape per chunk, cached against the row's content and size.
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            use std::hash::{Hash, Hasher};
            row.text.hash(&mut hasher);
            f32::from(font_size).to_bits().hash(&mut hasher);
            for run in &row.runs {
                (
                    run.fg,
                    run.bg,
                    run.bold,
                    run.italic,
                    run.underline,
                    run.strikeout,
                    run.dim,
                    run.text.len(),
                )
                    .hash(&mut hasher);
            }
            let key = hasher.finish();
            if self.shaped.len() <= r {
                self.shaped.resize(r + 1, (0, vec![]));
            }
            if self.shaped[r].0 != key {
                let t = std::time::Instant::now();
                let chunks = shape_row(row, &base, bold_weight, font_size, window);
                self.shaped[r] = (key, chunks);
                timing_add(2, t);
            }
            let t = std::time::Instant::now();
            for (col, line) in &self.shaped[r].1 {
                let _ = line.paint(
                    point(origin.x + cell_w * *col as f32, y),
                    line_h,
                    window,
                    cx,
                );
            }
            timing_add(3, t);
        }
        self.frame = frame;
        self.paint_scrim(bounds, focused, window);
    }

    fn resized(&mut self, _world: Size) {
        // `terminals.rs` refits each frame and resizes the PTY with it; a
        // refit here would leave the PTY at the old size.
    }

    fn key(&mut self, k: &Keystroke, now: f64, cx: &mut App) -> BodyAction {
        let m = &k.modifiers;
        // The app owns Cmd; the Cmd keys a terminal answers are copy, paste
        // and the line-movement arrows the encoder knows.
        if m.platform && k.key == "c" {
            if let Some(text) = self.grid.selection_text() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            return BodyAction::None;
        }
        if m.platform && k.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                let bracketed = self.grid.bracketed_paste();
                self.write(paste(&text, bracketed));
            }
            return BodyAction::None;
        }
        let key = Key {
            name: &k.key,
            text: k.key_char.as_deref(),
            ctrl: m.control,
            alt: m.alt,
            shift: m.shift,
            cmd: m.platform,
        };
        if let Some(bytes) = encode_with(&key, self.grid.app_cursor(), self.grid.kitty_keys()) {
            self.grid.scroll_to_bottom();
            // Typing is where the selection stops mattering and where a
            // blinking cursor must be visible.
            self.grid.clear_selection();
            self.blink_epoch = now;
            self.dirty = true;
            self.write(bytes);
        }
        BodyAction::None
    }

    fn mouse_down(
        &mut self,
        local: Point,
        button: gpui::MouseButton,
        modifiers: &gpui::Modifiers,
        clicks: usize,
    ) -> BodyAction {
        if self.error.is_some() {
            return BodyAction::Retry;
        }
        let (col, row) = self.cell_at(local);
        let b = match button {
            gpui::MouseButton::Left => MouseButton::Left,
            gpui::MouseButton::Middle => MouseButton::Middle,
            gpui::MouseButton::Right => MouseButton::Right,
            _ => return BodyAction::None,
        };
        // Cmd+click opens a link; Shift sends it out. The reflex from every
        // editor, where Shift on an open means "elsewhere".
        if modifiers.platform && b == MouseButton::Left {
            if let Some((f, kind)) = self.link_at(col, row).cloned() {
                let home = infiniterm_core::paths::home_dir()
                    .to_string_lossy()
                    .into_owned();
                if modifiers.shift {
                    return match f.kind {
                        LinkKind::Url => BodyAction::OpenExternal {
                            url: Some(f.target),
                            path: None,
                        },
                        LinkKind::Path => BodyAction::OpenExternal {
                            url: None,
                            path: Some((self.cwd.clone(), f.target)),
                        },
                    };
                }
                return match (f.kind, kind) {
                    (LinkKind::Url, _) => BodyAction::Open(url_plan(&f.target, &self.cwd)),
                    (LinkKind::Path, Some(kind)) => BodyAction::Open(open_plan(
                        &self.resolve_against(&f.target, &home),
                        kind,
                        f.line.map(|l| l as f64),
                    )),
                    _ => BodyAction::None,
                };
            }
            return BodyAction::None;
        }
        if self.grid.wants_mouse() && !modifiers.shift {
            let sgr = self.grid.sgr_mouse();
            self.write(mouse::press(b, col, row, Self::mods(modifiers), sgr));
            self.dragging = Some(b);
            return BodyAction::None;
        }
        // A plain press: the start of a text selection. Shift over a mouse
        // program gets here too, the override every terminal gives it.
        if b == MouseButton::Left {
            let kind = match clicks {
                1 => SelectKind::Cells,
                2 => SelectKind::Words,
                _ => SelectKind::Lines,
            };
            self.grid.start_selection(col, row, kind);
            self.selecting = true;
            self.dirty = true;
        }
        BodyAction::None
    }

    fn mouse_up(&mut self, local: Point, button: gpui::MouseButton, modifiers: &gpui::Modifiers) {
        if self.selecting {
            self.selecting = false;
            // A click that did not move selects nothing, and a lone click
            // must leave no one-cell selection behind for Cmd+C to copy.
            if !self.grid.has_selection() {
                self.grid.clear_selection();
                self.dirty = true;
            }
            return;
        }
        let Some(b) = self.dragging.take() else {
            return;
        };
        let _ = button;
        let (col, row) = self.cell_at(local);
        let sgr = self.grid.sgr_mouse();
        self.write(mouse::release(b, col, row, Self::mods(modifiers), sgr));
    }

    fn mouse_move(&mut self, local: Point, modifiers: &gpui::Modifiers) {
        let (col, row) = self.cell_at(local);
        // Nothing else changes when the pointer moves, so the underline
        // appearing or going away is the only reason for the next frame.
        let was = self.hover;
        self.hover = modifiers.platform.then_some((col, row));
        if self.hover != was {
            self.dirty = true;
        }
        if self.selecting {
            self.grid.update_selection(col, row, self.right_half(local));
            self.dirty = true;
            return;
        }
        if let Some(b) = self.dragging {
            if self.grid.mouse_drag() {
                let sgr = self.grid.sgr_mouse();
                self.write(mouse::motion(b, col, row, Self::mods(modifiers), sgr));
            }
        }
    }

    /// A bare scroll: the program's, when it asked for the mouse; arrow keys
    /// on the alternate screen (`less`); the scrollback otherwise. `dy` is in
    /// card pixels, positive up.
    fn wheel(&mut self, local: Point, _dx: f64, dy: f64, modifiers: &gpui::Modifiers) {
        let lines = self.wheel_carry.lines(dy, self.font_px * self.line_height) as i32;
        if std::env::var_os("INFINITERM_KEYLOG").is_some() {
            eprintln!(
                "[wheel]   terminal: {lines} lines, mouse-mode {} alt-scroll {} offset {}",
                self.grid.wants_mouse(),
                self.grid.alternate_scroll(),
                self.frame.display_offset
            );
        }
        if lines == 0 {
            return;
        }
        if self.grid.wants_mouse() {
            let (col, row) = self.cell_at(local);
            let sgr = self.grid.sgr_mouse();
            let b = if lines > 0 {
                MouseButton::WheelUp
            } else {
                MouseButton::WheelDown
            };
            for _ in 0..lines.unsigned_abs().min(WHEEL_MAX_LINES_PER_EVENT) {
                self.write(mouse::press(b, col, row, Self::mods(modifiers), sgr));
            }
        } else if self.grid.alternate_scroll() {
            let key = if lines > 0 { "up" } else { "down" };
            let bytes = encode(
                &Key {
                    name: key,
                    ..Default::default()
                },
                self.grid.app_cursor(),
            )
            .unwrap_or_default();
            for _ in 0..lines.unsigned_abs().min(WHEEL_MAX_LINES_PER_EVENT) {
                self.write(bytes.clone());
            }
        } else {
            self.grid.scroll(lines);
            self.dirty = true;
        }
    }

    fn wants_frame(&self, now: f64) -> bool {
        self.dirty
            || (self.painted_focused
                && self.blink
                && self.error.is_none()
                && self.blink_on(now) != self.painted_phase)
    }

    fn captures_drag(&self) -> bool {
        self.selecting || self.dragging.is_some()
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// A row as chunks of `CHUNK` cells, each shaped on its own with the run
/// attributes that fall inside it.
fn shape_row(
    row: &infiniterm_term::grid::Row,
    base: &gpui::Font,
    bold_weight: FontWeight,
    font_size: Pixels,
    window: &Window,
) -> Vec<(usize, gpui::ShapedLine)> {
    // Per-cell attributes, then regrouped per chunk.
    let mut cells: Vec<(char, &infiniterm_term::grid::Run)> = Vec::with_capacity(row.text.len());
    for run in &row.runs {
        for ch in run.text.chars() {
            cells.push((ch, run));
        }
    }
    let mut out = vec![];
    let mut col = 0;
    while col < cells.len() {
        // A chunk is at most CHUNK cells of ASCII, whose advance in a
        // monospace font is the cell; anything else (an icon from a Nerd
        // Font, an emoji, a wide character) is shaped on its own and put
        // at its cell, because its advance is whatever the fallback font
        // says and everything shaped after it in the same line would drift.
        let end = if cells[col].0.is_ascii() {
            let mut e = col;
            while e < cells.len() && e - col < CHUNK && cells[e].0.is_ascii() {
                e += 1;
            }
            e
        } else {
            let mut e = col + 1;
            while e < cells.len() && cells[e].0 == infiniterm_term::grid::SPACER {
                e += 1;
            }
            e
        };
        let slice = &cells[col..end];
        if slice
            .iter()
            .all(|(c, _)| c.is_whitespace() || *c == infiniterm_term::grid::SPACER)
        {
            col = end;
            continue;
        }
        let mut text = String::new();
        let mut runs: Vec<TextRun> = vec![];
        for (ch, run) in slice {
            if *ch == infiniterm_term::grid::SPACER {
                continue;
            }
            let len = ch.len_utf8();
            text.push(*ch);
            let same = runs.last().is_some_and(|last: &TextRun| {
                let color = if run.dim {
                    crate::chrome::with_alpha(rgb(run.fg), TERM_DIM_ALPHA)
                } else {
                    rgb(run.fg)
                };
                last.color == color
                    && last.font.weight == if run.bold { bold_weight } else { base.weight }
                    && last.font.style
                        == if run.italic {
                            FontStyle::Italic
                        } else {
                            base.style
                        }
                    && last.underline.is_some() == run.underline
                    && last.strikethrough.is_some() == run.strikeout
            });
            if same {
                runs.last_mut().unwrap().len += len;
                continue;
            }
            let mut f = base.clone();
            if run.bold {
                f.weight = bold_weight;
            }
            if run.italic {
                f.style = FontStyle::Italic;
            }
            let color = if run.dim {
                crate::chrome::with_alpha(rgb(run.fg), TERM_DIM_ALPHA)
            } else {
                rgb(run.fg)
            };
            runs.push(TextRun {
                len,
                font: f,
                color,
                background_color: None,
                underline: run.underline.then_some(UnderlineStyle {
                    thickness: px(crate::chrome::HAIRLINE_PX as f32),
                    color: Some(color),
                    wavy: false,
                }),
                strikethrough: run.strikeout.then_some(gpui::StrikethroughStyle {
                    thickness: px(crate::chrome::HAIRLINE_PX as f32),
                    color: Some(color),
                }),
            });
        }
        out.push((
            col,
            window
                .text_system()
                .shape_line(SharedString::from(text), font_size, &runs, None),
        ));
        col = end;
    }
    out
}

/// Whether a link occupying `len` cells from `start` on row `r` is the one
/// under the pointer. `hover` is `None` unless Cmd is held, which is what
/// makes the underline a Cmd+hover affordance rather than permanent markup.
///
/// A stationary pointer whose Cmd is released keeps its underline until the
/// next move: nothing reports a bare modifier release to a body.
fn under_hover(hover: Option<(usize, usize)>, r: usize, start: usize, len: usize) -> bool {
    hover.is_some_and(|(hc, hr)| hr == r && hc >= start && hc < start + len)
}

/// Whether a row can be skipped by the painter. Blank text is not enough:
/// a space with a background is a painted cell. Pi draws its cursor as an
/// inverse-video space, and on an empty input line that space is the whole
/// row, so skipping "rows with no text" skipped the cursor and it could
/// not be seen until something was typed beside it. The same skip would
/// have hidden any TUI's full-width highlight on an empty row.
fn row_paints_nothing(row: &infiniterm_term::grid::Row) -> bool {
    row.text.trim().is_empty() && row.runs.iter().all(|r| r.bg.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> TerminalBody {
        let metrics = Metrics {
            family: "Menlo".into(),
            font_px: 14.,
            line_height: 1.2,
            cell_w: 8.4,
            weight: FontWeight::NORMAL,
            bold_weight: FontWeight::BOLD,
        };
        TerminalBody::new(
            &metrics,
            Palette::default_palette(),
            Size { w: 800., h: 600. },
            1000,
            "/tmp".into(),
        )
    }

    // Pi's cursor on an empty input line: one inverse-video space. Its
    // text trims to nothing and it must be painted anyway.
    #[test]
    fn a_blank_row_with_a_background_is_still_painted() {
        use infiniterm_term::grid::{Row, Run};
        let run = |bg: Option<[u8; 3]>| Run {
            text: " ".into(),
            fg: [0, 0, 0],
            bg,
            bold: false,
            italic: false,
            underline: false,
            strikeout: false,
            dim: false,
        };
        let cursor = Row {
            runs: vec![run(Some([200, 200, 200]))],
            text: " ".into(),
        };
        assert!(
            !row_paints_nothing(&cursor),
            "an inverse space is a painted cell"
        );
        let empty = Row {
            runs: vec![run(None)],
            text: "      ".into(),
        };
        assert!(
            row_paints_nothing(&empty),
            "a truly blank row is still skipped"
        );
        let text = Row {
            runs: vec![run(None)],
            text: " x ".into(),
        };
        assert!(!row_paints_nothing(&text));
    }

    // A cursor position report, a clipboard write and a title, as a ring
    // would hold them from a program's past. Live, the report is answered
    // and the clipboard is set; replayed, only the title survives.
    const HISTORY: &[u8] = b"\x1b]0;old title\x07\x1b[6n\x1b]52;c;aGVsbG8=\x07text";

    #[test]
    fn live_output_answers_queries_and_sets_the_clipboard() {
        let mut b = body();
        b.feed(HISTORY);
        assert!(!b.replies.is_empty(), "the cursor report is answered");
        assert_eq!(b.clipboard_out.as_deref(), Some("hello"));
        assert_eq!(b.title.as_deref(), Some("old title"));
    }

    // The same bytes as history: nothing is answered, nothing reaches the
    // clipboard, the screen and the title are what they were.
    #[test]
    fn a_replay_draws_the_past_without_reliving_it() {
        let mut b = body();
        b.feed_replay(HISTORY);
        assert!(
            b.replies.is_empty(),
            "a question from the past is not answered again"
        );
        assert!(
            b.clipboard_out.is_none(),
            "history does not touch the clipboard"
        );
        assert_eq!(b.title.as_deref(), Some("old title"));
        assert!(b.dirty, "and it still needs painting");
    }

    // The underline is drawn only under the pointer, and only while Cmd is
    // held: `hover` is None otherwise.
    #[test]
    fn a_link_underlines_only_under_a_cmd_hover() {
        assert!(!under_hover(None, 3, 5, 4));
        assert!(under_hover(Some((5, 3)), 3, 5, 4));
        assert!(under_hover(Some((8, 3)), 3, 5, 4));
        // The cell past the end, the row above, the cell before.
        assert!(!under_hover(Some((9, 3)), 3, 5, 4));
        assert!(!under_hover(Some((5, 2)), 3, 5, 4));
        assert!(!under_hover(Some((4, 3)), 3, 5, 4));
    }

    #[test]
    fn weights_are_spelled_as_css_spells_them() {
        assert_eq!(weight_of("normal"), FontWeight::NORMAL);
        assert_eq!(weight_of("bold"), FontWeight::BOLD);
        assert_eq!(weight_of("800"), FontWeight(800.));
        assert_eq!(weight_of(" 600 "), FontWeight(600.));
        assert_eq!(weight_of("heavy"), FontWeight::NORMAL);
        assert_eq!(weight_of("1000"), FontWeight::NORMAL);
    }
}
