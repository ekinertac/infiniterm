//! Stable identity colors and readable foreground colors for card/group labels.
//! Port of labelColors.ts and its tests. Theme loading supplies a sparse palette.
//! Hash UTF-16 ids with wrapping FNV-1a; keep the key order stable and red excluded.
use std::collections::BTreeMap;
pub type Palette = BTreeMap<String, String>;
pub const LABEL_KEYS: [&str; 10] = [
    "blue",
    "green",
    "magenta",
    "cyan",
    "yellow",
    "brightBlue",
    "brightGreen",
    "brightMagenta",
    "brightCyan",
    "brightYellow",
];
pub fn hash_index(id: &str, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let mut h = 0x811c9dc5_u32;
    for c in id.encode_utf16() {
        h ^= c as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h as usize % count
}
pub fn label_color<'a>(id: &str, theme: Option<&'a Palette>) -> Option<&'a str> {
    let theme = theme?;
    let available: Vec<_> = LABEL_KEYS
        .iter()
        .filter_map(|k| theme.get(*k))
        .filter(|c| !c.is_empty())
        .collect();
    available
        .get(hash_index(id, available.len()))
        .map(|s| s.as_str())
}
pub fn luminance(hex: &str) -> f64 {
    let stripped = hex.replacen('#', "", 1);
    let h = stripped.trim();
    let full = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        h.into()
    };
    if full.len() != 6 || !full.bytes().all(|c| c.is_ascii_hexdigit()) {
        return 0.;
    }
    let channel = |i: usize| {
        let c =
            u8::from_str_radix(&full[i * 2..i * 2 + 2], 16).expect("validated hex") as f64 / 255.;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(0) + 0.7152 * channel(1) + 0.0722 * channel(2)
}
pub const LABEL_FG_DARK: &str = "#12151a";
pub const LABEL_FG_LIGHT: &str = "#f2f6fb";
pub fn readable_on(hex: &str) -> &'static str {
    if luminance(hex) > 0.4 {
        LABEL_FG_DARK
    } else {
        LABEL_FG_LIGHT
    }
}

/// Least contrast ratio (WCAG's, 1 to 21) a theme's `Cursor Text Color` must
/// have against its cursor to be trusted: many themes leave the two equal or
/// near, which hid the letter under a block cursor (#343).
const MIN_CURSOR_CONTRAST: f64 = 3.;

fn contrast_ratio(a: &str, b: &str) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The colour for the letter under a block cursor: the theme's own cursor
/// text when it reads against the cursor, else black or white, whichever
/// does. `accent` is None when the cursor colour was set by the user, since
/// the theme chose its text for another cursor.
pub fn cursor_text_on(cursor: &str, accent: Option<&str>) -> String {
    match accent.filter(|a| !a.is_empty()) {
        Some(a) if contrast_ratio(cursor, a) >= MIN_CURSOR_CONTRAST => a.to_string(),
        _ => readable_on(cursor).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn theme() -> Palette {
        LABEL_KEYS
            .iter()
            .enumerate()
            .map(|(i, k)| (k.to_string(), format!("#00000{i}")))
            .collect()
    }
    #[test]
    fn stable_same_id() {
        assert_eq!(hash_index("card-a", 10), hash_index("card-a", 10));
    }
    #[test]
    fn hash_stays_in_range() {
        for id in ["a", "bb", "a-very-long-uuid-like-string", ""] {
            assert!(hash_index(id, 10) < 10);
        }
    }
    #[test]
    fn hash_spreads_across_palette() {
        let used = (0..200)
            .map(|i| hash_index(&format!("card-{i}"), 10))
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(used.len(), 10);
    }
    #[test]
    fn empty_palette_safe() {
        assert_eq!(hash_index("a", 0), 0);
    }
    #[test]
    fn color_comes_from_palette() {
        let theme = theme();
        let c = label_color("card-a", Some(&theme)).unwrap();
        assert!(theme.values().any(|v| v == c));
    }
    #[test]
    fn no_theme_no_color() {
        assert_eq!(label_color("card-a", None), None);
        assert_eq!(label_color("card-a", Some(&Palette::new())), None);
    }
    #[test]
    fn excludes_reserved_colors() {
        for key in ["red", "brightRed", "black", "white"] {
            assert!(!LABEL_KEYS.contains(&key));
        }
    }
    #[test]
    fn only_defined_colors() {
        let t = Palette::from([("green".into(), "#0a0".into())]);
        assert_eq!(label_color("card-a", Some(&t)), Some("#0a0"));
    }
    #[test]
    fn readable_foreground() {
        assert_eq!(readable_on("#ffffff"), LABEL_FG_DARK);
        assert_eq!(readable_on("#000000"), LABEL_FG_LIGHT);
        assert_eq!(readable_on("#e8e884"), LABEL_FG_DARK);
        assert_eq!(readable_on("#204a87"), LABEL_FG_LIGHT);
    }
    #[test]
    fn three_digit_hex() {
        assert_eq!(readable_on("#fff"), LABEL_FG_DARK);
        assert_eq!(readable_on("#000"), LABEL_FG_LIGHT);
    }
    #[test]
    fn malformed_color_safe() {
        assert_eq!(luminance("nonsense"), 0.);
        assert_eq!(readable_on("nonsense"), LABEL_FG_LIGHT);
    }

    #[test]
    fn the_letter_under_a_cursor_reads_against_it() {
        // A theme whose cursor text equals its cursor falls back to black or white.
        assert_eq!(cursor_text_on("#e0e0e0", Some("#e0e0e0")), LABEL_FG_DARK);
        assert_eq!(cursor_text_on("#202020", Some("#202020")), LABEL_FG_LIGHT);
        // A readable theme choice is kept.
        assert_eq!(cursor_text_on("#e0e0e0", Some("#101010")), "#101010");
        // No accent (a cursor colour the user set): black or white.
        assert_eq!(cursor_text_on("#ffcc00", None), LABEL_FG_DARK);
        assert_eq!(cursor_text_on("#223344", Some("")), LABEL_FG_LIGHT);
    }
}
