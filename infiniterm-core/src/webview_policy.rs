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
    /// Something drawn by gpui is over part of the page: a card in front, an
    /// open overlay.
    pub covered: bool,
    /// A card is being moved (the ghost is on screen).
    pub dragging: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageView {
    /// The live WebKit view is shown.
    Live,
    /// The view is hidden and a picture of the page is drawn by gpui.
    Snapshot,
}

/// A native view floats above everything gpui draws, so whenever gpui must
/// draw over the page (a cover, a drag) or the page cannot be shown at this
/// zoom, a picture stands in for it.
pub fn page_view(s: &PageState) -> PageView {
    if s.zoom < FAR_ZOOM || s.covered || s.dragging {
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
    fn the_page_zoom_stays_inside_what_webkit_takes() {
        assert_eq!(page_zoom(None, 0.01), PAGE_ZOOM_MIN);
        assert_eq!(page_zoom(Some(3.), 4.), PAGE_ZOOM_MAX);
    }

    fn state(zoom: f64, covered: bool, dragging: bool) -> PageState {
        PageState {
            zoom,
            covered,
            dragging,
        }
    }

    #[test]
    fn a_page_is_live_at_and_above_half_zoom_with_nothing_over_it() {
        assert_eq!(page_view(&state(1., false, false)), PageView::Live);
        assert_eq!(page_view(&state(FAR_ZOOM, false, false)), PageView::Live);
    }

    #[test]
    fn far_zoom_a_cover_and_a_drag_each_give_the_snapshot() {
        assert_eq!(page_view(&state(0.49, false, false)), PageView::Snapshot);
        assert_eq!(page_view(&state(1., true, false)), PageView::Snapshot);
        assert_eq!(page_view(&state(1., false, true)), PageView::Snapshot);
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
