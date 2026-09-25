//! The in-place editor: `ift <file>` run inside a terminal card opens the
//! file in an editor card laid exactly OVER that terminal, locked for
//! typing, and `ift` waits until it is closed, so the shell's prompt comes
//! back the way it does after vim (Ekin, 2026-09-26: "open a new card, write
//! `vim ~/.zshrc` and you're done", where the editor card needed a new slot,
//! a pan and a lock first). With `ift` waiting, `EDITOR=ift` works for
//! `git commit` and `crontab -e` too.
//!
//! A COVER is an ordinary editor card, so the lock, Cmd+S, the unsaved
//! guard and drafts are the editor's own; what makes it a cover is one
//! entry in `Model::covers` (cover id to the terminal it covers). That is
//! session-only on purpose: a cover lives as long as the `ift` waiting on
//! it, and a relaunch ends that connection, so a cover is not saved at all
//! (`persist.rs`) rather than coming back as a stray overlapping card.
//!
//! The pair moves as one (`sync_covers`, from `tick`): whichever of the two
//! moved since the last sync, the other follows. The covered terminal is
//! out of `here()`, so arrow navigation, tidy and fit-all see one card, and
//! the drag's overlap checks let the two share their rect (`paired`).
//!
//! Called from `ift_in.rs` (the `edit` verb) and `lifecycle.rs` (a close
//! ends the cover and answers the waiting `ift` through
//! `Effect::CliReply`).
use super::{Effect, Model, NewCard};
use crate::saved_layout::CardKind;

impl Model {
    /// Opens `path` over the terminal `base`, focused and locked. `waiter`
    /// is the `ift` request to answer when it closes.
    pub fn open_cover(
        &mut self,
        path: &str,
        line: Option<u64>,
        base: &str,
        waiter: u64,
    ) -> Option<String> {
        let base_card = self.card(base).cloned()?;
        let cwd = std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| base_card.cwd.clone());
        let id = self.add_card(
            &cwd,
            NewCard {
                kind: CardKind::Editor,
                path: Some(path.to_string()),
                line,
                rect: Some(base_card.rect),
                workspace_id: Some(base_card.workspace_id.clone()),
                ..Default::default()
            },
        );
        if let Some(c) = self.card_mut(&id) {
            // You named the file: typing goes into it at once.
            c.locked = true;
        }
        self.covers.insert(id.clone(), base.to_string());
        self.cover_at.insert(id.clone(), base_card.rect);
        self.edit_waiters.insert(id.clone(), waiter);
        self.selection.maximized = false;
        self.set_focus(Some(&id));
        Some(id)
    }

    /// The terminal a cover covers.
    pub fn covered_by(&self, base: &str) -> Option<&str> {
        self.covers
            .iter()
            .find(|(_, b)| b.as_str() == base)
            .map(|(c, _)| c.as_str())
    }

    /// `a` and `b` are a cover and its terminal: they share a rect.
    pub fn paired(&self, a: &str, b: &str) -> bool {
        self.covers.get(a).is_some_and(|x| x == b) || self.covers.get(b).is_some_and(|x| x == a)
    }

    /// The pair moves as one: the side that moved since the last sync
    /// leads. The cover is on top, so a drag, a resize or a tidy moves it;
    /// a swap or an undo may move the terminal.
    pub fn sync_covers(&mut self) {
        let pairs: Vec<(String, String)> = self
            .covers
            .iter()
            .map(|(c, b)| (c.clone(), b.clone()))
            .collect();
        for (cover, base) in pairs {
            let (Some(c), Some(b)) = (
                self.card(&cover).map(|c| c.rect),
                self.card(&base).map(|c| c.rect),
            ) else {
                continue;
            };
            let at = self.cover_at.get(&cover).copied().unwrap_or(c);
            let to = if c != at { c } else { b };
            if let Some(x) = self.card_mut(&cover) {
                x.rect = to;
            }
            if let Some(x) = self.card_mut(&base) {
                x.rect = to;
            }
            self.cover_at.insert(cover, to);
        }
    }

    /// `id` closed: if it was a cover, the waiting `ift` is answered and the
    /// terminal is uncovered; if it was a covered terminal (its shell
    /// exited), the cover becomes an ordinary editor card.
    pub(super) fn end_cover(&mut self, id: &str) {
        if self.covers.remove(id).is_some() {
            self.cover_at.remove(id);
            if let Some(waiter) = self.edit_waiters.remove(id) {
                self.effects.push(Effect::CliReply {
                    id: waiter,
                    ok: true,
                    text: String::new(),
                });
            }
        }
        let orphans: Vec<String> = self
            .covers
            .iter()
            .filter(|(_, b)| b.as_str() == id)
            .map(|(c, _)| c.clone())
            .collect();
        for c in orphans {
            self.covers.remove(&c);
            self.cover_at.remove(&c);
            if let Some(waiter) = self.edit_waiters.remove(&c) {
                self.effects.push(Effect::CliReply {
                    id: waiter,
                    ok: true,
                    text: String::new(),
                });
            }
        }
    }
}
