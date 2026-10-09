// The socket between the host card and the probe, one JSON object per line.
// Host -> child: what the card received (size, pointer, wheel, keys, composed
// text). Child -> host: the IOSurface to show (its global id, which the host
// looks up with IOSurfaceLookup) and a frame counter.
//
// Not stable: this is a spike. Points are in the card's own logical space,
// sizes in device pixels, so `scale` says how many device pixels make a point.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum FromHost {
    /// The card's size in device pixels and the scale that maps points to them.
    /// A zoom sends a new one; the child re-renders at that scale, so text is
    /// drawn at the zoom instead of scaled up from a smaller bitmap.
    Size { w: u32, h: u32, scale: f32 },
    /// Pointer position in points. `button` is "primary", "secondary",
    /// "middle" or absent for a plain move.
    Pointer {
        x: f32,
        y: f32,
        #[serde(default)]
        button: Option<String>,
        #[serde(default)]
        down: bool,
        #[serde(default)]
        mods: Mods,
    },
    /// Wheel or trackpad delta in points.
    Scroll {
        dx: f32,
        dy: f32,
        #[serde(default)]
        mods: Mods,
    },
    /// A physical key. `key` is an egui key name ("ArrowLeft", "Enter", "A").
    Key {
        key: String,
        down: bool,
        #[serde(default)]
        mods: Mods,
    },
    /// Text the host's ime.rs path produced (typed characters, a dead key's
    /// composed letter, the emoji panel). Never sent together with the key
    /// that produced it.
    Text { s: String },
}

#[derive(Debug, Default, Deserialize, Clone, Copy)]
pub struct Mods {
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub cmd: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ToHost {
    /// A new surface exists. `id` is IOSurfaceGetID; `bpr` its bytes per row.
    Surface { id: u32, w: u32, h: u32, bpr: u32 },
    /// The surface with this id holds frame `seq`.
    Frame { id: u32, seq: u64 },
}

pub fn encode(msg: &ToHost) -> String {
    // A line per message; serialising these plain structs cannot fail.
    let mut s = serde_json::to_string(msg).expect("ToHost serialises");
    s.push('\n');
    s
}
