//! Which commands you actually run: every command the registry runs (a
//! key, the palette, `ift`) appends a line to `usage.log` beside the save
//! file, and `ift usage` reads it back against the full command list, so
//! the features never touched show up (Ekin, 2026-09-27: "what features we
//! added that I'm not using daily").
//!
//! LOCAL ONLY. Nothing reads this file but `ift usage` and nothing sends
//! it anywhere; it is a log like `agent.log`, which is why the README's "no
//! telemetry" still holds. Capped at `MAX_BYTES`, keeping the newest half
//! when it grows past.
//!
//! Written from the registry's log hook in the ui (`runtime.rs`), read in
//! `model/ift_in.rs` (`ift usage`). Commands only: a drag or a double-click
//! that is not a command does not appear.
use std::io::Write;
use std::path::Path;

/// The mouse gestures recorded beside the commands (`AppView::note_use`),
/// one line per finished gesture, not per event, so dragging a card can
/// be weighed against the arrow-key moves (`card.move.*`). With what each
/// one is, for `ift usage`.
pub const MOUSE_GESTURES: &[(&str, &str)] = &[
    ("mouse.card.drag", "Mouse: drag a card to a new place"),
    ("mouse.group.drag", "Mouse: drag a group by its tab"),
    ("mouse.card.resize", "Mouse: resize a card by its edge"),
    ("mouse.canvas.pan", "Mouse: pan the canvas by dragging"),
    ("mouse.marquee", "Mouse: drag a rectangle to select cards"),
    (
        "mouse.selection.drag",
        "Mouse: drag several selected cards together",
    ),
    (
        "mouse.card.cmdclick",
        "Mouse: Cmd+click to add or remove a card",
    ),
    (
        "mouse.canvas.zoom",
        "Mouse: zoom with Cmd+scroll or a pinch",
    ),
    (
        "mouse.fitCard.doubleclick",
        "Mouse: double-click a frame to fit the card",
    ),
    (
        "mouse.fitAll.doubleclick",
        "Mouse: double-click the canvas to fit all",
    ),
    (
        "mouse.fitAll.chord",
        "Mouse: hold left, click right to fit all",
    ),
];

/// Wheel zoom arrives as a stream of events; one line per this much quiet.
pub const ZOOM_GESTURE_GAP_MS: u64 = 2000;

/// `ift usage`: the commands and gestures used in the last `days`, most
/// first, then the ones never used, each with its label. `commands` is
/// the registry as (id, label).
pub fn report(text: &str, now_ms: u64, days: u64, commands: &[(&str, &str)]) -> String {
    let mut all: Vec<(&str, &str)> = commands.to_vec();
    all.extend(MOUSE_GESTURES.iter().copied());
    let ids: Vec<&str> = all.iter().map(|(id, _)| *id).collect();
    let since = now_ms.saturating_sub(days * 24 * 60 * 60 * 1000);
    let (used, unused) = summarize(text, since, &ids);
    let label = |id: &str| {
        all.iter()
            .find(|(i, _)| *i == id)
            .map(|(_, l)| *l)
            .unwrap_or("")
    };
    let mut out = format!("used in the last {days} days:\n");
    if used.is_empty() {
        out.push_str("  nothing yet\n");
    }
    for u in &used {
        out.push_str(&format!("  {:>5}  {}  ({})\n", u.count, u.id, label(&u.id)));
    }
    out.push_str(&format!("\nnever used in that time ({}):\n", unused.len()));
    for id in &unused {
        out.push_str(&format!("  {id}  ({})\n", label(id)));
    }
    out
}

/// A year of heavy use is well under this; past it the older half goes.
pub const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Appends `id` at `ms` (Unix milliseconds), trimming the file first when
/// it has grown past `MAX_BYTES`. Errors are the caller's to ignore: a
/// usage line is never worth a failed command.
pub fn record(path: &Path, id: &str, ms: u64) -> std::io::Result<()> {
    if std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) > MAX_BYTES {
        let text = std::fs::read_to_string(path)?;
        std::fs::write(path, keep_newest_half(&text))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{ms}\t{id}")
}

/// The newer half of the lines.
fn keep_newest_half(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = lines[lines.len() / 2..].join("\n");
    out.push('\n');
    out
}

/// One command's use since the cutoff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Used {
    pub id: String,
    pub count: usize,
}

/// What `text` says about `all` since `since_ms`: the commands used, most
/// first, and the ones never used, in registry order. Commands the log
/// names that are not in `all` any more are dropped; `dev.*` is left out
/// (development builds only).
pub fn summarize(text: &str, since_ms: u64, all: &[&str]) -> (Vec<Used>, Vec<String>) {
    let mut counts: Vec<Used> = vec![];
    for line in text.lines() {
        let Some((ms, id)) = line.split_once('\t') else {
            continue;
        };
        let Ok(ms) = ms.parse::<u64>() else { continue };
        if ms < since_ms || !all.contains(&id) || id.starts_with("dev.") {
            continue;
        }
        match counts.iter_mut().find(|u| u.id == id) {
            Some(u) => u.count += 1,
            None => counts.push(Used {
                id: id.to_string(),
                count: 1,
            }),
        }
    }
    counts.sort_by(|a, b| b.count.cmp(&a.count).then(a.id.cmp(&b.id)));
    let unused = all
        .iter()
        .filter(|id| !id.starts_with("dev.") && !counts.iter().any(|u| u.id == **id))
        .map(|id| id.to_string())
        .collect();
    (counts, unused)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_counts_what_ran_since_the_cutoff_and_lists_the_rest() {
        let text = "100\tcard.find\n200\tcard.find\n300\tcanvas.tidy\n50\tcard.new.terminal\n400\tgone.command\n500\tdev.stress.cards\nnot a line\n";
        let all = [
            "card.find",
            "canvas.tidy",
            "card.new.terminal",
            "card.mask",
            "dev.stress.cards",
        ];
        let (used, unused) = summarize(text, 100, &all);
        assert_eq!(
            used,
            vec![
                Used {
                    id: "card.find".into(),
                    count: 2
                },
                Used {
                    id: "canvas.tidy".into(),
                    count: 1
                },
            ]
        );
        assert_eq!(
            unused,
            ["card.new.terminal", "card.mask"],
            "before the cutoff counts as unused"
        );
    }

    #[test]
    fn the_report_counts_gestures_beside_commands() {
        let day = 24 * 60 * 60 * 1000;
        let text = format!(
            "{}\tmouse.card.drag\n{}\tcard.move.left\n{}\tcard.move.left\n",
            10 * day,
            10 * day,
            10 * day
        );
        let r = report(
            &text,
            11 * day,
            30,
            &[
                ("card.move.left", "Card: move left"),
                ("card.mask", "Card: mask"),
            ],
        );
        assert!(
            r.contains("      2  card.move.left  (Card: move left)"),
            "{r}"
        );
        assert!(r.contains("      1  mouse.card.drag"), "{r}");
        assert!(r.contains("  card.mask  (Card: mask)"), "{r}");
        assert!(r.contains("  mouse.canvas.pan  (Mouse: pan"), "{r}");
    }

    #[test]
    fn the_log_appends_and_keeps_the_newest_half_past_the_cap() {
        let path = std::env::temp_dir().join(format!("ift-usage-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        record(&path, "card.find", 1).unwrap();
        record(&path, "canvas.tidy", 2).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "1\tcard.find\n2\tcanvas.tidy\n"
        );
        assert_eq!(keep_newest_half("a\nb\nc\nd\n"), "c\nd\n");
        let _ = std::fs::remove_file(&path);
    }
}
