//! The one status bar readout for every download in progress (#819): what it
//! says, as a pure function with tests.
//!
//! Where it fits: `infiniterm-ui/src/webview_download.rs` reads each
//! download's received and expected bytes from WebKit and hands the list to
//! `readout`; `browsers.rs` puts the text into `AppView::download_readout`
//! and `overlays.rs` draws it with the status bar's other transient items. A
//! download that finishes, fails or is cancelled leaves the list, so the
//! readout is gone when the last one is.
//!
//! One readout for all of them, not one per file: the count, and the total
//! received over the total expected across the downloads whose size is
//! known. A download with no known size counts in the number and adds
//! nothing to the percent.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DownloadProgress {
    pub received: u64,
    /// The size the server announced; `None` when it did not.
    pub total: Option<u64>,
}

/// "1 download", "2 downloads, 41%"; `None` when nothing is downloading.
/// The percent is the sum received over the sum expected of the downloads
/// with a known size (a received count past its total is capped at it),
/// rounded down so 100% is only said when it is done; it is left out when no
/// download has a known size or the known sizes are all zero.
pub fn readout(downloads: &[DownloadProgress]) -> Option<String> {
    if downloads.is_empty() {
        return None;
    }
    let n = downloads.len();
    let noun = if n == 1 { "download" } else { "downloads" };
    let (mut got, mut all) = (0u128, 0u128);
    for d in downloads {
        if let Some(t) = d.total.filter(|t| *t > 0) {
            got += d.received.min(t) as u128;
            all += t as u128;
        }
    }
    // No division by zero: with no known size there is no percent.
    Some(match (got * 100).checked_div(all) {
        Some(percent) => format!("{n} {noun}, {percent}%"),
        None => format!("{n} {noun}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(received: u64, total: Option<u64>) -> DownloadProgress {
        DownloadProgress { received, total }
    }

    #[test]
    fn nothing_downloading_says_nothing() {
        assert_eq!(readout(&[]), None);
    }

    #[test]
    fn one_download_with_a_known_size_says_its_percent() {
        assert_eq!(
            readout(&[d(41, Some(100))]).as_deref(),
            Some("1 download, 41%")
        );
    }

    #[test]
    fn several_downloads_are_one_readout_over_the_total_bytes() {
        // 100 of 200 and 300 of 800: 400 of 1000.
        assert_eq!(
            readout(&[d(100, Some(200)), d(300, Some(800))]).as_deref(),
            Some("2 downloads, 40%")
        );
    }

    #[test]
    fn a_download_of_unknown_size_counts_but_adds_no_percent() {
        assert_eq!(readout(&[d(5000, None)]).as_deref(), Some("1 download"));
        // Beside a known one it adds to the count only.
        assert_eq!(
            readout(&[d(50, Some(100)), d(9999, None)]).as_deref(),
            Some("2 downloads, 50%")
        );
    }

    #[test]
    fn a_zero_size_is_unknown() {
        assert_eq!(readout(&[d(0, Some(0))]).as_deref(), Some("1 download"));
    }

    #[test]
    fn the_percent_rounds_down_and_never_passes_a_hundred() {
        assert_eq!(
            readout(&[d(999, Some(1000))]).as_deref(),
            Some("1 download, 99%")
        );
        // More received than announced: capped at its total.
        assert_eq!(
            readout(&[d(5000, Some(1000))]).as_deref(),
            Some("1 download, 100%")
        );
    }

    #[test]
    fn huge_sizes_do_not_overflow() {
        let big = u64::MAX / 2;
        assert_eq!(
            readout(&[d(big, Some(big)), d(0, Some(big))]).as_deref(),
            Some("2 downloads, 50%")
        );
    }
}
