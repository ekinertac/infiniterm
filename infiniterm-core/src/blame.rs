//! Compact blame ages, fixed gutter columns, and full tooltips.
//! Port of blame.ts and its tests; the backend supplies parsed blame records.
//! The caller supplies time. Uncommitted lines use the explicit seven-zero hash.
#[derive(Clone, Debug, PartialEq)]
pub struct BlameLine {
    pub line: usize,
    pub hash: String,
    pub author: String,
    pub time: f64,
    pub summary: String,
}
const UNCOMMITTED: &str = "0000000";
pub fn age(time_seconds: f64, now_seconds: f64) -> String {
    let s = (now_seconds - time_seconds).max(0.);
    if s < 60. {
        return format!("{}s", s.floor());
    }
    let m = s / 60.;
    if m < 60. {
        return format!("{}m", m.floor());
    }
    let h = m / 60.;
    if h < 24. {
        return format!("{}h", h.floor());
    }
    let d = h / 24.;
    if d < 7. {
        return format!("{}d", d.floor());
    }
    if d < 30. {
        return format!("{}w", (d / 7.).floor());
    }
    if d < 365. {
        return format!("{}mo", (d / 30.).floor());
    }
    format!("{}y", (d / 365.).floor())
}
pub fn blame_text(b: &BlameLine, now_seconds: f64) -> String {
    if b.hash == UNCOMMITTED {
        return format!("{} {:10} now", "·".repeat(7), "you");
    }
    let first = b
        .author
        .split(' ')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(&b.author);
    let mut units: Vec<_> = first.encode_utf16().take(10).collect();
    units.resize(10, 32);
    let who = String::from_utf16_lossy(&units);
    format!("{} {} {:>3}", b.hash, who, age(b.time, now_seconds))
}
pub fn blame_title(b: &BlameLine) -> String {
    if b.hash == UNCOMMITTED {
        "not committed yet".into()
    } else {
        format!("{} {}: {}", b.hash, b.author, b.summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn line(hash: &str, author: &str, time: f64, summary: &str) -> BlameLine {
        BlameLine {
            line: 1,
            hash: hash.into(),
            author: author.into(),
            time,
            summary: summary.into(),
        }
    }
    #[test]
    fn age_steps_units() {
        let now = 1e9;
        for (elapsed, expected) in [
            (12., "12s"),
            (5. * 60., "5m"),
            (3. * 3600., "3h"),
            (2. * 86400., "2d"),
            (20. * 86400., "2w"),
            (100. * 86400., "3mo"),
            (800. * 86400., "2y"),
            (-50., "0s"),
        ] {
            assert_eq!(age(now - elapsed, now), expected);
        }
    }
    #[test]
    fn fixed_hash_author_age_columns() {
        let now = 1700000000.;
        let t = blame_text(&line("abc1234", "Ekin Ertaç", now - 3. * 86400., "x"), now);
        assert_eq!(t, "abc1234 Ekin        3d");
        assert_eq!(t.len(), 7 + 1 + 10 + 1 + 3);
    }
    #[test]
    fn long_name_cut_to_column() {
        let now = 1700000000.;
        assert!(blame_text(&line("abc1234", "Maximilianus", now, ""), now)
            .starts_with("abc1234 Maximilian "));
    }
    #[test]
    fn uncommitted_is_yours_now() {
        assert_eq!(
            blame_text(&line("0000000", "Not Committed Yet", 0., ""), 1700000000.),
            "······· you        now"
        );
        assert_eq!(
            blame_title(&line("0000000", "", 0., "")),
            "not committed yet"
        );
    }
}
