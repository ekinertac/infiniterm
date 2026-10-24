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
use infiniterm_core::file_source::{Code, FileSource};
use infiniterm_core::grid::{Point, Size};
use infiniterm_core::page::{layout, parse, Block, Line, LineKind};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

const PAD_X: f64 = 14.;
const PAD_Y: f64 = 10.;
/// Prose reads better looser than a terminal's rows.
const LINE: f64 = 1.45;
const DISK_POLL_MS: f64 = 2000.;
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
    /// Where the document is: this machine's disk, or a host's (#560).
    source: Arc<dyn FileSource>,
    /// A read of a host's file in flight; its answer is taken in `idle`.
    pending: Option<Receiver<PageLoad>>,
    /// Whether a document has been shown yet: a failed first load says so on
    /// the page, a failed poll keeps what is there.
    loaded_once: bool,
}

/// What a host read answered.
enum PageLoad {
    /// The file's mtime is the one already shown.
    Unchanged,
    Loaded {
        mtime: Option<u64>,
        text: String,
    },
    Failed {
        code: Code,
        message: String,
    },
}

/// The page's own text for a document that could not be read.
fn cannot_read(path: &str, why: &str) -> String {
    format!("# Cannot read this page\n\n`{path}`: {why}")
}

impl PageBody {
    pub fn new(
        path: String,
        metrics: &Metrics,
        colors: PageColors,
        world: Size,
        source: Arc<dyn FileSource>,
    ) -> PageBody {
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
            source,
            pending: None,
            loaded_once: false,
        };
        b.reload();
        b
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.laid_cols = 0;
    }

    /// Re-reads the document when it changed on disk (`DISK_POLL_MS`), and
    /// takes the answer of a host read that was asked for earlier.
    pub fn idle(&mut self, now: f64) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(answer) => {
                    self.pending = None;
                    self.apply(answer);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.pending = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
            }
        }
        if now - self.last_poll < DISK_POLL_MS {
            return;
        }
        self.last_poll = now;
        if self.source.is_local() {
            if self.source.stat(&self.path).ok().and_then(|s| s.mtime_ms) != self.mtime {
                self.reload();
            }
        } else {
            self.ask_host();
        }
    }

    /// Loads the document. A local file is read now, as it always was; a
    /// host's is asked for on a thread (it comes over ssh) and shown when it
    /// arrives, with a line saying so meanwhile.
    fn reload(&mut self) {
        if self.source.is_local() {
            let answer = match self.source.read(&self.path) {
                Ok(d) => PageLoad::Loaded {
                    mtime: d.mtime_ms,
                    text: d.text(),
                },
                Err(e) => PageLoad::Failed {
                    code: e.code,
                    message: e.message,
                },
            };
            self.apply(answer);
        } else {
            if !self.loaded_once {
                self.blocks = parse("*Loading from the host...*");
                self.laid_cols = 0;
                self.dirty = true;
            }
            self.ask_host();
        }
    }

    /// One read of the host's file: its mtime first, the text only when that
    /// is not the one shown.
    fn ask_host(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = channel();
        self.pending = Some(rx);
        let (source, path, known) = (self.source.clone(), self.path.clone(), self.mtime);
        let first = !self.loaded_once;
        std::thread::spawn(move || {
            let answer = (|| {
                if !first {
                    let stat = source.stat(&path)?;
                    if stat.mtime_ms == known {
                        return Ok(PageLoad::Unchanged);
                    }
                }
                let d = source.read(&path)?;
                Ok::<_, infiniterm_core::file_source::FileError>(PageLoad::Loaded {
                    mtime: d.mtime_ms,
                    text: d.text(),
                })
            })()
            .unwrap_or_else(|e| PageLoad::Failed {
                code: e.code,
                message: e.message,
            });
            let _ = tx.send(answer);
        });
    }

    fn apply(&mut self, answer: PageLoad) {
        match answer {
            PageLoad::Unchanged => {}
            PageLoad::Loaded { mtime, text } => {
                self.mtime = mtime;
                self.blocks = parse(&text);
                self.loaded_once = true;
            }
            // A shown document stays shown while the host cannot be reached;
            // a first load says what went wrong on the page itself.
            PageLoad::Failed { code, message } => {
                if self.loaded_once && code == Code::Lost {
                    return;
                }
                self.mtime = None;
                self.blocks = parse(&cannot_read(&self.path, &message));
                self.loaded_once = true;
            }
        }
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
        // A document of a host's: the card that shows it is the host's too,
        // so the host is asked to open it (#555), never a card of this
        // window's own copy of the canvas.
        if !self.source.is_local() {
            return BodyAction::OpenOnHost { path, line: None };
        }
        BodyAction::Open(infiniterm_core::ift::OpenPlan::Page {
            cwd: base.to_string_lossy().into_owned(),
            path,
        })
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
        window.paint_quad(crate::chrome::card_body_quad(
            cx,
            bounds,
            self.colors.background,
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
        crate::scrollbar::paint(
            window,
            bounds,
            self.lines.len(),
            self.visible(),
            self.scroll,
            self.colors.faint,
        );
        if !focused && self.inactive_dim > 0. {
            window.paint_quad(crate::chrome::card_wash(
                cx,
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
            Arc::new(infiniterm_core::file_source::LocalSource),
        );
        (b, dir)
    }

    /// A source that is not local, answering from a script of replies, so the
    /// page's way of asking a host can be tested with no ssh.
    type Reply = Result<(String, Option<u64>), infiniterm_core::file_source::FileError>;

    struct Scripted {
        replies: std::sync::Mutex<Vec<Reply>>,
        reads: std::sync::atomic::AtomicUsize,
        stats: std::sync::atomic::AtomicUsize,
        mtime: std::sync::Mutex<Option<u64>>,
    }

    impl FileSource for Scripted {
        fn is_local(&self) -> bool {
            false
        }
        fn home(
            &self,
        ) -> Result<infiniterm_core::file_source::HostPaths, infiniterm_core::file_source::FileError>
        {
            unreachable!()
        }
        fn stat(
            &self,
            _: &str,
        ) -> Result<infiniterm_core::file_source::Stat, infiniterm_core::file_source::FileError>
        {
            self.stats.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(infiniterm_core::file_source::Stat {
                kind: infiniterm_core::file_source::EntryKind::File,
                size: 0,
                mtime_ms: *self.mtime.lock().unwrap(),
                symlink: false,
            })
        }
        fn exists(
            &self,
            _: &[String],
        ) -> Result<
            Vec<Option<infiniterm_core::file_source::EntryKind>>,
            infiniterm_core::file_source::FileError,
        > {
            unreachable!()
        }
        fn list(
            &self,
            _: &str,
        ) -> Result<
            Vec<infiniterm_core::file_source::ListEntry>,
            infiniterm_core::file_source::FileError,
        > {
            unreachable!()
        }
        fn read(
            &self,
            _: &str,
        ) -> Result<infiniterm_core::file_source::FileData, infiniterm_core::file_source::FileError>
        {
            self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // Slow, as an ssh is: the body must not be waiting on it.
            std::thread::sleep(std::time::Duration::from_millis(150));
            let next = self.replies.lock().unwrap().remove(0);
            next.map(|(text, mtime_ms)| infiniterm_core::file_source::FileData {
                bytes: text.into_bytes(),
                mtime_ms,
            })
        }
    }

    fn host_page(replies: Vec<Reply>) -> (PageBody, Arc<Scripted>) {
        let src = Arc::new(Scripted {
            replies: std::sync::Mutex::new(replies),
            reads: Default::default(),
            stats: Default::default(),
            mtime: std::sync::Mutex::new(Some(1)),
        });
        let (local, _) = page("x");
        let mut b = local;
        b.source = src.clone();
        b.pending = None;
        b.loaded_once = false;
        b.mtime = None;
        b.reload();
        (b, src)
    }

    /// Waits until the host's answer is taken.
    fn settle(b: &mut PageBody) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while b.pending.is_some() && std::time::Instant::now() < deadline {
            b.idle(0.);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(b.pending.is_none(), "the host's answer never came");
    }

    fn text_of(b: &PageBody) -> String {
        format!("{:?}", b.blocks)
    }

    // A host's document is asked for on a thread: the body shows a line saying
    // so at once, never waits, and shows the document when it arrives.
    #[test]
    fn a_host_page_loads_without_waiting_and_then_shows_the_document() {
        let started = std::time::Instant::now();
        let (mut b, src) = host_page(vec![Ok(("# From the host\n\nbody".into(), Some(1)))]);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(100),
            "the first call waited on the host"
        );
        assert!(
            text_of(&b).contains("Loading from the host"),
            "{}",
            text_of(&b)
        );
        settle(&mut b);
        assert!(text_of(&b).contains("From the host"));
        assert_eq!(src.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(b.dirty, "a frame is owed for the new text");
    }

    // The poll asks only for the file's mtime; the text is read again when it
    // moved, and a host that cannot be reached leaves the page as it was.
    #[test]
    fn a_host_page_rereads_only_when_the_mtime_moved_and_keeps_its_text_when_the_host_is_lost() {
        let (mut b, src) = host_page(vec![
            Ok(("first".into(), Some(1))),
            Ok(("second".into(), Some(2))),
            Err(infiniterm_core::file_source::FileError::new(
                Code::Lost,
                "gone",
            )),
        ]);
        settle(&mut b);
        let reads = || src.reads.load(std::sync::atomic::Ordering::SeqCst);
        // Same mtime: one stat, no read.
        b.last_poll = -1e9;
        b.idle(DISK_POLL_MS * 10.);
        settle(&mut b);
        assert_eq!(reads(), 1);
        assert!(src.stats.load(std::sync::atomic::Ordering::SeqCst) >= 1);
        // The mtime moved: the text is read again.
        *src.mtime.lock().unwrap() = Some(2);
        b.last_poll = -1e9;
        b.idle(DISK_POLL_MS * 20.);
        settle(&mut b);
        assert_eq!(reads(), 2);
        assert!(text_of(&b).contains("second"));
        // The mtime moves once more and the host is gone: the page stays.
        *src.mtime.lock().unwrap() = Some(3);
        b.last_poll = -1e9;
        b.idle(DISK_POLL_MS * 30.);
        settle(&mut b);
        assert!(
            text_of(&b).contains("second"),
            "a lost host must not blank the page"
        );
    }

    // The first load failing says why on the page, in the page's own words.
    #[test]
    fn a_host_page_that_cannot_be_read_says_so_on_the_page() {
        let (mut b, _) = host_page(vec![Err(infiniterm_core::file_source::FileError::new(
            Code::NotFound,
            "/x/y.md: No such file or directory",
        ))]);
        settle(&mut b);
        let t = text_of(&b);
        assert!(
            t.contains("Cannot read this page") && t.contains("No such file"),
            "{t}"
        );
    }

    // A document linked from a host's page is the host's: it is opened there.
    #[test]
    fn a_link_from_a_host_page_asks_the_host_and_never_makes_a_local_card() {
        let (b, _) = host_page(vec![Ok(("x".into(), Some(1)))]);
        let mut b = b;
        b.path = "/srv/docs/index.md".into();
        match b.follow("other.md#top") {
            BodyAction::OpenOnHost { path, line } => {
                assert_eq!(path, "/srv/docs/other.md");
                assert_eq!(line, None);
            }
            _ => panic!("a host page's link must go to the host"),
        }
        // A web address still goes to the browser on this Mac.
        assert!(matches!(
            b.follow("https://infiniterm.app/"),
            BodyAction::OpenExternal { .. }
        ));
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
