//! Parses an iTerm2 `.itermcolors` file into a named colour table. Port of
//! itermcolors.ts and its tests.
//!
//! That format has the ecosystem (thousands of community schemes, 521 of
//! them bundled with the app), so importing it beats inventing one. It is an
//! XML plist of `<key>NAME</key><dict>` entries whose components are floats
//! from 0 to 1. Scanned by hand rather than with a plist parser so a
//! malformed file degrades to missing keys instead of an error. Colour Space
//! is ignored: schemes mix sRGB and Calibrated, and converting shifts colours
//! more than leaving it be.
//!
//! Names are xterm's (`red`, `brightBlack`, `foreground`, `cursorAccent`,
//! `selectionBackground`) because the theme layer keys off them; the
//! terminal palette in `infiniterm-term` maps them to ANSI slots.
use std::collections::BTreeMap;

pub type Theme = BTreeMap<String, String>;

pub const ANSI: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "brightBlack",
    "brightRed",
    "brightGreen",
    "brightYellow",
    "brightBlue",
    "brightMagenta",
    "brightCyan",
    "brightWhite",
];

/// iTerm's own names for the non-ANSI slots, mapped to xterm's.
const NAMED: [(&str, &str); 6] = [
    ("Foreground Color", "foreground"),
    ("Background Color", "background"),
    ("Cursor Color", "cursor"),
    ("Cursor Text Color", "cursorAccent"),
    ("Selection Color", "selectionBackground"),
    ("Selected Text Color", "selectionForeground"),
];

/// The `<key>NAME</key><dict>…</dict>` entries of `xml`, in order.
fn entries(xml: &str) -> Vec<(&str, &str)> {
    let mut out = vec![];
    let mut rest = xml;
    while let Some(k) = rest.find("<key>") {
        let after_key = &rest[k + 5..];
        let Some(k_end) = after_key.find("</key>") else {
            break;
        };
        let key = &after_key[..k_end];
        let after = after_key[k_end + 6..].trim_start();
        // Only a key whose value is a dict is a colour; `<key>Color Space</key>
        // <string>` inside a dict is skipped by the same test.
        if let Some(body) = after.strip_prefix("<dict>") {
            if let Some(d_end) = body.find("</dict>") {
                out.push((key, &body[..d_end]));
                rest = &body[d_end + 7..];
                continue;
            }
        }
        rest = after_key;
    }
    out
}

/// `<key>X Component</key><real>n</real>` inside a dict, as a float.
/// `<integer>` is accepted too: iTerm writes a component that is exactly 0
/// or 1 that way, and the reference's `<real>`-only regex silently drops
/// the slot, which is why Catppuccin Latte and three other bundled schemes
/// load incomplete there (reported 2026-09-15).
fn component(dict: &str, channel: &str) -> Option<f64> {
    let tag = format!("<key>{channel} Component</key>");
    let after = dict[dict.find(&tag)? + tag.len()..].trim_start();
    let (open, close) = if after.starts_with("<real>") {
        ("<real>", "</real>")
    } else {
        ("<integer>", "</integer>")
    };
    let inner = after.strip_prefix(open)?;
    let end = inner.find(close)?;
    inner[..end].trim().parse().ok()
}

fn to_hex(dict: &str) -> Option<String> {
    let (r, g, b) = (
        component(dict, "Red")?,
        component(dict, "Green")?,
        component(dict, "Blue")?,
    );
    // Clamped because some published schemes carry values slightly outside 0..1.
    let byte = |n: f64| (n * 255.).round().clamp(0., 255.) as u8;
    Some(format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b)))
}

pub fn parse_iterm_colors(xml: &str) -> Theme {
    let mut theme = Theme::new();
    for (key, dict) in entries(xml) {
        let Some(hex) = to_hex(dict) else { continue };
        let key = key.trim();
        if let Some(n) = key
            .strip_prefix("Ansi ")
            .and_then(|k| k.strip_suffix(" Color"))
        {
            if let Some(name) = n.parse::<usize>().ok().and_then(|i| ANSI.get(i)) {
                theme.insert(name.to_string(), hex);
            }
            continue;
        }
        if let Some((_, name)) = NAMED.iter().find(|(iterm, _)| *iterm == key) {
            theme.insert(name.to_string(), hex);
        }
    }
    theme
}

/// A theme is usable only if it defines all sixteen ANSI slots plus fg and bg.
pub fn is_complete_theme(theme: &Theme) -> bool {
    ANSI.iter().all(|k| theme.contains_key(*k))
        && theme.contains_key("foreground")
        && theme.contains_key("background")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, r: f64, g: f64, b: f64) -> String {
        format!(
            "\n  <key>{key}</key>\n  <dict>\n    <key>Alpha Component</key><real>1</real>\n    <key>Blue Component</key><real>{b}</real>\n    <key>Color Space</key><string>sRGB</string>\n    <key>Green Component</key><real>{g}</real>\n    <key>Red Component</key><real>{r}</real>\n  </dict>"
        )
    }

    fn doc(body: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><!DOCTYPE plist><plist version="1.0"><dict>{body}</dict></plist>"#
        )
    }

    fn sixteen(f: impl Fn(usize) -> f64) -> String {
        (0..16)
            .map(|i| entry(&format!("Ansi {i} Color"), f(i), 0., 0.))
            .collect()
    }

    #[test]
    fn reads_an_ansi_slot_as_hex() {
        assert_eq!(
            parse_iterm_colors(&doc(&entry("Ansi 1 Color", 1., 0., 0.)))["red"],
            "#ff0000"
        );
    }

    #[test]
    fn maps_all_sixteen_slots_to_xterm_names() {
        let t = parse_iterm_colors(&doc(&sixteen(|i| i as f64 / 15.)));
        assert_eq!(t["black"], "#000000");
        assert!(t.contains_key("white"));
        assert!(t.contains_key("brightBlack"));
        assert_eq!(t["brightWhite"], "#ff0000");
    }

    #[test]
    fn maps_the_named_slots() {
        let body = entry("Foreground Color", 1., 1., 1.)
            + &entry("Background Color", 0., 0., 0.)
            + &entry("Cursor Color", 0.5, 0.5, 0.5)
            + &entry("Selection Color", 0., 0., 1.);
        let t = parse_iterm_colors(&doc(&body));
        assert_eq!(t["foreground"], "#ffffff");
        assert_eq!(t["background"], "#000000");
        assert_eq!(t["cursor"], "#808080");
        assert_eq!(t["selectionBackground"], "#0000ff");
    }

    // 0.8 * 255 = 204 = 0xcc
    #[test]
    fn rounds_components_the_same_way_iterm_displays_them() {
        assert_eq!(
            parse_iterm_colors(&doc(&entry("Ansi 2 Color", 0.8, 0.8, 0.8)))["green"],
            "#cccccc"
        );
    }

    // Published schemes do carry these; they must not wrap around.
    #[test]
    fn clamps_components_outside_0_to_1() {
        assert_eq!(
            parse_iterm_colors(&doc(&entry("Ansi 3 Color", 1.4, -0.2, 0.)))["yellow"],
            "#ff0000"
        );
    }

    #[test]
    fn ignores_keys_it_does_not_know() {
        let t = parse_iterm_colors(&doc(
            &(entry("Badge Color", 1., 0., 0.) + &entry("Ansi 0 Color", 0., 0., 0.))
        ));
        assert_eq!(t.keys().collect::<Vec<_>>(), ["black"]);
    }

    #[test]
    fn a_dict_missing_a_component_is_skipped_rather_than_failing() {
        let broken = "<key>Ansi 4 Color</key><dict><key>Red Component</key><real>1</real></dict>";
        assert!(parse_iterm_colors(&doc(broken)).is_empty());
    }

    #[test]
    fn garbage_input_yields_an_empty_theme() {
        assert!(parse_iterm_colors("not xml at all").is_empty());
        assert!(parse_iterm_colors("").is_empty());
    }

    // Native check: iTerm writes an exact 0 or 1 as <integer>; four bundled
    // schemes have one and the reference drops the slot.
    #[test]
    fn an_integer_component_counts() {
        let e = "<key>Ansi 12 Color</key><dict><key>Blue Component</key><integer>1</integer><key>Green Component</key><real>0.5</real><key>Red Component</key><integer>0</integer></dict>";
        assert_eq!(parse_iterm_colors(&doc(e))["brightBlue"], "#0080ff");
    }

    #[test]
    fn completeness_needs_all_sixteen_slots_plus_foreground_and_background() {
        let full = sixteen(|_| 0.);
        assert!(!is_complete_theme(&parse_iterm_colors(&doc(&full))));
        let with_fg_bg =
            full + &entry("Foreground Color", 1., 1., 1.) + &entry("Background Color", 0., 0., 0.);
        assert!(is_complete_theme(&parse_iterm_colors(&doc(&with_fg_bg))));
    }
}

#[cfg(test)]
mod bundled {
    use super::*;
    use std::path::Path;

    // Native check: every scheme the app ships must parse complete, since
    // `seed()` copies all of them and the picker lists them all. Skipped
    // when the reference checkout is not beside this repo.
    #[test]
    fn every_bundled_scheme_parses_complete() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../infiniterm/src-tauri/resources/themes");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        let mut incomplete = vec![];
        let mut count = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "itermcolors") {
                continue;
            }
            count += 1;
            let theme = parse_iterm_colors(&std::fs::read_to_string(&path).unwrap());
            if !is_complete_theme(&theme) {
                incomplete.push(path.file_name().unwrap().to_string_lossy().into_owned());
            }
        }
        assert!(count > 500, "found only {count} schemes");
        // One shipped scheme has no ANSI slots at all; the picker shows it
        // and the fallback palette fills the gaps, as in the reference.
        assert_eq!(
            incomplete,
            ["Black Metal (Dissection).itermcolors"],
            "incomplete: {incomplete:?}"
        );
    }
}
