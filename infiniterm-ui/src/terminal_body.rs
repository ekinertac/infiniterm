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
//! paths in the output are underlined and open with Cmd+click, the same
//! path as `ift <path>`. A plain click only focuses the card.
//!
//! Size follows the card's rect in world units at scale 1: the grid is
//! however many cells fit, and the PTY is told, so programs reflow. Zooming
//! changes nothing about the grid, only the pixel size it is drawn at.
use crate::body::{BodyAction, CardBody};
use gpui::{
    fill, font, point, px, size, App, Bounds, FontStyle, FontWeight, Hsla, Keystroke, Pixels,
    SharedString, TextRun, UnderlineStyle, Window,
};
use infiniterm_core::backend::PaneId;
use infiniterm_core::grid::{Point, Size};
use infiniterm_core::ift::{open_plan, url_plan, PathKind};
use infiniterm_core::links::{find_links, Found, LinkKind};
use infiniterm_core::links_fs::path_kinds;
use infiniterm_term::grid::{CursorKind, Frame, Grid, TermEvent};
use infiniterm_term::keys::{encode, paste, Key};
use infiniterm_term::mouse::{self, Mods, MouseButton};
use infiniterm_term::palette::Palette;

/// Inset from the card's edge to the first cell, in world units.
const PAD: f64 = 6.;

pub struct TerminalBody {
    pub pane: Option<PaneId>,
    pub grid: Grid,
    pub palette: Palette,
    pub font_family: String,
    pub font_px: f64,
    pub line_height: f64,
    /// The cell width at `font_px`, measured once from the font.
    pub cell_w: f64,
    pub cwd: String,
    /// The card's directory, for resolving paths in the output.
    pub error: Option<String>,
    /// Bytes to write to the pty: the ui drains them after each event.
    pub outgoing: Vec<Vec<u8>>,
    /// Bell rang since the last frame; the frame flashes the border.
    pub bell: bool,
    /// Output arrived since the last paint.
    pub dirty: bool,
    pub title: Option<String>,
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

pub struct Metrics {
    pub family: String,
    pub font_px: f64,
    pub line_height: f64,
    pub cell_w: f64,
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
            font_px: metrics.font_px,
            line_height: metrics.line_height,
            cell_w: metrics.cell_w,
            cwd,
            error: None,
            outgoing: vec![],
            bell: false,
            dirty: true,
            title: None,
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

    fn drain_events(&mut self) {
        for event in self.grid.take_events() {
            match event {
                TermEvent::Write(s) => self.outgoing.push(s.into_bytes()),
                TermEvent::Title(t) => self.title = Some(t),
                TermEvent::Bell => self.bell = true,
                TermEvent::Clipboard(_) => {} // OSC 52 into the system clipboard: Phase 10
            }
        }
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    /// Metrics changed (a settings edit): the grid is re-counted, which the
    /// reference's `refit` rule insists on, and the caller resizes the PTY.
    pub fn set_metrics(&mut self, metrics: &Metrics, world: Size) -> bool {
        self.font_family = metrics.family.clone();
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

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
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

    /// Re-scans the rows that changed for links and asks the filesystem
    /// about the paths, so a version number is never underlined.
    fn refresh_links(&mut self, frame: &Frame) {
        let texts: Vec<String> = frame
            .rows
            .iter()
            .map(|r| r.text.trim_end().to_string())
            .collect();
        if texts == self.link_texts {
            return;
        }
        self.links = texts
            .iter()
            .map(|line| {
                let found = find_links(line);
                let paths: Vec<&str> = found
                    .iter()
                    .filter(|f| f.kind == LinkKind::Path)
                    .map(|f| f.target.as_str())
                    .collect();
                let kinds = if paths.is_empty() {
                    vec![]
                } else {
                    path_kinds(&self.cwd, &paths)
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
            })
            .collect();
        self.link_texts = texts;
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

fn rgb(c: [u8; 3]) -> Hsla {
    gpui::rgb(((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32).into()
}

impl CardBody for TerminalBody {
    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        scale: f64,
        _focused: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.scale = scale;
        self.dirty = false;
        let frame = self.grid.frame(&self.palette);
        self.refresh_links(&frame);
        window.paint_quad(fill(bounds, rgb(self.palette.background)));
        let font_size = px((self.font_px * scale) as f32);
        let line_h = px((self.font_px * self.line_height * scale) as f32);
        let cell_w = px((self.cell_w * scale) as f32);
        let pad = px((PAD * scale) as f32);
        let origin = point(bounds.origin.x + pad, bounds.origin.y + pad);
        let base = font(self.font_family.clone());
        // Too small to read: skip the glyphs, keep the ground. The mid-zoom
        // label names the card instead.
        let legible = font_size >= px(3.);
        // The cursor under the text.
        if frame.cursor_kind != CursorKind::Hidden && frame.display_offset == 0 {
            let (col, row) = frame.cursor;
            let x = origin.x + cell_w * col as f32;
            let y = origin.y + line_h * row as f32;
            let rect = match frame.cursor_kind {
                CursorKind::Beam => {
                    Bounds::new(point(x, y), size(px(2. * scale as f32).max(px(1.)), line_h))
                }
                CursorKind::Underline => {
                    Bounds::new(point(x, y + line_h - px(2.)), size(cell_w, px(2.)))
                }
                _ => Bounds::new(point(x, y), size(cell_w, line_h)),
            };
            window.paint_quad(fill(rect, rgb(self.palette.cursor)));
        }
        if !legible {
            return;
        }
        let hover = self.hover;
        for (r, row) in frame.rows.iter().enumerate() {
            if row.text.trim().is_empty() {
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
            // Link underlines, for the ones the filesystem confirmed.
            if let Some(links) = self.links.get(r) {
                for (f, _) in links {
                    let start = row.text[..f.start.min(row.text.len())].chars().count();
                    let len = row.text[f.start.min(row.text.len())..f.end.min(row.text.len())]
                        .chars()
                        .count();
                    let hovered =
                        hover.is_some_and(|(hc, hr)| hr == r && hc >= start && hc < start + len);
                    let ux = origin.x + cell_w * start as f32;
                    let color = if hovered {
                        rgb(self.palette.selection)
                    } else {
                        crate::chrome::with_alpha(rgb(self.palette.foreground), 0.5)
                    };
                    window.paint_quad(fill(
                        Bounds::new(
                            point(ux, y + line_h - px(1.5)),
                            size(cell_w * len as f32, px(1.)),
                        ),
                        color,
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
                let chunks = shape_row(row, &base, font_size, window);
                self.shaped[r] = (key, chunks);
            }
            for (col, line) in &self.shaped[r].1 {
                let _ = line.paint(
                    point(origin.x + cell_w * *col as f32, y),
                    line_h,
                    window,
                    cx,
                );
            }
        }
    }

    fn resized(&mut self, world: Size) {
        self.refit(world);
    }

    fn key(&mut self, k: &Keystroke, cx: &mut App) {
        let m = &k.modifiers;
        // The app owns Cmd; the two Cmd keys a terminal answers are paste and
        // the line-movement arrows the encoder knows.
        if m.platform && k.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                let bracketed = self.grid.bracketed_paste();
                self.write(paste(&text, bracketed));
            }
            return;
        }
        let key = Key {
            name: &k.key,
            text: k.key_char.as_deref(),
            ctrl: m.control,
            alt: m.alt,
            shift: m.shift,
            cmd: m.platform,
        };
        if let Some(bytes) = encode(&key, self.grid.app_cursor()) {
            self.grid.scroll_to_bottom();
            self.write(bytes);
        }
    }

    fn mouse_down(
        &mut self,
        local: Point,
        button: gpui::MouseButton,
        modifiers: &gpui::Modifiers,
        _clicks: usize,
    ) -> BodyAction {
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
        }
        BodyAction::None
    }

    fn mouse_up(&mut self, local: Point, button: gpui::MouseButton, modifiers: &gpui::Modifiers) {
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
        self.hover = modifiers.platform.then_some((col, row));
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
        let lines = (dy / (self.font_px * self.line_height) * 3.).round() as i32;
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
            for _ in 0..lines.unsigned_abs().min(10) {
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
            for _ in 0..lines.unsigned_abs().min(10) {
                self.write(bytes.clone());
            }
        } else {
            self.grid.scroll(lines);
            self.dirty = true;
        }
    }

    fn wants_frame(&self) -> bool {
        self.dirty
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
        let end = (col + CHUNK).min(cells.len());
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
                    crate::chrome::with_alpha(rgb(run.fg), 0.6)
                } else {
                    rgb(run.fg)
                };
                last.color == color
                    && last.font.weight
                        == if run.bold {
                            FontWeight::BOLD
                        } else {
                            base.weight
                        }
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
                f.weight = FontWeight::BOLD;
            }
            if run.italic {
                f.style = FontStyle::Italic;
            }
            let color = if run.dim {
                crate::chrome::with_alpha(rgb(run.fg), 0.6)
            } else {
                rgb(run.fg)
            };
            runs.push(TextRun {
                len,
                font: f,
                color,
                background_color: None,
                underline: run.underline.then_some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(color),
                    wavy: false,
                }),
                strikethrough: run.strikeout.then_some(gpui::StrikethroughStyle {
                    thickness: px(1.),
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
