//! Who a remote instance is (#118): the name in its window, the colour that
//! tells it from the local instance at a glance, and the host it runs on.
//!
//! `ift connect` puts these in the environment (`INFINITERM_REMOTE`,
//! `INFINITERM_REMOTE_NAME`, `INFINITERM_REMOTE_COLOR`); the app reads them
//! once at startup. The colour is chosen, not random: the host name is hashed
//! to a hue, so a host looks the same every time and two hosts look different,
//! and `--color` overrides it.
//!
//! Called by `infiniterm-ui/src/runtime.rs` (startup), `overlays.rs` (the title
//! bar), `instance_icon.rs` (the Dock icon) and the model, which refuses
//! browser cards while one is set. Related: `infiniterm-cli/src/connect.rs`.

/// An sRGB colour, one byte a channel.
pub type Rgb = (u8, u8, u8);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteIdentity {
    /// The ssh target, `user@host`.
    pub target: String,
    /// What the window and the Dock call this instance.
    pub name: String,
    pub color: Rgb,
}

/// Saturation and lightness of a hashed colour: strong enough to read on a
/// dark title bar and as a Dock badge, the same for every host so only the
/// hue tells hosts apart.
const SATURATION: f64 = 0.62;
const LIGHTNESS: f64 = 0.55;

impl RemoteIdentity {
    pub fn from_env() -> Option<RemoteIdentity> {
        let var = |k: &str| std::env::var(k).ok();
        RemoteIdentity::from_vars(
            var("INFINITERM_REMOTE").as_deref(),
            var("INFINITERM_REMOTE_NAME").as_deref(),
            var("INFINITERM_REMOTE_COLOR").as_deref(),
        )
    }

    /// `from_env` over explicit values. No target is no remote; no name is the
    /// target; no (or a bad) colour is the hashed one.
    pub fn from_vars(
        target: Option<&str>,
        name: Option<&str>,
        color: Option<&str>,
    ) -> Option<RemoteIdentity> {
        let target = target.map(str::trim).filter(|t| !t.is_empty())?;
        Some(RemoteIdentity {
            target: target.to_string(),
            name: name
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .unwrap_or(target)
                .to_string(),
            color: color
                .and_then(parse_hex)
                .unwrap_or_else(|| color_for(target)),
        })
    }
}

/// `3b82f6` or `#3B82F6`.
pub fn parse_hex(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// A colour for a host name: FNV-1a of the name picks the hue.
pub fn color_for(host: &str) -> Rgb {
    let mut h: u32 = 0x811c9dc5;
    for b in host.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    hsl_to_rgb((h % 360) as f64, SATURATION, LIGHTNESS)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Rgb {
    let c = (1. - (2. * l - 1.).abs()) * s;
    let x = c * (1. - ((h / 60.) % 2. - 1.).abs());
    let m = l - c / 2.;
    let (r, g, b) = match (h / 60.) as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let byte = |v: f64| ((v + m) * 255.).round().clamp(0., 255.) as u8;
    (byte(r), byte(g), byte(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hex_colour_is_six_digits_with_or_without_a_hash() {
        assert_eq!(parse_hex("3b82f6"), Some((0x3b, 0x82, 0xf6)));
        assert_eq!(parse_hex("#FF8800"), Some((255, 136, 0)));
        assert_eq!(parse_hex("fff"), None);
        assert_eq!(parse_hex("gggggg"), None);
        assert_eq!(parse_hex(""), None);
    }

    #[test]
    fn a_host_has_the_same_colour_every_time_and_hosts_differ() {
        assert_eq!(color_for("root@100.1.1.1"), color_for("root@100.1.1.1"));
        let hosts = ["root@100.1.1.1", "mini", "ops@box", "build", "air", "gpu"];
        let colours: Vec<Rgb> = hosts.iter().map(|h| color_for(h)).collect();
        for (i, a) in colours.iter().enumerate() {
            for b in &colours[i + 1..] {
                assert_ne!(a, b, "two of the sample hosts share a colour");
            }
        }
    }

    #[test]
    fn a_hashed_colour_is_never_too_dark_or_too_pale_to_see() {
        for h in ["a", "b", "mini", "root@100.1.1.1", "x.example.com", "z"] {
            let (r, g, b) = color_for(h);
            let lightest = r.max(g).max(b);
            let darkest = r.min(g).min(b);
            assert!(lightest >= 150, "{h}: too dark {r},{g},{b}");
            assert!(darkest <= 160, "{h}: washed out {r},{g},{b}");
        }
    }

    #[test]
    fn the_identity_takes_the_name_the_colour_or_their_defaults() {
        assert_eq!(RemoteIdentity::from_vars(None, None, None), None);
        assert_eq!(RemoteIdentity::from_vars(Some(" "), None, None), None);
        let id = RemoteIdentity::from_vars(Some("ops@box"), None, None).unwrap();
        assert_eq!(id.name, "ops@box");
        assert_eq!(id.color, color_for("ops@box"));
        let id =
            RemoteIdentity::from_vars(Some("ops@box"), Some("build box"), Some("ff8800")).unwrap();
        assert_eq!((id.name.as_str(), id.color), ("build box", (255, 136, 0)));
        let id = RemoteIdentity::from_vars(Some("ops@box"), Some(""), Some("nope")).unwrap();
        assert_eq!(
            (id.name.as_str(), id.color),
            ("ops@box", color_for("ops@box"))
        );
    }
}
