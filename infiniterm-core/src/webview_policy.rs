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
/// world width divided by its own zoom, as a CEF page does.
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

#[cfg(test)]
mod tests {
    use super::*;

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
