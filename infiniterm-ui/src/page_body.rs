//! The Page card (#42): a Markdown document drawn read-only, for the
//! welcome card first. Parsing and wrapping are core's `page.rs`; this is
//! the drawing, the scroll and the links.
//!
//! It never takes the keyboard. The editor it replaces had to either lock on
//! a click (a newcomer clicking "Start here" lost the canvas keys with
//! nothing saying how to get them back) or never lock (and the docs lost
//! their keys too, #41). A page has nothing to type into, so the wheel
//! scrolls it and, while it is focused, the arrows, Page Up/Down, Space,
//! Home and End scroll it; every other key is left to the canvas.
//!
//! A click on a link opens it: a web address in the system browser, a
//! Markdown file beside this one as another Page. The file is re-read when
//! its mtime moves (the welcome file is rewritten at every launch).
//!
//! Built and kept in step by `editors.rs` (`reconcile_pages`).
use crate::body::{BodyAction, CardBody};
use crate::terminal_body::Metrics;
use gpui::{
    fill, point, px, size, App, Bounds, FontStyle, FontWeight, Hsla, Keystroke, Pixels,
    SharedString, TextRun, UnderlineStyle, Window,
};
use infiniterm_core::files::file_mtime;
use infiniterm_core::grid::{Point, Size};
use infiniterm_core::page::{layout, parse, Block, Line, LineKind};

const PAD_X: f64 = 14.;
const PAD_Y: f64 = 10.;
/// Prose reads better looser than a terminal's rows.
const LINE: f64 = 1.45;
const DISK_POLL_MS: f64 = 2000.;
/// The scrollbar, in SCREEN pixels like every affordance: it keeps its width
/// at any zoom, and the card's text is what scales. Faint on purpose: it says
/// the card scrolls, nothing more.
const BAR_W_PX: f32 = 3.;
const BAR_INSET_PX: f32 = 5.;
const BAR_MIN_PX: f64 = 24.;
const BAR_ALPHA: f32 = 0.55;
/// Lines the arrows move; Page Up/Down and Space move a screen less this.
const ARROW_LINES: usize = 3;
const PAGE_OVERLAP_LINES: usize = 2;

/// Resolved by `editors.rs` from the theme and the chrome.
#[derive(Clone, Debug, PartialEq)]
pub struct PageColors {
    pub background: Hsla,
    pub text: Hsla,
    pub faint: Hsla,
    pub heading: Hsla,
    pub code: Hsla,
    pub link: Hsla,
}

pub struct PageBody {
    pub path: String,
    pub metrics: Metrics,
    pub colors: PageColors,
    pub inactive_dim: f64,
    blocks: Vec<Block>,
    /// The layout for `laid_cols`, rebuilt when the width changes.
    lines: Vec<Line>,
    laid_cols: usize,
    scroll: usize,
    world: Size,
    mtime: Option<u64>,
    last_poll: f64,
    dirty: bool,
    wheel_carry: crate::chrome::WheelCarry,
}

impl PageBody {
    pub fn new(path: String, metrics: &Metrics, colors: PageColors, world: Size) -> PageBody {
        let mut b = PageBody {
            path,
            metrics: metrics.clone(),
            colors,
            inactive_dim: 0.,
            blocks: vec![],
            lines: vec![],
            laid_cols: 0,
            scroll: 0,
            world,
            mtime: None,
            last_poll: 0.,
            dirty: true,
            wheel_carry: Default::default(),
        };
        b.reload();
        b
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.laid_cols = 0;
    }

    /// Re-reads the document when it changed on disk (`DISK_POLL_MS`).
    pub fn idle(&mut self, now: f64) {
        if now - self.last_poll < DISK_POLL_MS {
            return;
        }
        self.last_poll = now;
        if file_mtime(&self.path) != self.mtime {
            self.reload();
        }
    }

    fn reload(&mut self) {
        self.mtime = file_mtime(&self.path);
        let text = std::fs::read_to_string(&self.path)
            .unwrap_or_else(|e| format!("# Cannot read this page\n\n`{}`: {e}", self.path));
        self.blocks = parse(&text);
        self.laid_cols = 0;
        self.dirty = true;
    }

    fn line_h(&self) -> f64 {
        self.metrics.font_px * LINE
    }

    fn cols(&self) -> usize {
        ((self.world.w - PAD_X * 2.) / self.metrics.cell_w)
            .floor()
            .max(8.) as usize
    }

    fn visible(&self) -> usize {
        ((self.world.h - PAD_Y * 2.) / self.line_h())
            .floor()
            .max(1.) as usize
    }

    fn relayout(&mut self) {
        let cols = self.cols();
        if cols != self.laid_cols {
            self.lines = layout(&self.blocks, cols);
            self.laid_cols = cols;
        }
    }

    fn scroll_by(&mut self, lines: isize) {
        self.relayout();
        let max = self.lines.len().saturating_sub(self.visible()) as isize;
        let next = (self.scroll as isize + lines).clamp(0, max.max(0));
        if next as usize != self.scroll {
            self.scroll = next as usize;
            self.dirty = true;
        }
    }

    /// The line and cell under a point in card pixels.
    fn cell_at(&self, local: Point) -> Option<(usize, usize)> {
        if local.y < PAD_Y || local.x < PAD_X {
            return None;
        }
        let row = ((local.y - PAD_Y) / self.line_h()).floor() as usize + self.scroll;
        let col = ((local.x - PAD_X) / self.metrics.cell_w).floor() as usize;
        (row < self.lines.len()).then_some((row, col))
    }

    /// Scroll keys only; anything else is not the page's (`Ignored`), so it
    /// stays the canvas's, which is the point of a page.
    fn scroll_key(&mut self, k: &Keystroke) -> BodyAction {
        let m = &k.modifiers;
        if m.platform || m.alt || m.control {
            return BodyAction::Ignored;
        }
        let page = self.visible().saturating_sub(PAGE_OVERLAP_LINES).max(1) as isize;
        match k.key.as_str() {
            "down" => self.scroll_by(ARROW_LINES as isize),
            "up" => self.scroll_by(-(ARROW_LINES as isize)),
            "pagedown" => self.scroll_by(page),
            "space" if m.shift => self.scroll_by(-page),
            "space" => self.scroll_by(page),
            "pageup" => self.scroll_by(-page),
            "home" => self.scroll_by(-(self.lines.len() as isize)),
            "end" => self.scroll_by(self.lines.len() as isize),
            _ => return BodyAction::Ignored,
        }
        BodyAction::None
    }

    /// Where a link goes: a web address to the system browser, a document
    /// next to this one to a Page of its own.
    fn follow(&self, target: &str) -> BodyAction {
        if target.starts_with("http://") || target.starts_with("https://") {
            return BodyAction::OpenExternal {
                url: Some(target.to_string()),
                path: None,
            };
        }
        let base = std::path::Path::new(&self.path)
            .parent()
            .unwrap_or(std::path::Path::new("/"));
        let file = target.split('#').next().unwrap_or(target);
        let path = base.join(file).to_string_lossy().into_owned();
        BodyAction::Open(infiniterm_core::ift::OpenPlan::Page {
            cwd: base.to_string_lossy().into_owned(),
            path,
        })
    }
}

impl PageBody {
    /// A thin thumb at the right edge when the page is longer than the card,
    /// painted before the inactive dim so it dims with the card.
    fn paint_scrollbar(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let inset = px(BAR_INSET_PX);
        let track = f32::from(bounds.size.height - inset * 2.) as f64;
        let Some((start, len)) = infiniterm_core::scrollbar::thumb(
            self.lines.len(),
            self.visible(),
            self.scroll,
            track,
            BAR_MIN_PX,
        ) else {
            return;
        };
        let thumb = Bounds::new(
            point(
                bounds.origin.x + bounds.size.width - inset - px(BAR_W_PX),
                bounds.origin.y + inset + px(start as f32),
            ),
            size(px(BAR_W_PX), px(len as f32)),
        );
        window.paint_quad(
            fill(
                thumb,
                crate::chrome::with_alpha(self.colors.faint, self.colors.faint.a * BAR_ALPHA),
            )
            .corner_radii(px(BAR_W_PX / 2.)),
        );
    }
}

impl CardBody for PageBody {
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
        self.world = Size {
            w: f32::from(bounds.size.width) as f64 / scale,
            h: f32::from(bounds.size.height) as f64 / scale,
        };
        window.paint_quad(fill(
            bounds,
            crate::chrome::card_fill(cx, self.colors.background),
        ));
        let font_size = px((self.metrics.font_px * scale) as f32);
        if font_size < crate::chrome::legible_font_px(window.scale_factor()) {
            return;
        }
        self.relayout();
        let max = self.lines.len().saturating_sub(self.visible());
        self.scroll = self.scroll.min(max);
        let s = |v: f64| px((v * scale) as f32);
        let line_h = s(self.line_h());
        let base = self.metrics.font();
        let x0 = bounds.origin.x + s(PAD_X);
        let mut y = bounds.origin.y + s(PAD_Y);
        for line in self.lines.iter().skip(self.scroll).take(self.visible()) {
            let x = x0 + s(line.indent as f64 * self.metrics.cell_w);
            match line.kind {
                LineKind::Rule => {
                    let w = bounds.size.width - s(PAD_X * 2.);
                    window.paint_quad(fill(
                        Bounds::new(point(x0, y + line_h / 2.), size(w, px(1.))),
                        crate::chrome::with_alpha(self.colors.faint, 0.6),
                    ));
                }
                LineKind::Quote => {
                    window.paint_quad(fill(
                        Bounds::new(point(x0, y), size(s(2.), line_h)),
                        self.colors.faint,
                    ));
                }
                _ => {}
            }
            if !line.spans.is_empty() {
                let mut text = String::new();
                let mut runs = vec![];
                for sp in &line.spans {
                    let mut f = base.clone();
                    let heading = matches!(line.kind, LineKind::Heading(_));
                    if sp.style.bold || heading {
                        f.weight = FontWeight::BOLD;
                    }
                    if sp.style.italic {
                        f.style = FontStyle::Italic;
                    }
                    let color = if sp.link.is_some() {
                        self.colors.link
                    } else if heading {
                        self.colors.heading
                    } else if sp.style.code || line.kind == LineKind::Code {
                        self.colors.code
                    } else if line.kind == LineKind::Quote {
                        self.colors.faint
                    } else {
                        self.colors.text
                    };
                    text.push_str(&sp.text);
                    runs.push(TextRun {
                        len: sp.text.len(),
                        font: f,
                        color,
                        background_color: None,
                        underline: sp.link.as_ref().map(|_| UnderlineStyle {
                            thickness: px(1.),
                            color: Some(color),
                            wavy: false,
                        }),
                        strikethrough: None,
                    });
                }
                let shaped = window.text_system().shape_line(
                    SharedString::from(text),
                    font_size,
                    &runs,
                    None,
                );
                let _ = shaped.paint(point(x, y), line_h, window, cx);
            }
            y += line_h;
        }
        self.paint_scrollbar(bounds, window);
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
        self.scroll_key(k)
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
        let target = self
            .cell_at(local)
            .and_then(|(row, col)| self.lines[row].link_at(col).map(str::to_string));
        match target {
            Some(t) => self.follow(&t),
            None => BodyAction::None,
        }
    }

    fn wheel(&mut self, _local: Point, _dx: f64, dy: f64, _modifiers: &gpui::Modifiers) {
        let lines = self.wheel_carry.lines(dy, self.line_h());
        if lines != 0 {
            self.scroll_by(-(lines as isize));
        }
    }

    fn wants_frame(&self, _now: f64) -> bool {
        self.dirty
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(md: &str) -> (PageBody, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("ift-page-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("p{}.md", md.len()));
        std::fs::write(&path, md).unwrap();
        let metrics = Metrics {
            family: "Menlo".into(),
            font_px: 14.,
            line_height: 1.2,
            cell_w: 8.4,
            weight: FontWeight::NORMAL,
            bold_weight: FontWeight::BOLD,
        };
        let colors = PageColors {
            background: gpui::black(),
            text: gpui::white(),
            faint: gpui::white(),
            heading: gpui::white(),
            code: gpui::white(),
            link: gpui::white(),
        };
        let b = PageBody::new(
            path.to_string_lossy().into_owned(),
            &metrics,
            colors,
            Size { w: 400., h: 200. },
        );
        (b, dir)
    }

    fn key(s: &str) -> Keystroke {
        Keystroke::parse(s).unwrap()
    }

    // The scroll keys scroll and every other key stays the canvas's.
    #[test]
    fn only_scroll_keys_are_the_pages() {
        let long: String = (0..200).map(|i| format!("line {i}\n\n")).collect();
        let (mut b, _) = page(&long);
        b.scroll_by(0);
        assert!(matches!(b.scroll_key(&key("down")), BodyAction::None));
        assert_eq!(b.scroll, ARROW_LINES);
        assert!(matches!(b.scroll_key(&key("end")), BodyAction::None));
        assert!(b.scroll > 100);
        for other in ["a", "enter", "escape", "cmd-w", "cmd-c"] {
            assert!(
                matches!(b.scroll_key(&key(other)), BodyAction::Ignored),
                "{other}"
            );
        }
    }

    // A web link goes to the browser, a document beside this one opens as
    // a page of its own.
    #[test]
    fn links_open_the_web_outside_and_documents_as_pages() {
        let (b, dir) = page("x");
        match b.follow("https://infiniterm.app/") {
            BodyAction::OpenExternal { url, .. } => {
                assert_eq!(url.as_deref(), Some("https://infiniterm.app/"))
            }
            _ => panic!("web link"),
        }
        match b.follow("canvas.md#keys") {
            BodyAction::Open(infiniterm_core::ift::OpenPlan::Page { path, .. }) => {
                assert_eq!(path, dir.join("canvas.md").to_string_lossy())
            }
            _ => panic!("page link"),
        }
    }
}
