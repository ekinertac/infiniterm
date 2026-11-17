//! The pure rules of the webview browser card (`browser.engine` = `webview`,
//! #761): which store a page keeps its sign-ins in, how the canvas zoom and
//! the page's own zoom make one WebKit page zoom, when the page is drawn as
//! a still picture instead of live, how often that picture is refreshed, and
//! which cards are drawn over which.
//!
//! Why here and not next to the engine: the engine (`infiniterm-ui/src/
//! webview.rs`) is AppKit calls and cannot run in a test; every decision it
//! makes is made here, with tests, and it only carries them out.
//!
//! Where it fits: `webview.rs` and `browser_body.rs` in the ui crate ask it
//! each frame; `paint.rs` fills in the covers before the bodies paint.
//! Related: `docs/agent/browser-and-pages.md` (the webview engine) and
//! `docs/agent/webview-spike.md` on the spike branch (#812) for the
//! measurements behind the numbers.

/// Effective page zoom under which WebKit stops shrinking text (it keeps
/// text from going under about 9 px), so a live page stops matching its card
/// and a picture is drawn instead. Measured in the #761 spike: sharp at 0.5
/// and above, reflowed and cropped at 0.25.
pub const FAR_ZOOM: f64 = 0.5;

/// WebKit's own limits for `pageZoom`, which a canvas zoom must stay inside
/// or the page would be asked for a value WebKit then clamps.
pub const PAGE_ZOOM_MIN: f64 = 0.1;
pub const PAGE_ZOOM_MAX: f64 = 5.0;

/// The page zoom WebKit gets: the canvas zoom times the page's own zoom
/// (`card.zoom`, the browser zoom keys), so the page lays out at the card's
/// world width divided by its own zoom.
pub fn page_zoom(card_zoom: Option<f64>, canvas_scale: f64) -> f64 {
    (card_zoom.unwrap_or(1.) * canvas_scale).clamp(PAGE_ZOOM_MIN, PAGE_ZOOM_MAX)
}

/// What the card knows about itself in one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageState {
    /// The page zoom from `page_zoom`.
    pub zoom: f64,
    /// One cover hides the whole page (an open overlay, a card bigger than
    /// it): nothing of the live page would show, so the picture stands in.
    /// Partial covers are not this: they are holes in the live page
    /// (`holes_of`), which keeps it playing and scrolling.
    pub fully_covered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageView {
    /// The live WebKit view is shown.
    Live,
    /// The view is hidden and a picture of the page is drawn by gpui.
    Snapshot,
}

/// A native view floats above everything gpui draws. A card over part of the
/// page, an open menu's rect or the drag ghost are cut out of the live view
/// as holes (`holes_of`); only a page that cannot be shown at this zoom, or
/// that one cover hides entirely, is swapped for its picture.
pub fn page_view(s: &PageState) -> PageView {
    if s.zoom < FAR_ZOOM || s.fully_covered {
        PageView::Snapshot
    } else {
        PageView::Live
    }
}

/// Which view should take the keyboard when a page switches between live
/// and its picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeysTo {
    /// The web view.
    Page,
    /// gpui's own view, where the app's keys and its double-Escape live.
    App,
}

/// A page hidden for its picture cannot keep the keyboard: AppKit gives a
/// hidden first responder's keys to the window itself, which passes them to
/// nobody, so a locked card in the fit-all view ignored Escape (#837). Going
/// to the picture hands the keys to the app (the caller does it only if this
/// page had them); coming back live hands them back to a locked page. Every
/// other frame changes nothing, so a find bar typed into over a live page
/// keeps the keys.
pub fn keys_on_view_change(was: Option<PageView>, now: PageView, locked: bool) -> Option<KeysTo> {
    match (was, now) {
        (Some(PageView::Live), PageView::Snapshot) => Some(KeysTo::App),
        (Some(PageView::Snapshot), PageView::Live) if locked => Some(KeysTo::Page),
        _ => None,
    }
}

/// Alpha of the colour wash over an unfocused page (`ui.inactiveDim`). The
/// webview's own alpha does nothing, so the wash is a view above it.
pub fn dim_alpha(focused: bool, inactive_dim: f64) -> f64 {
    if focused {
        0.
    } else {
        inactive_dim.clamp(0., 1.)
    }
}

/// How old a snapshot may be before another is taken, while the page is
/// live. A picture is only seen when the page is covered, dragged or far
/// away, so it has to be fresh BEFORE then; the call is asynchronous.
pub const SNAPSHOT_MAX_AGE_MS: f64 = 2000.;
/// A page that just finished loading is snapshotted after this pause, for
/// its first paint to settle.
pub const SNAPSHOT_AFTER_LOAD_MS: f64 = 400.;

/// Whether to take a snapshot now. `live` is whether the view is shown (a
/// hidden view's snapshot would be of nothing), `request_pending` whether
/// one was asked for and has not come back.
pub fn snapshot_due(
    live: bool,
    request_pending: bool,
    last_taken_ms: Option<f64>,
    loaded_at_ms: Option<f64>,
    now_ms: f64,
) -> bool {
    if !live || request_pending {
        return false;
    }
    match last_taken_ms {
        None => true,
        Some(t) => {
            let after_load =
                loaded_at_ms.is_some_and(|l| l > t && now_ms - l >= SNAPSHOT_AFTER_LOAD_MS);
            now_ms - t >= SNAPSHOT_MAX_AGE_MS || after_load
        }
    }
}

/// A rect in window pixels: x, y, width, height.
pub type Rect = [f64; 4];

pub fn overlap(a: &Rect, b: &Rect) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// The cards drawn over card `index`: those after it in paint order whose
/// rect meets it. `order` is every visible card's rect in paint order (the
/// cards being moved last, as `paint.rs` draws them).
pub fn covers_of(order: &[Rect], index: usize) -> Vec<Rect> {
    let Some(me) = order.get(index) else {
        return vec![];
    };
    order[index + 1..]
        .iter()
        .filter(|r| overlap(me, r))
        .cloned()
        .collect()
}

/// Whether one rect contains another.
pub fn contains(outer: &Rect, inner: &Rect) -> bool {
    outer[0] <= inner[0]
        && outer[1] <= inner[1]
        && outer[0] + outer[2] >= inner[0] + inner[2]
        && outer[1] + outer[3] >= inner[1] + inner[3]
}

/// Whether one of the covers hides the whole page.
pub fn fully_covered(page: &Rect, covers: &[Rect]) -> bool {
    covers.iter().any(|c| contains(c, page))
}

/// The parts of `page` the covers hide, as rects that do not overlap: the
/// holes of the layer mask. Two covers that meet (a card and the drag ghost)
/// must not become two overlapping holes: an even-odd mask fills an overlap
/// again, so the page showed through where two covers met (found in the #761
/// spike). Coordinate compression: cut the plane along every edge, keep the
/// cells inside any cover.
pub fn holes_of(page: &Rect, covers: &[Rect]) -> Vec<Rect> {
    let clipped: Vec<Rect> = covers
        .iter()
        .filter_map(|c| {
            let x0 = c[0].max(page[0]);
            let y0 = c[1].max(page[1]);
            let x1 = (c[0] + c[2]).min(page[0] + page[2]);
            let y1 = (c[1] + c[3]).min(page[1] + page[3]);
            (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
        })
        .collect();
    let mut xs: Vec<f64> = clipped.iter().flat_map(|h| [h[0], h[0] + h[2]]).collect();
    let mut ys: Vec<f64> = clipped.iter().flat_map(|h| [h[1], h[1] + h[3]]).collect();
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    xs.dedup();
    ys.dedup();
    let mut out = Vec::new();
    for xw in xs.windows(2) {
        for yw in ys.windows(2) {
            let mid = (xw[0] + (xw[1] - xw[0]) / 2., yw[0] + (yw[1] - yw[0]) / 2.);
            let hit = clipped.iter().any(|h| {
                mid.0 >= h[0] && mid.0 < h[0] + h[2] && mid.1 >= h[1] && mid.1 < h[1] + h[3]
            });
            if hit {
                out.push([xw[0], yw[0], xw[1] - xw[0], yw[1] - yw[0]]);
            }
        }
    }
    out
}

/// The 16 bytes that name a WKWebsiteDataStore (`dataStoreForIdentifier`,
/// macOS 14+) for one data dir. WebKit keeps the store under
/// `~/Library/WebKit/<bundle id>/WebsiteDataStore/<uuid>/` whatever we do,
/// so the data dir cannot hold it; this names it instead, and the real app
/// and every scratch instance (`INFINITERM_DATA_DIR`) get a store of their
/// own. THE VALUE MUST NEVER CHANGE for a given path: it is where the
/// person's sign-ins live (a test pins one).
///
/// Not security: two FNV-1a 64 passes with different bases, spread over 16
/// bytes, then the UUID version (4) and variant bits set so WebKit accepts
/// it as a UUID. No crate: the std hasher is not stable across releases.
pub fn store_identifier(data_dir: &str) -> [u8; 16] {
    fn fnv(basis: u64, bytes: &[u8]) -> u64 {
        let mut h = basis;
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }
    let a = fnv(0xcbf2_9ce4_8422_2325, data_dir.as_bytes());
    let b = fnv(0x84222325_cbf29ce4, data_dir.as_bytes());
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&a.to_be_bytes());
    out[8..].copy_from_slice(&b.to_be_bytes());
    out[6] = (out[6] & 0x0f) | 0x40;
    out[8] = (out[8] & 0x3f) | 0x80;
    out
}

/// The Safari that ships with a macOS major version: 11 is Safari 14 up to
/// 15 is Safari 18, and from macOS 26 the two share a number.
pub fn safari_major(macos_major: i64) -> i64 {
    if macos_major >= 26 {
        macos_major
    } else {
        macos_major + 3
    }
}

/// The user agent a webview page sends: WKWebView's own (frozen `10_15_7`,
/// AppleWebKit/605.1.15) plus the `Version/N Safari/605.1.15` tail Safari
/// adds. Without the tail WebKit's default says "an app's web view", which
/// some sites (video and music players, sign-in flows) serve a reduced page
/// to; with it a page sees Safari, which is what this engine is. Google's
/// sign-in page served normally with either (#761 spike and this engine).
pub fn safari_user_agent(macos_major: i64) -> String {
    format!(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{}.0 Safari/605.1.15",
        safari_major(macos_major)
    )
}

/// What the engine logs when it cannot use a store of its own (macOS older
/// than 14): one shared non-persistent store, so nothing is written to the
/// global default store and nothing survives a restart.
pub const NO_CUSTOM_STORE_NOTE: &str =
    "webview engine: macOS before 14 has no per-instance data store; pages use one shared store that is not kept";

/// What the DevTools command says on the webview engine: WebKit's inspector
/// cannot be docked into a card (attached, it takes over the whole window, as
/// the spike found), so the page is made inspectable and Safari's Develop
/// menu is the way in.
pub const NATIVE_DEVTOOLS_NOTE: &str = "DevTools for a webview page: open Safari, then Develop > this Mac > the page (turn on Develop in Safari's Advanced settings once)";

/// The message the page's `window.close()` posts to the host, and the script
/// that makes it do so: WKWebView has no close callback wry exposes, and a
/// tab a page opened must close when the page asks (#672).
pub const CLOSE_MESSAGE: &str = "infiniterm:close";
pub const CLOSE_SCRIPT: &str =
    "window.close = function () { window.ipc.postMessage('infiniterm:close'); };";

/// Where a download goes: the folder WebKit proposed, under the name it
/// proposed with any path stripped, and never over a file that is there
/// (`name (1).ext`, `name (2).ext`...). `taken` says whether a path exists.
pub fn download_destination(
    dir: &std::path::Path,
    suggested: &str,
    taken: impl Fn(&std::path::Path) -> bool,
) -> std::path::PathBuf {
    let name = suggested
        .rsplit(['/', '\\'])
        .next()
        .filter(|n| !n.trim().is_empty() && *n != "." && *n != "..")
        .unwrap_or("download");
    let first = dir.join(name);
    if !taken(&first) {
        return first;
    }
    let path = std::path::Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = path.extension().and_then(|e| e.to_str());
    (1..)
        .map(|n| match ext {
            Some(e) => dir.join(format!("{stem} ({n}).{e}")),
            None => dir.join(format!("{stem} ({n})")),
        })
        .find(|p| !taken(p))
        .expect("an unbounded range has a free name")
}

/// JS that counts the occurrences of `text` in the page, case-insensitively,
/// as a number: WebKit's find says only whether there was a match, and the
/// find bar shows "n of m". The text goes in as a JSON string, so a quote or
/// a backslash in it cannot end the script.
pub fn find_count_js(text: &str) -> String {
    let lit = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
    format!(
        "(function(){{var t={lit}.toLowerCase();if(!t)return 0;var h=((document.body&&document.body.innerText)||'').toLowerCase();var n=0,i=0;while((i=h.indexOf(t,i))!==-1){{n++;i+=t.length;}}return n;}})()"
    )
}

/// The count `find_count_js` returned, however the engine wrapped it.
pub fn parse_count(raw: &str) -> i32 {
    raw.trim().trim_matches('"').parse().unwrap_or(0).max(0)
}

/// The next current match after a find step, wrapping: `matches` is the
/// count, `active` the 1-based current one (0 for none).
pub fn step_find(active: i32, matches: i32, forward: bool) -> i32 {
    if matches <= 0 {
        return 0;
    }
    if forward {
        if active >= matches {
            1
        } else {
            active + 1
        }
    } else if active <= 1 {
        matches
    } else {
        active - 1
    }
}

/// What a right-click landed on, for the slim menu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextInfo {
    pub link: Option<String>,
    pub editable: bool,
    pub selection: bool,
}

/// Two helpers the page scripts share: the element at a point, looking into
/// shadow roots (a plain `elementFromPoint` stops at a web component's
/// host), and the link above an element, walking out of shadow roots through
/// their hosts (`closest` stops at the root).
const POINT_HELPERS_JS: &str = "function deep(x,y){var e=document.elementFromPoint(x,y);while(e&&e.shadowRoot){var i=e.shadowRoot.elementFromPoint(x,y);if(!i||i===e)break;e=i;}return e;}function up(e){while(e){if(e.matches&&e.matches('a[href]'))return e;e=e.parentElement||(e.getRootNode&&e.getRootNode().host)||null;}return null;}";

/// JS for the element at a CSS pixel point: the link above it, whether it is
/// editable, whether text is selected. Returns a JSON string.
pub fn context_js(x: f64, y: f64) -> String {
    format!(
        "(function(){{{POINT_HELPERS_JS}var e=deep({x:.1},{y:.1});var a=up(e);var ed=!!(e&&(e.isContentEditable||(e.closest&&e.closest('input,textarea,[contenteditable=\"true\"]'))));var s=window.getSelection?String(window.getSelection()):'';return JSON.stringify({{link:a?a.href:null,editable:ed,selection:s.length>0}});}})()"
    )
}

/// JS for the address of the link at a CSS pixel point, or "" when there is
/// none: what a Cmd+click or a middle-click opens in a new tab (#821). The
/// system web view reports the modifier flags only on a navigation, which wry
/// does not pass on; the app already tells a click from a drag (the press of
/// a Cmd or middle click is the canvas's until it is released unmoved), so it
/// asks the page for the link instead.
pub fn link_js(x: f64, y: f64) -> String {
    format!("(function(){{{POINT_HELPERS_JS}var a=up(deep({x:.1},{y:.1}));return a?a.href:'';}})()")
}

/// The address `link_js` found, however many times the engine quoted it.
pub fn parse_link(raw: &str) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
    for _ in 0..4 {
        match value {
            serde_json::Value::String(inner) => match serde_json::from_str(&inner) {
                Ok(next) => value = next,
                // Not JSON any more: the string is the answer.
                Err(_) => return Some(inner).filter(|s| !s.is_empty()),
            },
            _ => return None,
        }
    }
    None
}

/// What a Cmd+click, Cmd+Shift+click or middle-click on a link does, as in
/// Chrome (every non-current-tab disposition opens a tab): the link, unless
/// it is another app's, which goes to the app-link question. Only a page
/// address opens a tab: `javascript:` and `data:` links, which `a.href`
/// reports like any other, would run or show script-made content in a tab of
/// their own, and Chrome does not open them either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkOpen {
    Tab(String),
    App(String),
    /// A scheme no tab opens: dropped without a word.
    Ignore,
}

/// The schemes a link may open a tab on: web pages, local files and
/// `about:`. A web view's `a.href` carries every scheme, so the list is
/// explicit.
const TAB_SCHEMES: &[&str] = &["http", "https", "file", "about"];

fn link_scheme(href: &str) -> Option<String> {
    let (scheme, _) = href.trim().split_once(':')?;
    (!scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then(|| scheme.to_ascii_lowercase())
}

pub fn classify_link(href: &str) -> LinkOpen {
    if crate::app_link::is_app_link(href) {
        return LinkOpen::App(href.to_string());
    }
    match link_scheme(href) {
        Some(s) if TAB_SCHEMES.contains(&s.as_str()) => LinkOpen::Tab(href.to_string()),
        _ => LinkOpen::Ignore,
    }
}

/// Whether a click asks for a new tab: Cmd (with or without Shift) or the
/// middle button.
pub fn opens_in_new_tab(command: bool, middle: bool) -> bool {
    command || middle
}

/// The answer to `context_js`. The engine hands a script's string result
/// back JSON-encoded, so the text may be wrapped once more in quotes.
pub fn parse_context(raw: &str) -> Option<ContextInfo> {
    let mut value: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
    // Wrapped as many times as the engine encoded it (seen twice on macOS:
    // the script's own JSON string, quoted again by wry's result encoder).
    for _ in 0..4 {
        match value {
            serde_json::Value::String(inner) => value = serde_json::from_str(&inner).ok()?,
            _ => break,
        }
    }
    Some(ContextInfo {
        link: value
            .get("link")
            .and_then(|l| l.as_str())
            .filter(|l| !l.is_empty())
            .map(str::to_string),
        editable: value
            .get("editable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        selection: value
            .get("selection")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

/// How far (screen pixels) a natural size may be from the available one and
/// still count as the same: the card's bounds are fractional and a layout
/// size is whole CSS pixels, so a settled card is up to about a pixel off.
/// Treating that as equal keeps a picture flush with its card instead of
/// leaving a one-pixel seam.
const SNAP_PX: f64 = 1.5;

/// The size, in screen pixels, to draw a page picture (a snapshot) at, so a
/// card being resized never stretches an old one (#479, moved here from the
/// CEF crate in #826). `css_w` and `css_h` are the layout size the picture
/// was taken at (0 when unknown), `scale` the canvas zoom (screen pixels per
/// CSS pixel), `avail_w` and `avail_h` the page area of the card now. The
/// picture is drawn at its layout size times the zoom, anchored top-left, so
/// the card shows a page of the old size, clipped or with card background
/// beside it, until a new picture replaces it. A canvas ZOOM is not a
/// stretch: the layout size is unchanged, so a settled card fills exactly.
/// Unknown layout size: the available size.
pub fn frame_draw_size(
    css_w: i32,
    css_h: i32,
    scale: f64,
    avail_w: f64,
    avail_h: f64,
) -> (f64, f64) {
    if css_w <= 0 || css_h <= 0 || scale <= 0. {
        return (avail_w, avail_h);
    }
    let snap = |natural: f64, avail: f64| {
        if (natural - avail).abs() <= SNAP_PX {
            avail
        } else {
            natural
        }
    };
    (
        snap(css_w as f64 * scale, avail_w),
        snap(css_h as f64 * scale, avail_h),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settled_card_is_filled_exactly() {
        assert_eq!(frame_draw_size(800, 600, 1., 800., 600.), (800., 600.));
        // Fractional bounds a hair off the rounded layout size still fill.
        assert_eq!(frame_draw_size(800, 600, 1., 800.4, 599.6), (800.4, 599.6));
    }

    #[test]
    fn a_zoom_is_not_a_stretch() {
        for z in [0.25, 0.5, 1., 1.5, 3.] {
            assert_eq!(
                frame_draw_size(800, 600, z, 800. * z, 600. * z),
                (800. * z, 600. * z),
                "zoom {z}"
            );
        }
    }

    #[test]
    fn a_card_dragged_wider_or_narrower_keeps_the_old_picture_size() {
        assert_eq!(frame_draw_size(800, 600, 1., 1000., 600.), (800., 600.));
        assert_eq!(frame_draw_size(800, 600, 1., 500., 400.), (800., 600.));
    }

    #[test]
    fn the_natural_size_follows_the_zoom_during_a_resize() {
        assert_eq!(frame_draw_size(800, 600, 0.5, 600., 300.), (400., 300.));
    }

    #[test]
    fn a_picture_with_no_known_layout_fills_the_card() {
        assert_eq!(frame_draw_size(0, 0, 1., 640., 480.), (640., 480.));
        assert_eq!(frame_draw_size(800, 600, 0., 640., 480.), (640., 480.));
    }

    #[test]
    fn a_download_keeps_the_proposed_name_when_the_path_is_free() {
        let d = std::path::Path::new("/dl");
        assert_eq!(
            download_destination(d, "report.pdf", |_| false),
            std::path::PathBuf::from("/dl/report.pdf")
        );
    }

    #[test]
    fn a_download_never_goes_over_a_file_that_is_there() {
        let d = std::path::Path::new("/dl");
        let taken = |p: &std::path::Path| {
            p == std::path::Path::new("/dl/a.txt") || p == std::path::Path::new("/dl/a (1).txt")
        };
        assert_eq!(
            download_destination(d, "a.txt", taken),
            std::path::PathBuf::from("/dl/a (2).txt")
        );
        let taken_no_ext = |p: &std::path::Path| p == std::path::Path::new("/dl/data");
        assert_eq!(
            download_destination(d, "data", taken_no_ext),
            std::path::PathBuf::from("/dl/data (1)")
        );
    }

    #[test]
    fn a_proposed_name_cannot_leave_the_folder() {
        let d = std::path::Path::new("/dl");
        for bad in ["../../etc/passwd", "a/b/c.txt", "..", "", "  "] {
            let p = download_destination(d, bad, |_| false);
            assert_eq!(p.parent(), Some(d), "{bad:?} gave {p:?}");
        }
        assert_eq!(
            download_destination(d, "../../etc/passwd", |_| false),
            std::path::PathBuf::from("/dl/passwd")
        );
        assert_eq!(
            download_destination(d, "", |_| false),
            std::path::PathBuf::from("/dl/download")
        );
    }

    #[test]
    fn the_find_script_carries_the_text_as_one_json_string() {
        let js = find_count_js("say \"hi\"\\n");
        assert!(js.contains(r#""say \"hi\"\\n""#), "{js}");
        // A closing quote in the text does not end the string early: it is
        // escaped inside the one JSON literal.
        let hostile = find_count_js("\"); alert(1); (\"");
        assert!(
            hostile.contains(r#"var t="\"); alert(1); (\""."#),
            "{hostile}"
        );
    }

    #[test]
    fn a_count_is_read_from_a_number_or_a_quoted_one() {
        assert_eq!(parse_count("3"), 3);
        assert_eq!(parse_count("\"12\""), 12);
        assert_eq!(parse_count("null"), 0);
        assert_eq!(parse_count("-4"), 0);
    }

    #[test]
    fn find_steps_wrap_both_ways() {
        assert_eq!(step_find(0, 0, true), 0);
        assert_eq!(step_find(1, 3, true), 2);
        assert_eq!(step_find(3, 3, true), 1);
        assert_eq!(step_find(1, 3, false), 3);
        assert_eq!(step_find(2, 3, false), 1);
    }

    #[test]
    fn a_context_answer_is_read_plain_or_wrapped() {
        let plain = r#"{"link":"https://a.test/x","editable":false,"selection":true}"#;
        let want = ContextInfo {
            link: Some("https://a.test/x".into()),
            editable: false,
            selection: true,
        };
        assert_eq!(parse_context(plain), Some(want.clone()));
        let wrapped = serde_json::to_string(plain).unwrap();
        assert_eq!(parse_context(&wrapped), Some(want.clone()));
        // Quoted twice, as the engine really answered.
        let twice = serde_json::to_string(&wrapped).unwrap();
        assert_eq!(parse_context(&twice), Some(want));
        assert_eq!(
            parse_context(r#"{"link":null,"editable":true,"selection":false}"#),
            Some(ContextInfo {
                link: None,
                editable: true,
                selection: false
            })
        );
        assert_eq!(parse_context("garbage"), None);
    }

    #[test]
    fn the_context_script_asks_for_the_point_it_is_given() {
        let js = context_js(12.34, 56.78);
        assert!(js.contains("deep(12.3,56.8)"), "{js}");
    }

    #[test]
    fn the_link_script_looks_into_shadow_roots_at_the_given_point() {
        let js = link_js(1.26, 2.0);
        assert!(js.contains("up(deep(1.3,2.0))"), "{js}");
        assert!(js.contains("shadowRoot"), "{js}");
        assert!(js.contains("getRootNode().host"), "{js}");
    }

    #[test]
    fn a_link_answer_is_read_plain_or_quoted_and_empty_is_no_link() {
        assert_eq!(
            parse_link("\"https://a.test/x\""),
            Some("https://a.test/x".into())
        );
        // Quoted a second time, as the engine does with a string result.
        let twice = serde_json::to_string("\"https://a.test/x\"").unwrap();
        assert_eq!(parse_link(&twice), Some("https://a.test/x".into()));
        assert_eq!(parse_link("\"\""), None);
        assert_eq!(parse_link("null"), None);
        assert_eq!(parse_link("garbage"), None);
    }

    #[test]
    fn only_page_addresses_open_a_tab_and_script_links_are_dropped() {
        for ok in [
            "http://a.test/",
            "HTTPS://a.test/x?y=1",
            "file:///tmp/a.html",
            "about:blank",
        ] {
            assert_eq!(classify_link(ok), LinkOpen::Tab(ok.into()), "{ok}");
        }
        for dropped in [
            "javascript:alert(1)",
            "JavaScript:void(0)",
            "data:text/html,<b>x</b>",
            "blob:https://a.test/uuid",
            "",
            "no scheme at all",
            ":empty",
        ] {
            assert_eq!(classify_link(dropped), LinkOpen::Ignore, "{dropped:?}");
        }
        // Another app's scheme is still the app-link question, not dropped.
        assert_eq!(
            classify_link("slack://open"),
            LinkOpen::App("slack://open".into())
        );
    }

    #[test]
    fn a_link_opens_as_a_tab_unless_it_is_another_apps() {
        assert_eq!(
            classify_link("https://a.test/x"),
            LinkOpen::Tab("https://a.test/x".into())
        );
        assert_eq!(
            classify_link("slack://open"),
            LinkOpen::App("slack://open".into())
        );
        assert_eq!(
            classify_link("mailto:a@b.c"),
            LinkOpen::App("mailto:a@b.c".into())
        );
    }

    #[test]
    fn cmd_and_middle_click_ask_for_a_new_tab_a_plain_click_does_not() {
        assert!(opens_in_new_tab(true, false));
        assert!(opens_in_new_tab(false, true));
        assert!(!opens_in_new_tab(false, false));
    }

    #[test]
    fn the_page_zoom_is_the_canvas_zoom_times_the_pages_own() {
        assert_eq!(page_zoom(None, 1.), 1.);
        assert_eq!(page_zoom(Some(1.5), 2.), 3.);
        assert_eq!(page_zoom(Some(0.5), 0.5), 0.25);
    }

    #[test]
    fn the_safari_version_follows_the_macos_version() {
        assert_eq!(safari_major(11), 14);
        assert_eq!(safari_major(14), 17);
        assert_eq!(safari_major(15), 18);
        assert_eq!(safari_major(26), 26);
        assert_eq!(safari_major(27), 27);
    }

    #[test]
    fn the_user_agent_is_webkits_own_plus_safaris_tail() {
        let ua = safari_user_agent(15);
        assert!(
            ua.starts_with("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15")
        );
        assert!(ua.ends_with("Version/18.0 Safari/605.1.15"), "{ua}");
    }

    #[test]
    fn the_page_zoom_stays_inside_what_webkit_takes() {
        assert_eq!(page_zoom(None, 0.01), PAGE_ZOOM_MIN);
        assert_eq!(page_zoom(Some(3.), 4.), PAGE_ZOOM_MAX);
    }

    fn state(zoom: f64, fully_covered: bool) -> PageState {
        PageState {
            zoom,
            fully_covered,
        }
    }

    #[test]
    fn a_page_is_live_at_and_above_half_zoom_with_nothing_hiding_it() {
        assert_eq!(page_view(&state(1., false)), PageView::Live);
        assert_eq!(page_view(&state(FAR_ZOOM, false)), PageView::Live);
    }

    #[test]
    fn the_keys_leave_a_page_that_turns_into_its_picture_and_come_back_when_locked() {
        use PageView::{Live, Snapshot};
        assert_eq!(
            keys_on_view_change(Some(Live), Snapshot, true),
            Some(KeysTo::App)
        );
        assert_eq!(
            keys_on_view_change(Some(Live), Snapshot, false),
            Some(KeysTo::App)
        );
        assert_eq!(
            keys_on_view_change(Some(Snapshot), Live, true),
            Some(KeysTo::Page)
        );
        // Unlocked, the app keeps the keys when the page comes back.
        assert_eq!(keys_on_view_change(Some(Snapshot), Live, false), None);
        // No change, or the first placement: nothing moves.
        assert_eq!(keys_on_view_change(Some(Live), Live, true), None);
        assert_eq!(keys_on_view_change(Some(Snapshot), Snapshot, true), None);
        assert_eq!(keys_on_view_change(None, Live, true), None);
        assert_eq!(keys_on_view_change(None, Snapshot, true), None);
    }

    #[test]
    fn far_zoom_and_a_total_cover_give_the_snapshot() {
        assert_eq!(page_view(&state(0.49, false)), PageView::Snapshot);
        assert_eq!(page_view(&state(1., true)), PageView::Snapshot);
    }

    #[test]
    fn one_cover_that_contains_the_page_hides_it_and_a_partial_one_does_not() {
        let page: Rect = [10., 10., 100., 100.];
        assert!(fully_covered(&page, &[[0., 0., 500., 500.]]));
        assert!(fully_covered(&page, &[[10., 10., 100., 100.]]));
        assert!(!fully_covered(&page, &[[50., 50., 100., 100.]]));
        // Two covers that together hide it are holes, not a total cover.
        assert!(!fully_covered(
            &page,
            &[[0., 0., 60., 200.], [60., 0., 60., 200.]]
        ));
    }

    #[test]
    fn holes_are_the_covers_cut_to_the_page_and_never_overlap() {
        let page: Rect = [0., 0., 100., 100.];
        // A card and the drag ghost that overlap each other.
        let holes = holes_of(&page, &[[50., 50., 100., 100.], [70., 70., 100., 100.]]);
        let area: f64 = holes.iter().map(|h| h[2] * h[3]).sum();
        // 50x50 plus 30x30 minus their 30x30 overlap inside the page: 2500.
        assert_eq!(area, 2500.);
        for (i, a) in holes.iter().enumerate() {
            for b in &holes[i + 1..] {
                assert!(!overlap(a, b), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn a_cover_off_the_page_makes_no_hole() {
        let page: Rect = [0., 0., 100., 100.];
        assert!(holes_of(&page, &[[200., 200., 10., 10.], [100., 0., 5., 5.]]).is_empty());
        assert!(holes_of(&page, &[]).is_empty());
    }

    #[test]
    fn a_focused_page_has_no_wash_and_an_unfocused_one_the_setting() {
        assert_eq!(dim_alpha(true, 0.45), 0.);
        assert_eq!(dim_alpha(false, 0.45), 0.45);
        assert_eq!(dim_alpha(false, 3.), 1.);
        assert_eq!(dim_alpha(false, -1.), 0.);
    }

    #[test]
    fn the_first_snapshot_is_taken_at_once_and_then_by_age() {
        assert!(snapshot_due(true, false, None, None, 0.));
        assert!(!snapshot_due(true, false, Some(0.), None, 1999.));
        assert!(snapshot_due(
            true,
            false,
            Some(0.),
            None,
            SNAPSHOT_MAX_AGE_MS
        ));
    }

    #[test]
    fn no_snapshot_of_a_hidden_view_or_while_one_is_coming() {
        assert!(!snapshot_due(false, false, None, None, 0.));
        assert!(!snapshot_due(true, true, None, None, 0.));
    }

    #[test]
    fn a_page_that_finished_loading_after_the_last_snapshot_gets_one_soon() {
        assert!(!snapshot_due(true, false, Some(100.), Some(150.), 300.));
        assert!(snapshot_due(
            true,
            false,
            Some(100.),
            Some(150.),
            150. + SNAPSHOT_AFTER_LOAD_MS
        ));
        // A load that came BEFORE the last snapshot is not news.
        assert!(!snapshot_due(true, false, Some(500.), Some(150.), 1000.));
    }

    #[test]
    fn covers_are_the_later_cards_that_meet_the_page() {
        let order: [Rect; 4] = [
            [0., 0., 100., 100.],
            [50., 50., 100., 100.],
            [500., 500., 10., 10.],
            [99., 99., 5., 5.],
        ];
        assert_eq!(covers_of(&order, 0), vec![order[1], order[3]]);
        // Cards before it are under it, not over it.
        assert_eq!(covers_of(&order, 1), vec![order[3]]);
        assert!(covers_of(&order, 3).is_empty());
        assert!(covers_of(&order, 9).is_empty());
    }

    #[test]
    fn touching_edges_do_not_overlap() {
        assert!(!overlap(&[0., 0., 10., 10.], &[10., 0., 5., 5.]));
        assert!(overlap(&[0., 0., 10., 10.], &[9., 9., 5., 5.]));
    }

    #[test]
    fn a_data_dirs_store_id_is_stable_and_its_own() {
        assert_eq!(store_identifier("/a"), store_identifier("/a"));
        assert_ne!(store_identifier("/a"), store_identifier("/b"));
        assert_ne!(
            store_identifier("/tmp/x"),
            store_identifier("/Users/someone/Library/Application Support/dev.ekinertac.infiniterm")
        );
    }

    #[test]
    fn a_store_id_is_a_valid_uuid_and_does_not_drift() {
        let id =
            store_identifier("/Users/someone/Library/Application Support/dev.ekinertac.infiniterm");
        assert_eq!(id[6] >> 4, 4, "version 4");
        assert_eq!(id[8] >> 6, 0b10, "variant 1");
        // Pinned: a change here moves every person's sign-ins to a new,
        // empty store. Do not update this value to make a test pass.
        let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "342ace4885d844f898f81614631f33eb");
    }
}
