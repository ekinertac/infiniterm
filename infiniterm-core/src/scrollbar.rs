//! Where a scrollbar's thumb goes: a pure function of how much there is, how
//! much shows and where the view is, so the maths is tested and any body that
//! scrolls (a Page card now, an editor or a terminal later) paints the same
//! thumb. The ui crate only draws the rectangle this returns.
//!
//! Called by `infiniterm-ui/src/page_body.rs`. Units are the caller's: lines
//! for `total`, `visible` and `offset`, screen pixels for `track` and
//! `min_len`, and the answer is in the track's pixels.

/// `(start, len)` of the thumb inside a track `track` long, or `None` when
/// everything fits (a scrollbar on a view that does not scroll says nothing
/// and invites a click that does nothing). The thumb is as long as the visible
/// share of the whole, never shorter than `min_len` (a 3000-line page would
/// otherwise draw a sliver nobody can see) and never longer than the track.
/// `offset` is the first visible line; past the end it counts as the end.
pub fn thumb(
    total: usize,
    visible: usize,
    offset: usize,
    track: f64,
    min_len: f64,
) -> Option<(f64, f64)> {
    if visible == 0 || total <= visible || track <= 0. {
        return None;
    }
    let len = (visible as f64 / total as f64 * track)
        .max(min_len)
        .min(track);
    let max_offset = (total - visible) as f64;
    let at = offset.min(total - visible) as f64 / max_offset;
    Some((at * (track - len), len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_to_scroll_gives_no_thumb() {
        assert_eq!(thumb(10, 10, 0, 100., 8.), None);
        assert_eq!(thumb(5, 10, 0, 100., 8.), None);
        assert_eq!(thumb(0, 0, 0, 100., 8.), None);
        assert_eq!(thumb(50, 0, 0, 100., 8.), None);
        assert_eq!(thumb(50, 10, 0, 0., 8.), None);
    }

    #[test]
    fn the_thumb_is_the_visible_share_of_the_track() {
        assert_eq!(thumb(100, 25, 0, 200., 8.), Some((0., 50.)));
    }

    #[test]
    fn it_sits_at_the_top_at_the_bottom_and_between() {
        let (start, len) = thumb(100, 25, 0, 200., 8.).unwrap();
        assert_eq!(start, 0.);
        let (start, len_end) = thumb(100, 25, 75, 200., 8.).unwrap();
        assert_eq!((start + len_end, len_end), (200., len));
        // half way through the scrollable lines, half way along the free track
        let (start, len) = thumb(100, 25, 37, 200., 8.).unwrap();
        assert!((start - 0.4933 * (200. - len)).abs() < 0.5, "{start}");
    }

    #[test]
    fn a_very_long_page_still_gets_a_thumb_you_can_see() {
        let (start, len) = thumb(10_000, 30, 9_970, 300., 24.).unwrap();
        assert_eq!(len, 24.);
        assert_eq!(start + len, 300.);
    }

    #[test]
    fn an_offset_past_the_end_counts_as_the_end() {
        assert_eq!(thumb(100, 25, 500, 200., 8.), thumb(100, 25, 75, 200., 8.));
    }

    #[test]
    fn a_min_len_longer_than_the_track_fills_it() {
        assert_eq!(thumb(100, 10, 30, 20., 50.), Some((0., 20.)));
    }
}
