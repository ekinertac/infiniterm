//! Tab commands for a browser card: new, close, next/prev, jump to N,
//! reopen the last closed one. Plain `Card` field mutation, no `Effect`:
//! `browsers.rs::reconcile_browsers` already turns a changed `card.url`
//! into ui action every frame, and is extended (Task 7) to do the same
//! for `card.tabs`.
use super::{Card, Model};
use crate::commands::CommandRegistry;
use crate::saved_layout::CardKind;

/// `closed_tabs` is a ring of the last few, not a full history: the reopen
/// command only ever wants the most recent one back.
const CLOSED_TABS_CAP: usize = 10;

/// Shared by `browser_tab_close` and `browser_tab_close_at`: remembers a
/// closed tab's url for `browser.tab.reopenClosed`, capped so a session of
/// closing tabs all day does not grow the save file without bound.
fn remember_closed(card: &mut Card, url: String) {
    card.closed_tabs.push(url);
    if card.closed_tabs.len() > CLOSED_TABS_CAP {
        card.closed_tabs.remove(0);
    }
}

/// A card's tabs, normalised: `tabs` always has at least the active url in
/// it, so `browser.tab.new`/`close` need not special-case "no tabs yet"
/// (a single-tab card keeps `tabs` empty on disk, but in memory it always
/// has exactly the browser card's fields to fall back to).
fn ensure_tabs(m: &mut Model, id: &str) {
    let Some(card) = m.card_mut(id) else { return };
    if card.tabs.is_empty() {
        let url = card.url.clone().unwrap_or_else(|| "about:blank".into());
        card.tabs = vec![url];
        card.active_tab = 0;
    }
}

impl Model {
    /// Appends a new tab at `url` (`None` is `about:blank`, the same
    /// default an empty browser card opens with) and makes it active.
    pub fn browser_tab_open(&mut self, card_id: &str, url: Option<&str>) {
        ensure_tabs(self, card_id);
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        card.tabs.push(url.unwrap_or("about:blank").to_string());
        card.active_tab = card.tabs.len() - 1;
        card.url = card.tabs.last().cloned();
        self.dirty_layout = true;
    }

    /// Closes the active tab. Closes the whole card instead when it was
    /// the last tab, `Cmd+W`'s rule.
    pub fn browser_tab_close(&mut self, card_id: &str) {
        ensure_tabs(self, card_id);
        let Some(card) = self.card(card_id) else {
            return;
        };
        if card.tabs.len() <= 1 {
            let focused_is_this = self.selection.focused_id.as_deref() == Some(card_id);
            if focused_is_this {
                self.close_selected();
            }
            return;
        }
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        let closed = card.tabs.remove(card.active_tab);
        remember_closed(card, closed);
        if card.active_tab >= card.tabs.len() {
            card.active_tab = card.tabs.len() - 1;
        }
        card.url = card.tabs.get(card.active_tab).cloned();
        self.dirty_layout = true;
    }

    /// Closes the tab AT `index`, which is not necessarily the active one —
    /// the strip's `x` on a background tab must not first jump the user
    /// onto it and only then close it (real Chrome does not move your
    /// active tab when you close a different one). `index` before the
    /// active tab shifts it down by one; `index` at the active tab follows
    /// `browser_tab_close`'s own rule (the neighbour that slides into the
    /// slot becomes active, clamped if it was the last); `index` after the
    /// active tab leaves it untouched. Out of range is a no-op, matching
    /// every other index-taking tab command here.
    pub fn browser_tab_close_at(&mut self, card_id: &str, index: usize) {
        ensure_tabs(self, card_id);
        let Some(card) = self.card(card_id) else {
            return;
        };
        if index >= card.tabs.len() {
            return;
        }
        if card.tabs.len() <= 1 {
            let focused_is_this = self.selection.focused_id.as_deref() == Some(card_id);
            if focused_is_this {
                self.close_selected();
            }
            return;
        }
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        let closed = card.tabs.remove(index);
        remember_closed(card, closed);
        if index < card.active_tab {
            card.active_tab -= 1;
        } else if card.active_tab >= card.tabs.len() {
            card.active_tab = card.tabs.len() - 1;
        }
        card.url = card.tabs.get(card.active_tab).cloned();
        self.dirty_layout = true;
    }

    /// Reopens the most recently closed tab, if there is one.
    pub fn browser_tab_reopen_closed(&mut self, card_id: &str) {
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        let Some(url) = card.closed_tabs.pop() else {
            return;
        };
        self.browser_tab_open(card_id, Some(&url));
    }

    /// `delta` +1/-1, wrapping.
    pub fn browser_tab_step(&mut self, card_id: &str, delta: i32) {
        ensure_tabs(self, card_id);
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        if card.tabs.is_empty() {
            return;
        }
        let n = card.tabs.len() as i32;
        card.active_tab = ((card.active_tab as i32 + delta).rem_euclid(n)) as usize;
        card.url = card.tabs.get(card.active_tab).cloned();
        self.dirty_layout = true;
    }

    /// 0-based `index`; out of range is a no-op, matching Chrome's
    /// `Cmd+N` on a window with fewer than N tabs.
    pub fn browser_tab_jump(&mut self, card_id: &str, index: usize) {
        ensure_tabs(self, card_id);
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        if index >= card.tabs.len() {
            return;
        }
        card.active_tab = index;
        card.url = card.tabs.get(index).cloned();
        self.dirty_layout = true;
    }

    pub fn browser_tab_jump_last(&mut self, card_id: &str) {
        ensure_tabs(self, card_id);
        let Some(card) = self.card_mut(card_id) else {
            return;
        };
        if let Some(last) = card.tabs.len().checked_sub(1) {
            card.active_tab = last;
            card.url = card.tabs.get(last).cloned();
            self.dirty_layout = true;
        }
    }
}

/// Runs `f` against the focused card's id if it is a browser card, else
/// notifies, matching `browser_history`'s existing not-a-browser message.
fn with_focused_browser(m: &mut Model, f: impl FnOnce(&mut Model, String)) {
    m.with_active_card(|m, id| {
        let Some(card) = m.card(&id) else { return };
        if card.kind != CardKind::Browser {
            m.notify("not a browser card");
            return;
        }
        f(m, id);
    });
}

pub fn register(r: &mut CommandRegistry<Model>) {
    r.register("browser.tab.new", "Browser: new tab", |m| {
        with_focused_browser(m, |m, id| m.browser_tab_open(&id, None));
    });
    r.register("browser.tab.close", "Browser: close tab", |m| {
        with_focused_browser(m, |m, id| m.browser_tab_close(&id));
    });
    r.register(
        "browser.tab.reopenClosed",
        "Browser: reopen closed tab",
        |m| {
            with_focused_browser(m, |m, id| m.browser_tab_reopen_closed(&id));
        },
    );
    r.register("browser.tab.next", "Browser: next tab", |m| {
        with_focused_browser(m, |m, id| m.browser_tab_step(&id, 1));
    });
    r.register("browser.tab.prev", "Browser: previous tab", |m| {
        with_focused_browser(m, |m, id| m.browser_tab_step(&id, -1));
    });
    for n in 1..=8 {
        r.register(
            &format!("browser.tab.jump.{n}"),
            &format!("Browser: tab {n}"),
            move |m| with_focused_browser(m, |m, id| m.browser_tab_jump(&id, n - 1)),
        );
    }
    r.register("browser.tab.jump.last", "Browser: last tab", |m| {
        with_focused_browser(m, |m, id| m.browser_tab_jump_last(&id));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Model, NewCard};

    fn browser_card() -> (Model, String) {
        let mut m = Model::new();
        m.home = "/h".into();
        m.start_dir = "/h".into();
        // `close_selected` (behind `browser_tab_close`'s single-tab case)
        // walks `here()`, which is scoped to `active_workspace`; a card's
        // `workspace_id` defaults to "" (`add_card`'s `unwrap_or_default`),
        // so the active workspace has to match or the card is invisible to
        // selection, same as a real session where `load_layout` sets both
        // before any card exists.
        m.active_workspace = Some(String::new());
        let id = m.add_card(
            "/h",
            NewCard {
                kind: CardKind::Browser,
                url: Some("https://a.example".into()),
                ..Default::default()
            },
        );
        m.set_focus(Some(&id));
        (m, id)
    }

    #[test]
    fn opening_a_tab_appends_and_activates_it() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        let card = m.card(&id).unwrap();
        assert_eq!(card.tabs, vec!["https://a.example", "https://b.example"]);
        assert_eq!(card.active_tab, 1);
        assert_eq!(card.url.as_deref(), Some("https://b.example"));
    }

    #[test]
    fn closing_the_active_tab_removes_it_and_activates_a_neighbour() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.browser_tab_open(&id, Some("https://c.example"));
        m.browser_tab_jump(&id, 1); // b.example active
        m.browser_tab_close(&id);
        let card = m.card(&id).unwrap();
        assert_eq!(card.tabs, vec!["https://a.example", "https://c.example"]);
        assert_eq!(card.active_tab, 1, "the tab that slid into the closed slot");
        assert_eq!(card.closed_tabs, vec!["https://b.example"]);
    }

    #[test]
    fn closing_the_last_tab_closes_the_card_instead() {
        let (mut m, id) = browser_card();
        m.browser_tab_close(&id);
        assert!(m.card(&id).is_none());
    }

    // Closing a BACKGROUND tab by index must not move you off the tab you
    // are looking at: [a, b, c] with c active, closing index 0 (a) leaves
    // c active still, not "whatever is now at the old active index" (which
    // a naive jump-then-close-active would give: index 2 after removal is
    // c anyway here, so the real regression case is caught by the next
    // test, where the shift would land on the wrong tab).
    #[test]
    fn closing_a_non_active_tab_by_index_leaves_the_active_tab_the_same() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.browser_tab_open(&id, Some("https://c.example"));
        m.browser_tab_jump(&id, 2); // c.example active
        m.browser_tab_close_at(&id, 0); // close a.example, not the active one
        let card = m.card(&id).unwrap();
        assert_eq!(card.tabs, vec!["https://b.example", "https://c.example"]);
        assert_eq!(
            card.active_tab, 1,
            "still on c.example, shifted down by the removed tab before it"
        );
        assert_eq!(card.url.as_deref(), Some("https://c.example"));
        assert_eq!(card.closed_tabs, vec!["https://a.example"]);
    }

    // The mirror case: closing a tab AFTER the active one must not move
    // the active index at all.
    #[test]
    fn closing_a_later_tab_by_index_does_not_move_the_active_index() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.browser_tab_open(&id, Some("https://c.example"));
        m.browser_tab_jump(&id, 0); // a.example active
        m.browser_tab_close_at(&id, 2); // close c.example, after the active one
        let card = m.card(&id).unwrap();
        assert_eq!(card.tabs, vec!["https://a.example", "https://b.example"]);
        assert_eq!(card.active_tab, 0, "unaffected: a.example is still active");
        assert_eq!(card.url.as_deref(), Some("https://a.example"));
    }

    // Closing the ACTIVE tab by its own index is the same rule
    // `browser_tab_close` already gives it: the neighbour that slides into
    // the slot becomes active.
    #[test]
    fn closing_the_active_tab_by_index_activates_the_neighbour_that_slides_in() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.browser_tab_open(&id, Some("https://c.example"));
        m.browser_tab_jump(&id, 1); // b.example active
        m.browser_tab_close_at(&id, 1);
        let card = m.card(&id).unwrap();
        assert_eq!(card.tabs, vec!["https://a.example", "https://c.example"]);
        assert_eq!(card.active_tab, 1, "the tab that slid into the closed slot");
    }

    // Closing the active tab by index when it was the LAST tab still closes
    // the whole card, matching `browser_tab_close`'s existing behavior.
    #[test]
    fn closing_the_active_tab_by_index_closes_the_card_when_it_was_the_last_tab() {
        let (mut m, id) = browser_card();
        m.browser_tab_close_at(&id, 0);
        assert!(m.card(&id).is_none());
    }

    // An out-of-range index is a no-op: the click that produced it came
    // from a strip diffed against `card.tabs`, and a stale click must not
    // touch a tab it no longer names.
    #[test]
    fn closing_by_an_out_of_range_index_is_a_no_op() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.dirty_layout = false;
        m.browser_tab_close_at(&id, 5);
        assert_eq!(m.card(&id).unwrap().tabs.len(), 2);
        assert!(!m.dirty_layout);
    }

    #[test]
    fn reopen_closed_puts_the_last_closed_tab_back_and_activates_it() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.browser_tab_close(&id); // closes b.example (was active)
        m.browser_tab_reopen_closed(&id);
        let card = m.card(&id).unwrap();
        assert_eq!(card.tabs, vec!["https://a.example", "https://b.example"]);
        assert_eq!(card.active_tab, 1);
        assert!(card.closed_tabs.is_empty());
    }

    #[test]
    fn next_and_prev_wrap() {
        let (mut m, id) = browser_card();
        m.browser_tab_open(&id, Some("https://b.example"));
        m.dirty_layout = false; // opening already dirtied it; isolate the step
        m.browser_tab_step(&id, 1);
        assert_eq!(m.card(&id).unwrap().active_tab, 0, "wrapped past the end");
        assert!(
            m.dirty_layout,
            "a tab switch must reach the save file or a restart loses it"
        );
        m.browser_tab_step(&id, -1);
        assert_eq!(m.card(&id).unwrap().active_tab, 1, "wrapped past the start");
    }

    #[test]
    fn jump_out_of_range_is_a_no_op() {
        let (mut m, id) = browser_card();
        m.dirty_layout = false; // set_focus in the fixture already dirtied it
        m.browser_tab_jump(&id, 5);
        assert_eq!(m.card(&id).unwrap().active_tab, 0);
        assert!(!m.dirty_layout, "a no-op jump changed nothing to save");
        m.browser_tab_jump(&id, 0);
        assert!(m.dirty_layout, "a real jump does");
    }

    #[test]
    fn a_non_browser_card_notifies_and_changes_nothing() {
        let mut m = Model::new();
        m.home = "/h".into();
        m.start_dir = "/h".into();
        m.active_workspace = Some(String::new());
        let id = m.add_card("/h", NewCard::default());
        m.set_focus(Some(&id));
        m.browser_tab_open(&id, None); // direct call still works, no notice
        with_focused_browser(&mut m, |_, _| panic!("must not run: not a browser card"));
        assert_eq!(m.notice.as_deref(), Some("not a browser card"));
    }
}
