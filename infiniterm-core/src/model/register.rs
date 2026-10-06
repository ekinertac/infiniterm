//! Every command, registered in the order the palette and the shortcuts
//! panel list them: roughly how the features read. Port of the register
//! calls at the top of `App.svelte`.
use super::Model;
use crate::commands::CommandRegistry;

pub fn register_commands(r: &mut CommandRegistry<Model>) {
    super::cards_cmd::register(r);
    super::focus_cmd::register(r);
    super::canvas_cmd::register(r);
    super::workspaces_cmd::register(r);
    super::color_cmd::register(r);
    super::groups_cmd::register(r);
    super::dev_cmd::register(r);
    super::tabs_cmd::register(r);
}

/// What the command trace says about each command: the card it acted on.
pub fn describe_context(m: &Model) -> String {
    match m.focused() {
        None => "focus=none".to_string(),
        Some(c) => {
            let r = c.rect;
            format!(
                "focus={} at {},{} {}x{} soft={}",
                &c.id[..8.min(c.id.len())],
                (r.x / 25.).round(),
                (r.y / 25.).round(),
                (r.w / 25.).round(),
                (r.h / 25.).round(),
                c.soft_group_id
                    .as_deref()
                    .map_or("-".to_string(), |s| s[..8.min(s.len())].to_string())
            )
        }
    }
}

/// A chord, resolved the way `App.svelte`'s key handler resolves it: inside
/// an editor the chords it keeps are its own; inside a browser the zoom
/// chords are the page's; a LOCKED browser card widens that to real
/// Chrome tab shortcuts; then the keymap. True when a command ran.
pub fn handle_chord(m: &mut Model, r: &CommandRegistry<Model>, chord: &str) -> bool {
    let Some(id) = resolve_chord(m, chord) else {
        return false;
    };
    run_with_effects(m, r, &id);
    true
}

/// The command a chord runs on the focused card right now, or `None` when
/// the chord is the body's. Separate from `handle_chord` so the keycast can
/// label a chord with the command it ACTUALLY ran: on a locked card the
/// keymap says "Card: new terminal" for Cmd+T while this says the tab.
pub fn resolve_chord(m: &Model, chord: &str) -> Option<String> {
    let card = m.focused();
    let kind = card.map(|c| c.kind);
    // An editor keeps its find, comment, undo and select-to-boundary
    // chords only once you are IN it (locked). Merely focused, with
    // Cmd+Alt+Arrow, it is a card like any other and the canvas has its
    // keys: Cmd+/ on an arrowed-to editor commented the highlighted line
    // instead of opening the shortcuts panel.
    if kind == Some(crate::saved_layout::CardKind::Editor)
        && card.is_some_and(|c| c.locked)
        && crate::editor_keys::editor_keeps(chord)
    {
        return None;
    }
    let locked = card.is_some_and(|c| c.locked)
        && matches!(
            kind,
            Some(crate::saved_layout::CardKind::Browser | crate::saved_layout::CardKind::Editor)
        );
    // A locked card claims everything except Ctrl+1..9 and the two
    // app-carve-out chords below: real Chrome shortcuts shadow this app's
    // own bindings on the same keys, the whole point of locking. Workspace
    // switching is not a Chrome shortcut, so it alone keeps working. An
    // editor card locks the same way with its own tab commands behind the
    // same chords (`editor_keys::lock_override`); what neither table
    // knows falls through to the body, where the editor's own keys live.
    if locked && !is_workspace_switch(chord) && !is_locked_carveout(chord, kind) {
        let id = if kind == Some(crate::saved_layout::CardKind::Editor) {
            crate::editor_keys::lock_override(chord)
        } else {
            crate::browser_keys::lock_override(chord)
                .or_else(|| crate::browser_keys::browser_override(chord))
        };
        // Neither table knows it: not consumed, so `key_down` carries on
        // to `body.key()`, which forwards a Cmd chord to
        // `Surface::edit_chord` (copy, paste, cut, select-all, undo, redo)
        // exactly as an unlocked browser card already does. A Chrome window
        // has no app keymap behind it to fall back to; this one does, and
        // swallowing here used to hide it.
        return id.map(String::from);
    }
    let override_ = (kind == Some(crate::saved_layout::CardKind::Browser))
        .then(|| crate::browser_keys::browser_override(chord))
        .flatten();
    override_
        .map(String::from)
        .or_else(|| crate::keymap::lookup(&m.keymap, chord).map(String::from))
}

/// `Ctrl` plus a single digit, the one Ctrl range a locked browser card
/// does not claim (see `handle_chord`).
fn is_workspace_switch(chord: &str) -> bool {
    let parts: Vec<&str> = chord.split('+').collect();
    parts.len() == 2
        && parts[0] == "ctrl"
        && parts[1].len() == 1
        && parts[1].chars().all(|c| c.is_ascii_digit())
}

/// The other carve-outs a locked card does not claim: the way out of a
/// locked page (`browser.leave`, bound to `cmd+escape` in the default
/// keymap; a no-op on an editor, which unlocks by double-Escape only) and,
/// for a BROWSER only, the omnibox (`cmd+l`, an app overlay, not the page
/// — the design doc: "the omnibox already does this"). A locked editor
/// does not carve `cmd+l` out: 2026-09-24, Ekin wants an editor's own
/// shortcuts (`editor_keys::editor_keeps`) to shadow the app's while
/// locked the same way Chrome's do for a browser, and Cmd+L there is
/// "select the line", not the address bar. Hardcoded rather than a keymap
/// lookup, the same way `is_workspace_switch` is: this is about what these
/// chords MEAN to the app, not about tracking a rebind.
fn is_locked_carveout(chord: &str, kind: Option<crate::saved_layout::CardKind>) -> bool {
    // Ctrl+Tab is the card switcher, a way OUT of a locked card like
    // Cmd+Escape; Chrome's own next-tab is Cmd+Shift+] here.
    if matches!(chord, "cmd+escape" | "ctrl+tab" | "ctrl+shift+tab") {
        return true;
    }
    chord == "cmd+l" && kind != Some(crate::saved_layout::CardKind::Editor)
}

/// Runs a command and then any `RunCommand` effects it queued, so a palette
/// entry that forwards to another command is one call for the ui.
pub fn run_with_effects(m: &mut Model, r: &CommandRegistry<Model>, id: &str) {
    r.run(id, m);
    let mut guard = 0;
    while let Some(i) = m
        .effects
        .iter()
        .position(|e| matches!(e, super::Effect::RunCommand(_)))
    {
        let super::Effect::RunCommand(next) = m.effects.remove(i) else {
            unreachable!()
        };
        guard += 1;
        if guard > 8 {
            break;
        }
        r.run(&next, m);
    }
}

#[cfg(test)]
mod tests {
    use super::super::palette_state::Source;
    use super::super::*;
    use super::*;
    use crate::backend::PaneEvent;
    use crate::grid::Size;
    use crate::saved_layout::CardKind;

    struct Harness {
        m: Model,
        r: CommandRegistry<Model>,
    }

    impl Harness {
        fn new() -> Harness {
            let mut r = CommandRegistry::new(|_| {});
            register_commands(&mut r);
            r.set_context(describe_context);
            let mut m = Model::new();
            m.view_size = Size { w: 1600., h: 1000. };
            // The placement, focus and swap tests below were written against
            // the fixed 69 by 80 card, the default until 2026-09-24; they are
            // about geometry, not about the default size, so they keep it.
            m.config.cards.width = 69.;
            m.config.cards.height = 80.;
            m.home = "/Users/me".into();
            m.start_dir = "/Users/me".into();
            m.now_ms = 1_000_000.;
            m.load_layout(None);
            // The placement tests count from one seeded terminal; the
            // welcome card a real first launch adds has its own tests.
            m.first_run = false;
            let mut seeded = false;
            m.seed_first_card(&mut seeded);
            m.take_effects();
            Harness { m, r }
        }

        fn run(&mut self, id: &str) -> Vec<Effect> {
            run_with_effects(&mut self.m, &self.r, id);
            self.m.take_effects()
        }

        fn focused(&self) -> &Card {
            self.m.focused().expect("a focused card")
        }
    }

    /// Four cards, the first focused on purpose; returns their ids.
    fn four_cards(h: &mut Harness) -> Vec<String> {
        for _ in 0..3 {
            h.run("card.new.terminal");
        }
        let ids: Vec<String> = h.m.cards.iter().map(|c| c.id.clone()).collect();
        assert_eq!(ids.len(), 4);
        // Making a card focuses it on purpose, which earns it a place;
        // start the story from a trail of one.
        h.m.focus_trail.clear();
        h.m.set_focus(Some(&ids[0]));
        ids
    }

    // Cmd+T lands in block order whatever is focused: focus the first
    // card, and the fifth still starts column three, the sixth goes under
    // it; close card 2 and the next Cmd+T fills its hole.
    #[test]
    fn new_cards_fill_the_block_whatever_is_focused() {
        let mut h = Harness::new();
        for _ in 0..3 {
            h.run("card.new.terminal");
        }
        let first = h.m.cards[0].clone();
        let (w, hgt) = (first.rect.w, first.rect.h);
        h.m.set_focus(Some(&first.id));
        h.run("card.new.terminal");
        h.run("card.new.terminal");
        let at = |c: &Card| {
            (
                ((c.rect.x - first.rect.x) / w).round() as i64,
                ((c.rect.y - first.rect.y) / hgt).round() as i64,
            )
        };
        let spots: Vec<(i64, i64)> = h.m.cards.iter().map(at).collect();
        assert_eq!(spots, [(0, 0), (1, 0), (0, 1), (1, 1), (2, 0), (2, 1)]);
        let second = h.m.cards[1].id.clone();
        h.m.set_focus(Some(&second));
        h.run("card.close");
        h.run("card.new.terminal");
        assert_eq!(at(h.m.cards.last().unwrap()), (1, 0), "the hole first");
    }

    // Cmd+Alt+Arrow across two cards on the way to a third leaves no trace:
    // the switcher's second row is the card you worked in, not one you
    // crossed.
    #[test]
    fn walking_through_cards_does_not_put_them_in_the_switcher() {
        let mut h = Harness::new();
        let ids = four_cards(&mut h);
        let t = h.m.now_ms;
        h.m.now_ms = t + 100.;
        h.m.focus_traversing(Some(&ids[1]));
        h.m.now_ms = t + 200.;
        h.m.focus_traversing(Some(&ids[2]));
        h.m.now_ms = t + 300.;
        h.m.focus_traversing(Some(&ids[3]));
        assert!(!h.m.focus_trail.contains(&ids[1]));
        assert!(!h.m.focus_trail.contains(&ids[2]));
        h.run("card.switcher.next");
        let s = h.m.switcher.clone().expect("the switcher is up");
        assert_eq!(s.list[0], ids[3], "the card you are in first");
        assert_eq!(
            s.list[1], ids[0],
            "then the one you chose, not the ones crossed"
        );
        assert_eq!(s.index, 1, "one press lands on it");
    }

    // Staying earns a walked-into card its place; so does typing into it.
    #[test]
    fn a_walked_into_card_earns_its_place_by_staying_or_typing() {
        let mut h = Harness::new();
        let ids = four_cards(&mut h);
        let t = h.m.now_ms;
        h.m.focus_traversing(Some(&ids[1]));
        h.m.tick(t + crate::switcher::TRAIL_DWELL_MS + 1.);
        assert_eq!(h.m.focus_trail.last(), Some(&ids[1]), "stayed in: earned");
        h.m.focus_traversing(Some(&ids[2]));
        h.m.note_input();
        assert_eq!(h.m.focus_trail.last(), Some(&ids[2]), "typed in: earned");
    }

    // A done card goes grey once it has been looked at, with the app in
    // front; the others stay green until you get to them.
    #[test]
    fn a_done_card_clears_once_seen() {
        use crate::agent_state::AgentState::{Done, None};
        let mut h = Harness::new();
        let ids = four_cards(&mut h);
        for id in &ids {
            h.m.card_mut(id).unwrap().agent = Done;
        }
        h.m.set_focus(Some(&ids[1]));
        let t = h.m.now_ms;
        h.m.tick(t);
        h.m.tick(t + crate::model::SEEN_MS - 1.);
        assert_eq!(
            h.m.card(&ids[1]).unwrap().agent,
            Done,
            "a glance is not a read"
        );
        assert!(h.m.done_seen_due(t + crate::model::SEEN_MS));
        h.m.tick(t + crate::model::SEEN_MS);
        assert_eq!(h.m.card(&ids[1]).unwrap().agent, None);
        assert_eq!(h.m.card(&ids[2]).unwrap().agent, Done, "not looked at");
        // With another app in front nothing is seen.
        h.m.app_active = false;
        h.m.set_focus(Some(&ids[2]));
        h.m.tick(t + 10_000.);
        h.m.tick(t + 20_000.);
        assert_eq!(h.m.card(&ids[2]).unwrap().agent, Done);
        // Typing into it reads it at once.
        h.m.app_active = true;
        h.m.note_input();
        assert_eq!(h.m.card(&ids[2]).unwrap().agent, None);
    }

    // Release commits: the selected card is focused; Escape leaves things
    // where they were. A card in another workspace brings its workspace.
    #[test]
    fn the_switcher_commits_to_the_selection_and_cancel_changes_nothing() {
        let mut h = Harness::new();
        let ids = four_cards(&mut h);
        h.m.set_focus(Some(&ids[1]));
        h.m.set_focus(Some(&ids[2]));
        h.run("card.switcher.next");
        h.run("card.switcher.next");
        h.m.switcher_cancel();
        assert!(h.m.switcher.is_none());
        assert_eq!(h.focused().id, ids[2], "cancel moved nothing");
        h.run("card.switcher.next");
        h.m.switcher_commit();
        assert!(h.m.switcher.is_none());
        assert_eq!(h.focused().id, ids[1], "back to the previous card");
        // Backwards from the top wraps to the oldest.
        h.run("card.switcher.prev");
        assert_eq!(h.m.switcher.as_ref().unwrap().index, 3);
    }

    // #101: with `ui.workspaceIsolation` the switcher lists this workspace's
    // cards only and the count is this workspace's.
    #[test]
    fn workspace_isolation_limits_the_switcher_and_the_count() {
        let mut h = Harness::new();
        let here = four_cards(&mut h);
        let first_workspace = h.m.here().len();
        h.run("workspace.new");
        h.run("card.new.terminal");
        let there = h.focused().id.clone();
        assert!(!here.contains(&there));
        let all = h.m.cards.len();
        assert!(
            all > first_workspace,
            "the new workspace has cards of its own"
        );
        h.m.set_focus(Some(&here[0]));
        // Off: the other workspace's card is a row and counts.
        assert_eq!(h.m.card_count(), all);
        h.run("card.switcher.next");
        assert_eq!(h.m.switcher.as_ref().unwrap().list.len(), all);
        h.m.switcher_cancel();
        h.m.apply_settings_text(r#"{"ui.workspaceIsolation": true}"#);
        h.run("workspace.prev");
        h.m.set_focus(Some(&here[0]));
        assert_eq!(h.m.card_count(), first_workspace);
        h.run("card.switcher.next");
        let list = h.m.switcher.as_ref().unwrap().list.clone();
        assert_eq!(list.len(), first_workspace);
        assert!(!list.contains(&there));
    }

    // Ctrl+Tab is a way out of a locked browser card, like Cmd+Escape.
    #[test]
    fn ctrl_tab_reaches_the_switcher_from_a_locked_card() {
        let mut h = Harness::new();
        let ids = four_cards(&mut h);
        h.m.set_focus(Some(&ids[1]));
        h.m.cards[1].kind = CardKind::Browser;
        h.m.cards[1].locked = true;
        assert_eq!(
            resolve_chord(&h.m, "ctrl+tab").as_deref(),
            Some("card.switcher.next")
        );
    }

    // The palette finds the keycast by the word people call it.
    #[test]
    fn the_palette_finds_the_keycast_by_its_name() {
        let h = Harness::new();
        let labels: Vec<(&str, &str)> =
            h.r.all()
                .iter()
                .map(|c| (c.id.as_str(), c.label.as_str()))
                .collect();
        let items = h.m.palette_items(Source::Commands, &labels);
        let ranked = crate::palette::rank(&items, "keycast", 10, |_| 0.);
        assert_eq!(
            ranked.items.first().map(|r| r.item.id.as_str()),
            Some("app.keycast")
        );
    }

    // A closed card's number goes back into use, and a card reopened after
    // its number was taken gets a free one rather than a twin.
    #[test]
    fn numbers_are_the_lowest_free_and_never_doubled() {
        assert_eq!(lowest_free_number(&[]), 1);
        assert_eq!(lowest_free_number(&[1, 2, 4]), 3);
        let mut h = Harness::new();
        h.run("card.new.terminal");
        h.run("card.new.terminal");
        let numbers = |h: &Harness| {
            let mut n: Vec<u32> = h.m.cards.iter().map(|c| c.number).collect();
            n.sort();
            n
        };
        assert_eq!(numbers(&h), [1, 2, 3]);
        // Close #2, and the next card is #2 again, not #4.
        let two = h.m.cards.iter().find(|c| c.number == 2).unwrap().clone();
        h.m.set_focus(Some(&two.id));
        h.run("card.close");
        h.run("card.new.terminal");
        assert_eq!(numbers(&h), [1, 2, 3]);
        // The old #2 comes back (Cmd+Z, `card.reopen`): not a second #2.
        h.m.reopen_card(two);
        let n = numbers(&h);
        assert_eq!(n.len(), 4);
        n.windows(2).for_each(|w| assert_ne!(w[0], w[1], "{n:?}"));
    }

    // Every default binding names a command that exists: a binding pointing
    // at nothing reads as a broken shortcut.
    #[test]
    fn every_default_binding_is_registered() {
        let h = Harness::new();
        let ids: Vec<&str> = h.r.all().iter().map(|c| c.id.as_str()).collect();
        let missing = crate::keymap::unregistered_bindings(&crate::keymap::default_keymap(), &ids);
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn every_label_carries_its_domain_prefix() {
        let h = Harness::new();
        for c in h.r.all() {
            // The two split labels and `Run a command` are the reference's exceptions.
            let exempt = c.id.starts_with("card.split.") || c.id == "app.palette";
            assert!(exempt || c.label.contains(':'), "{} -> {}", c.id, c.label);
        }
    }

    // The first-card effect fires ONCE: closing the last card must reach an
    // empty canvas, not a replacement.
    #[test]
    fn the_first_card_is_seeded_once() {
        let mut h = Harness::new();
        assert_eq!(h.m.cards.len(), 1);
        assert!(h.m.selection.focused_id.is_some());
        let mut seeded = true;
        h.run("card.close");
        h.m.seed_first_card(&mut seeded);
        assert!(h.m.cards.is_empty());
        assert_eq!(h.m.selection.focused_id, None);
    }

    // A first launch (no save file) seeds the "Start here" card alone,
    // focused; the newcomer makes the first terminal from it.
    #[test]
    fn a_first_launch_opens_only_the_welcome_card() {
        let mut m = Model::new();
        m.view_size = Size { w: 1600., h: 1000. };
        m.home = "/Users/me".into();
        m.start_dir = "/Users/me".into();
        m.load_layout(None);
        assert!(m.first_run);
        let mut seeded = false;
        m.seed_first_card(&mut seeded);
        assert_eq!(m.cards.len(), 1);
        let welcome = crate::welcome::welcome_path()
            .to_string_lossy()
            .into_owned();
        let card = m.focused().unwrap();
        assert_eq!(card.kind, CardKind::Page);
        assert_eq!(card.path.as_deref(), Some(welcome.as_str()));
        assert!(m.layout_undo.is_empty(), "the seed is not an undo step");
        // Centred in the window (screen point = (world - origin) * scale),
        // at once rather than by a glide.
        let r = card.rect;
        let v = m.viewport;
        let centre_x = (r.x + r.w / 2. - v.x) * v.scale;
        let centre_y = (r.y + r.h / 2. - v.y) * v.scale;
        assert!((centre_x - 800.).abs() < 1., "x centre {centre_x}");
        assert!((centre_y - 500.).abs() < 1., "y centre {centre_y}");
        assert!(!m
            .take_effects()
            .iter()
            .any(|e| matches!(e, Effect::AnimatePan { .. } | Effect::AnimateFit(_))));
    }

    // What the welcome card promises: Cmd+T opens the first terminal
    // beside it, and Cmd+Alt+Left comes back to it.
    #[test]
    fn the_first_terminal_opens_right_of_the_welcome_card() {
        let mut r = CommandRegistry::new(|_| {});
        register_commands(&mut r);
        let mut m = Model::new();
        m.view_size = Size { w: 1600., h: 1000. };
        m.home = "/Users/me".into();
        m.start_dir = "/Users/me".into();
        m.load_layout(None);
        let mut seeded = false;
        m.seed_first_card(&mut seeded);
        let welcome = m.focused().unwrap().clone();
        run_with_effects(&mut m, &r, "card.new.terminal");
        let term = m.focused().unwrap().clone();
        assert_eq!(term.kind, CardKind::Terminal);
        assert!(term.rect.x > welcome.rect.x, "beside it, to the right");
        assert_eq!(term.rect.y, welcome.rect.y);
        run_with_effects(&mut m, &r, "focus.move.left");
        assert_eq!(m.focused().unwrap().id, welcome.id);
    }

    // An existing canvas that happens to be empty gets the one terminal.
    #[test]
    fn a_later_launch_on_an_empty_canvas_gets_no_welcome_card() {
        let mut m = Model::new();
        m.view_size = Size { w: 1600., h: 1000. };
        m.home = "/Users/me".into();
        m.start_dir = "/Users/me".into();
        m.load_layout(Some("{}"));
        assert!(!m.first_run);
        let mut seeded = false;
        m.seed_first_card(&mut seeded);
        assert_eq!(m.cards.len(), 1);
    }

    #[test]
    fn help_docs_opens_the_first_page_with_the_folder_beside_it() {
        let mut h = Harness::new();
        h.run("help.docs");
        let dir = crate::help_docs::docs_dir().to_string_lossy().into_owned();
        let card = h.focused().clone();
        assert_eq!(card.kind, CardKind::Editor);
        assert_eq!(card.root.as_deref(), Some(dir.as_str()));
        assert!(card.explorer, "the tree is the table of contents");
        assert!(card
            .path
            .as_deref()
            .unwrap()
            .ends_with(crate::help_docs::FIRST_PAGE));
        h.run("help.docs");
        assert_eq!(h.m.cards.len(), 2, "a second run focuses the open one");
    }

    #[test]
    fn help_welcome_opens_the_card_once() {
        let mut h = Harness::new();
        h.run("help.welcome");
        assert_eq!(h.m.cards.len(), 2);
        assert_eq!(h.focused().kind, CardKind::Page);
        h.run("help.welcome");
        assert_eq!(h.m.cards.len(), 2, "a second run focuses the open one");
    }

    // New cards take the first free slot AFTER the active card: beside it.
    #[test]
    fn a_new_card_opens_beside_the_active_one() {
        let mut h = Harness::new();
        let first = h.focused().rect;
        h.run("card.new.terminal");
        let second = h.focused().rect;
        assert_eq!(h.m.cards.len(), 2);
        assert_eq!(second.y, first.y);
        assert!(second.x > first.x, "{second:?} beside {first:?}");
    }

    // A new card takes the GROUP and not the directory. Opening a card to
    // start something else and landing in the last card's directory is a
    // surprise you only notice after you have run the wrong command there.
    #[test]
    fn a_new_card_inherits_the_group_but_never_the_directory() {
        let mut h = Harness::new();
        let first = h.focused().id.clone();
        h.m.card_mut(&first).unwrap().cwd = "/Users/me/Code/api".into();
        let g = h.m.add_group("api");
        h.m.card_mut(&first).unwrap().group_id = Some(g.clone());
        let start = h.m.start_dir.clone();
        h.run("card.new.terminal");
        assert_eq!(h.focused().group_id.as_deref(), Some(g.as_str()));
        assert_eq!(h.focused().cwd, start, "a new card starts at startingDir");
        h.m.set_focus(Some(&first));
        h.run("card.new.ungrouped");
        assert_eq!(h.focused().group_id, None);
        assert_eq!(h.focused().cwd, start);
    }

    // The other half of the rule: a split IS about carrying on where you
    // are, so it keeps the directory. This is the pair that has to move
    // together; changing one without the other is the bug being fixed.
    #[test]
    fn a_split_keeps_the_directory_it_was_carved_out_of() {
        let mut h = Harness::new();
        let first = h.focused().id.clone();
        h.m.card_mut(&first).unwrap().cwd = "/Users/me/Code/api".into();
        h.run("card.split.right");
        assert_eq!(h.focused().cwd, "/Users/me/Code/api");
    }

    // The restart is safe only because the daemon backend DETACHES on quit:
    // the shells and whatever is running in them survive and the cards
    // adopt them again. On a local pty the same chord would end every
    // process in every card, so it refuses and says why instead.
    #[test]
    fn restart_refuses_on_a_backend_that_would_kill_the_shells() {
        use crate::config::TerminalBackend;
        let mut h = Harness::new();

        h.m.config.terminal.backend = TerminalBackend::Pty;
        let effects = h.run("app.restart");
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::Restart)),
            "pty must not restart"
        );
        assert!(h.m.notice.is_some(), "and it says why");

        h.m.config.terminal.backend = TerminalBackend::Tmux;
        let effects = h.run("app.restart");
        assert!(!effects.iter().any(|e| matches!(e, Effect::Restart)));

        h.m.config.terminal.backend = TerminalBackend::Daemon;
        let effects = h.run("app.restart");
        assert!(
            effects.iter().any(|e| matches!(e, Effect::Restart)),
            "the daemon keeps the shells, so it may"
        );
    }

    // Focus after a close goes back to the card focused BEFORE the closed
    // one: an editor opened from a terminal closes back onto the terminal,
    // however the cards lie.
    #[test]
    fn closing_returns_to_the_card_focused_before() {
        let mut h = Harness::new();
        let a = h.focused().id.clone();
        h.run("card.new.terminal");
        let b = h.focused().id.clone();
        h.run("card.new.terminal");
        let c = h.focused().id.clone();
        // c is nearest to b; a was focused last.
        h.m.set_focus(Some(&a));
        h.m.set_focus(Some(&b));
        // a is off screen: the close pans to it.
        h.m.card_mut(&a).unwrap().rect.x += 20_000.;
        let effects = h.run("card.close");
        assert_eq!(h.focused().id, a);
        assert!(h.m.card(&b).is_none());
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::AnimatePan { .. })));
        // And back once more, past the closed card, to c.
        h.run("card.close");
        assert_eq!(h.focused().id, c);
    }

    // With no trail to follow (a restart has none), the NEAREST card by
    // geometry, not the next in the list.
    #[test]
    fn closing_hands_focus_to_the_nearest_card() {
        let mut h = Harness::new();
        let a = h.focused().id.clone();
        h.run("card.new.terminal");
        let b = h.focused().id.clone();
        h.run("card.new.terminal");
        let c = h.focused().id.clone();
        // Put c far away, then close b: a is nearest.
        h.m.card_mut(&c).unwrap().rect.x += 20_000.;
        h.m.set_focus(Some(&b));
        h.m.focus_trail.clear();
        let effects = h.run("card.close");
        assert_eq!(h.focused().id, a);
        assert!(h.m.card(&b).is_none());
        // No pane yet, so nothing to kill; the log line is there.
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::Log(l) if l.starts_with("close "))));
    }

    // The mask is a toggle on the focused card, a maximised one included.
    #[test]
    fn masking_toggles_the_focused_card() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        h.m.selection.maximized = true;
        h.run("card.mask");
        assert!(h.m.card(&id).unwrap().masked);
        h.run("card.mask");
        assert!(!h.m.card(&id).unwrap().masked);
    }

    // `ift attach` displacing the app is not an exit: the card stays, with
    // its pane, marked so the ui can take the session back later.
    #[test]
    fn a_detached_pane_marks_the_card_and_closes_nothing() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        h.m.card_mut(&id).unwrap().pane_id = Some(7);
        h.m.apply_pane_event(7, &PaneEvent::Detached);
        let card = h.m.card(&id).unwrap();
        assert!(card.displaced);
        assert_eq!(card.pane_id, Some(7));
        assert_eq!(h.m.cards.len(), 1);
    }

    #[test]
    fn a_shell_exit_closes_the_card_the_same_way() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        h.m.card_mut(&id).unwrap().pane_id = Some(7);
        h.m.apply_pane_event(7, &PaneEvent::Exited { code: 0 });
        assert!(h.m.cards.is_empty());
        // Already exited: no kill.
        assert!(!h
            .m
            .take_effects()
            .iter()
            .any(|e| matches!(e, Effect::KillPane(_))));
    }

    // A split halves the card; closing the new half gives the space back.
    #[test]
    fn a_split_half_reclaims_its_partner_on_close() {
        let mut h = Harness::new();
        let original = h.focused().rect;
        let kept_id = h.focused().id.clone();
        h.run("card.split.right");
        let made = h.focused().clone();
        assert_eq!(made.split_from.as_deref(), Some(kept_id.as_str()));
        assert!(h.m.card(&kept_id).unwrap().rect.w < original.w);
        let effects = h.run("card.close");
        assert_eq!(h.m.card(&kept_id).unwrap().rect, original);
        assert_eq!(h.focused().id, kept_id);
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::MarkSwap(ids) if ids.len() == 1 && ids[0].0 == kept_id && ids[0].1 != original)));
        // A soft group of one is nothing.
        assert_eq!(h.m.card(&kept_id).unwrap().soft_group_id, None);
    }

    // "#7" is how a card is named to somebody else, shown ahead of the
    // label. Since 2026-09-24 a closed card's number goes back into use, so
    // numbers stay small (Ekin: a week of cards would have read #2332).
    #[test]
    fn cards_are_numbered_and_the_number_leads_the_label() {
        let mut h = Harness::new();
        let first = h.focused().clone();
        assert_eq!(first.number, 1);
        assert!(h.m.numbered_label(&first).starts_with("#1 "));
        h.run("card.new.terminal");
        assert_eq!(h.focused().number, 2);
        h.run("card.close");
        h.run("card.new.terminal");
        assert_eq!(h.focused().number, 2, "a closed card's number is reused");
        // The bare label is for names that become something else.
        assert!(!h.m.label_of(h.focused()).starts_with('#'));
    }

    // Split down to a quarter and keep only the quarter: the other three
    // close without the survivor growing, the space stays free, and the
    // survivor is no longer half of a pair.
    #[test]
    fn close_leave_keeps_the_partner_at_its_size() {
        let mut h = Harness::new();
        let kept_id = h.focused().id.clone();
        h.run("card.split.right");
        let half = h.m.card(&kept_id).unwrap().rect;
        let made = h.focused().id.clone();
        let effects = h.run("card.close.leave");
        assert!(h.m.card(&made).is_none());
        assert_eq!(h.m.card(&kept_id).unwrap().rect, half, "not reclaimed");
        assert!(!effects.iter().any(|e| matches!(e, Effect::MarkSwap(_))));
        assert_eq!(h.focused().id, kept_id);
        assert_eq!(h.m.card(&kept_id).unwrap().soft_group_id, None);
        // The space is free, and the phantom beside the survivor is the
        // survivor's size, so the arrow lands exactly in the freed half.
        h.run("focus.move.right");
        let phantom = h.m.selection.phantom.clone().expect("a slot, not a card");
        assert_eq!((phantom.rect.w, phantom.rect.h), (half.w, half.h));
        assert!(phantom.rect.x > half.x + half.w - 1.);
        h.run("focus.move.left");
        assert_eq!(h.focused().id, kept_id);
        // And the survivor can be moved into it.
        h.run("card.swap.right");
        let moved = h.m.card(&kept_id).unwrap().rect;
        assert!(
            moved.x > half.x && (moved.y - half.y).abs() < 1.,
            "{moved:?} right of {half:?}"
        );
    }

    // The way back up: a quarter kept after Cmd+Ctrl+W grows to the
    // default size from its corner when the space is free, and stays put
    // with a word when a card is in the way.
    #[test]
    fn full_size_restores_the_default_when_the_space_is_free() {
        let mut h = Harness::new();
        let kept = h.focused().id.clone();
        let original = h.focused().rect;
        h.run("card.split.right");
        let half = h.m.card(&kept).unwrap().rect;
        assert!(half.w < original.w);
        h.m.set_focus(Some(&kept));
        // The split partner blocks the right, so it grows LEFT into the
        // free canvas, its right edge where it was (card #96's case).
        let effects = h.run("card.size.reset");
        let grown = h.m.card(&kept).unwrap().rect;
        assert_eq!(grown.w, original.w, "{grown:?}");
        assert_eq!(grown.x + grown.w, half.x + half.w, "the right edge stays");
        assert!(
            effects.iter().any(|e| matches!(e, Effect::MarkSwap(_))),
            "animated"
        );
        // Already the default size: nothing to grow into, and it says so.
        h.run("card.size.reset");
        assert_eq!(h.m.card(&kept).unwrap().rect, grown);
        assert!(h.m.notice.as_deref().is_some_and(|n| n.contains("no room")));

        // Close the partner leaving its space: the top-left corner grows
        // back into it, and the card is where it started.
        let mut h = Harness::new();
        let kept = h.focused().id.clone();
        let original = h.focused().rect;
        h.run("card.split.right");
        let partner = h.m.cards.iter().find(|c| c.id != kept).unwrap().id.clone();
        h.m.set_focus(Some(&partner));
        h.run("card.close.leave");
        h.m.set_focus(Some(&kept));
        h.run("card.size.reset");
        assert_eq!(h.m.card(&kept).unwrap().rect, original);
        assert_eq!(h.m.card(&kept).unwrap().soft_group_id, None);
    }

    // Cmd+Ctrl+S: the file's names, a paste into the focused card, and the
    // last row opens the file itself.
    #[test]
    fn the_snippet_picker_pastes_into_the_focused_card_and_opens_its_file() {
        use crate::snippets::{Snippet, EDIT_ROW};
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        let effects = h.run("snippet.paste");
        assert!(effects.iter().any(|e| matches!(e, Effect::RefreshSnippets)));
        assert_eq!(h.m.palette.source, Some(Source::Snippets));
        assert!(!Source::Snippets.keeps_its_order(), "the daily one rises");
        // The ui would have read the file; stand in for it.
        h.m.snippets = vec![Snippet {
            name: "review".into(),
            text: "line one\nline two".into(),
        }];
        let items = h.m.palette_items(Source::Snippets, &[]);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "review");
        assert_eq!(items[0].hint.as_deref(), Some("line one"));
        assert_eq!(items[1].id, EDIT_ROW);
        h.m.palette_run(Source::Snippets, "review");
        let pasted = h.m.effects.iter().find_map(|e| match e {
            Effect::PasteText { card_id, text } => Some((card_id.clone(), text.clone())),
            _ => None,
        });
        assert_eq!(pasted, Some((id.clone(), "line one\nline two".into())));
        // An unknown name pastes nothing.
        h.m.effects.clear();
        h.m.palette_run(Source::Snippets, "nope");
        assert!(!h
            .m
            .effects
            .iter()
            .any(|e| matches!(e, Effect::PasteText { .. })));
        // The edit row opens the snippets FOLDER in an editor card (its
        // tree lists the files), once.
        h.m.palette_run(Source::Snippets, EDIT_ROW);
        let editors: Vec<_> =
            h.m.cards
                .iter()
                .filter(|c| c.kind == CardKind::Editor)
                .collect();
        assert_eq!(editors.len(), 1);
        assert!(editors[0].root.as_deref().unwrap().ends_with("/snippets"));
        let editor = editors[0].id.clone();
        h.m.set_focus(Some(&id));
        h.m.palette_run(Source::Snippets, EDIT_ROW);
        assert_eq!(
            h.m.cards
                .iter()
                .filter(|c| c.kind == CardKind::Editor)
                .count(),
            1,
            "the card already showing it is focused instead"
        );
        assert_eq!(h.focused().id, editor);
    }

    // Cmd+Alt+R: one Enter to a quarter, and the rows stay in their order.
    // Growing needs the room; shrinking always fits.
    #[test]
    fn the_size_picker_resizes_from_the_corner_and_keeps_its_order() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        let full = h.focused().rect;
        h.run("card.size");
        assert_eq!(h.m.palette.source, Some(Source::Sizes));
        assert!(Source::Sizes.keeps_its_order());
        let items = h.m.palette_items(Source::Sizes, &[]);
        assert_eq!(items[0].id, "full");
        assert_eq!(items[3].id, "quarter");
        h.m.palette_run(Source::Sizes, "quarter");
        let q = h.m.card(&id).unwrap().rect;
        assert_eq!((q.x, q.y), (full.x, full.y));
        assert!(q.w < full.w / 2. + 1. && q.h < full.h / 2. + 1., "{q:?}");
        // Back to full: the space is still free, so it grows.
        h.m.palette_run(Source::Sizes, "full");
        assert_eq!(h.m.card(&id).unwrap().rect, full);
        // Double wide, with a card in the way: stays, and says so.
        h.run("card.new.terminal");
        h.m.set_focus(Some(&id));
        h.m.palette_run(Source::Sizes, "double-wide");
        assert_eq!(h.m.card(&id).unwrap().rect, full);
        assert!(h.m.notice.as_deref().is_some_and(|n| n.contains("no room")));
    }

    // A dragged card stays put until the drop: the ghost goes where the
    // pointer says, and the card follows only onto free space.
    #[test]
    fn a_drop_moves_the_card_only_onto_free_space() {
        let mut h = Harness::new();
        let a = h.focused().id.clone();
        let a_rect = h.focused().rect;
        h.run("card.new.terminal");
        let b_rect = h.focused().rect;
        // Half over b and half over space: refused, a is where it was, and
        // it says so. (Squarely on b it would swap; see the test below.)
        let straddle = Rect {
            x: b_rect.x - b_rect.w / 2. - 25., // centred on the gutter: on neither card
            ..b_rect
        };
        assert!(!h.m.drop_card(&a, straddle));
        assert_eq!(h.m.card(&a).unwrap().rect, a_rect);
        assert!(h.m.notice.as_deref().is_some_and(|n| n.contains("overlap")));
        // Onto free canvas, off-grid by a little: snapped there, animated.
        let target = Rect {
            x: a_rect.x + 7.,
            y: a_rect.y + a_rect.h + 3000.,
            ..a_rect
        };
        assert!(h.m.drop_card(&a, target));
        let landed = h.m.card(&a).unwrap().rect;
        assert_eq!(landed, crate::grid::snap_rect(target));
        assert!(h
            .m
            .take_effects()
            .iter()
            .any(|e| matches!(e, Effect::MarkSwap(_))));
        // Back where it is: nothing to do.
        assert!(h.m.drop_card(&a, landed));
        assert!(h.m.rect_free_for(&a, landed));
    }

    // Dropped with its centre on another card, the two swap places, the
    // way Cmd+Alt+Shift+Arrow does; straddling two, nothing moves.
    #[test]
    fn a_drop_on_a_card_swaps_the_two() {
        let mut h = Harness::new();
        let a = h.focused().id.clone();
        let a_rect = h.focused().rect;
        h.run("card.new.terminal");
        let b = h.focused().id.clone();
        let b_rect = h.focused().rect;
        let onto_b = Rect {
            x: b_rect.x + 40.,
            y: b_rect.y + 25.,
            ..a_rect
        };
        assert!(h.m.drop_card(&a, onto_b));
        assert_eq!(h.m.card(&a).unwrap().rect, b_rect);
        assert_eq!(h.m.card(&b).unwrap().rect, a_rect);
        // Cmd+Z puts both back; Cmd+Shift+Z swaps them again.
        h.run("layout.undo");
        assert_eq!(h.m.card(&a).unwrap().rect, a_rect);
        assert_eq!(h.m.card(&b).unwrap().rect, b_rect);
        h.run("layout.redo");
        assert_eq!(h.m.card(&a).unwrap().rect, b_rect);
        // Nothing left to redo, and it says so rather than doing nothing.
        h.run("layout.redo");
        assert!(h
            .m
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("nothing to redo")));
    }

    // The undo trail covers swaps, nudges and the size picker, in order,
    // and a new change after an undo forgets the redo.
    #[test]
    fn layout_undo_walks_back_through_moves_and_resizes() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        let r0 = h.focused().rect;
        h.run("card.move.right");
        let r1 = h.focused().rect;
        h.m.palette_run(Source::Sizes, "quarter");
        let r2 = h.focused().rect;
        assert!(r0 != r1 && r1 != r2);
        h.run("layout.undo");
        assert_eq!(h.m.card(&id).unwrap().rect, r1);
        h.run("layout.undo");
        assert_eq!(h.m.card(&id).unwrap().rect, r0);
        h.run("layout.undo");
        assert!(h
            .m
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("nothing to undo")));
        h.run("layout.redo");
        assert_eq!(h.m.card(&id).unwrap().rect, r1);
        h.run("card.move.down");
        assert!(h.m.layout_redo.is_empty(), "a new change forgets the redo");
        // One card alone: the ghost snaps to the placement lattice.
        let free = Rect {
            x: r0.x + 30.,
            y: r0.y + r0.h + 40.,
            ..r0
        };
        let snapped = h.m.snap_ghost(&id, free);
        assert_eq!(
            (snapped.x, snapped.y),
            (r0.x, r0.y + r0.h + 25.),
            "{snapped:?}"
        );
    }

    // One trail: a move and a close undo in the order they happened.
    // Undoing a close brings the card back to its slot. Undo NEVER closes
    // a card: not a new one you may be working in, not by redoing a close.
    // A card reopened by hand is not reopened again by Cmd+Z.
    #[test]
    fn undo_walks_back_moves_and_closes_and_never_closes_a_card() {
        let mut h = Harness::new();
        let a = h.focused().id.clone();
        let a_rect = h.focused().rect;
        h.run("card.new.terminal");
        let b = h.focused().id.clone();
        h.run("card.move.right");
        h.m.set_focus(Some(&a));
        h.run("card.close");
        assert!(h.m.card(&a).is_none());
        h.run("layout.undo");
        assert!(h.m.card(&a).is_some(), "the close undone: a is back");
        assert_eq!(h.m.card(&a).unwrap().rect, a_rect, "in its slot");
        assert!(h.m.closed.is_empty(), "and not twice through Cmd+Ctrl+T");
        h.run("layout.undo");
        assert_eq!(
            h.m.card(&b).unwrap().rect.x,
            a_rect.x + a_rect.w + 25.,
            "the move undone"
        );
        h.run("layout.undo");
        assert!(
            h.m.card(&b).is_some(),
            "the new card stays: undo closes nothing"
        );
        h.run("layout.redo");
        h.run("layout.redo");
        assert!(h.m.card(&a).is_some(), "and redo does not close it again");
        // A close reopened by hand leaves the trail: Cmd+Z does not bring
        // a second copy, and does not close the reopened card.
        h.m.set_focus(Some(&b));
        h.run("card.close");
        h.run("card.reopen");
        h.run("layout.undo");
        assert_eq!(h.m.cards.iter().filter(|c| c.id == b).count(), 1);
    }

    // A locked card is not closed by Cmd+W, by its shell exiting, or by
    // its workspace closing; the label says so; it survives the save file.
    #[test]
    fn a_protected_card_survives_every_close() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        h.run("card.protect");
        assert!(h.m.card(&id).unwrap().protected);
        assert!(h
            .m
            .numbered_label(h.m.card(&id).unwrap())
            .starts_with('\u{1f512}'));
        h.run("card.close");
        assert!(h.m.card(&id).is_some(), "refused");
        assert!(h.m.notice.as_deref().is_some_and(|n| n.contains("locked")));
        h.m.card_mut(&id).unwrap().pane_id = Some(9);
        h.m.apply_pane_event(9, &PaneEvent::Exited { code: 0 });
        assert!(h.m.card(&id).is_some(), "the shell exited, the card stays");
        assert_eq!(
            h.m.card(&id).unwrap().pane_id,
            None,
            "and gets a fresh shell"
        );
        let ws = h.m.active_workspace.clone().unwrap();
        h.m.close_workspace_confirmed(&ws);
        assert!(h.m.card(&id).is_some());
        assert!(
            h.m.workspaces.iter().any(|w| w.id == ws),
            "the workspace stays too"
        );
        let text = h.m.save_text().unwrap();
        assert!(text.contains("\"protected\": true"));
        h.run("card.protect");
        assert!(!h.m.card(&id).unwrap().protected);
        assert!(
            !h.m.save_text().unwrap().contains("protected"),
            "written only when on"
        );
    }

    // Closing an editor card of several tabs asks first, locked or not,
    // unsaved or not: closing one tab was meant.
    #[test]
    fn closing_an_editor_with_tabs_asks_first() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        h.m.card_mut(&id).unwrap().kind = CardKind::Editor;
        h.m.card_mut(&id).unwrap().path = Some("/h/a.rs".into());
        h.m.browser_tab_open(&id, Some("/h/b.rs"));
        h.run("card.close");
        assert!(h.m.card(&id).is_some(), "asked, not closed");
        assert!(h.m.prompt.is_open() && h.m.prompt.confirm);
        let (pending, text) = h.m.prompt.settle(Some("")).unwrap();
        h.m.answer(pending, text, |_| true);
        assert!(h.m.card(&id).is_none(), "confirmed: closed");
    }

    // #160: the local colour is the setting `ui.windowColor`, applied when the
    // settings are, and a remote window ignores it.
    #[test]
    fn the_window_colour_setting_tints_the_local_window_and_not_a_remote_one() {
        let mut h = Harness::new();
        h.m.effects.clear();
        h.m.apply_settings_text(r#"{"ui.windowColor": "teal"}"#);
        assert_eq!(h.m.window_color(), crate::remote_identity::named("teal"));
        assert!(
            h.m.effects
                .iter()
                .any(|e| matches!(e, Effect::WindowColor { color: Some(_), .. })),
            "the Dock follows the file"
        );
        h.m.effects.clear();
        h.m.apply_settings_text(r#"{"ui.windowColor": "teal"}"#);
        assert!(
            !h.m.effects
                .iter()
                .any(|e| matches!(e, Effect::WindowColor { .. })),
            "nothing changed, nothing to redraw"
        );
        h.m.apply_settings_text("{}");
        assert_eq!(h.m.window_color(), None, "removed from the file: no colour");
        h.m.remote = crate::remote_identity::RemoteIdentity::from_vars(
            Some("ops@box"),
            None,
            Some("ff8800"),
        );
        h.m.apply_settings_text(r#"{"ui.windowColor": "teal"}"#);
        assert_eq!(
            h.m.window_color(),
            Some((0xff, 0x88, 0x00)),
            "a remote window wears its host's"
        );
    }

    // A remote instance starts its cards in the server's home, not in the
    // Mac folder its seeded settings name, and keeps doing so when the
    // settings are applied again.
    #[test]
    fn a_remote_instance_starts_cards_in_the_servers_home() {
        let mut h = Harness::new();
        h.m.remote = crate::remote_identity::RemoteIdentity::from_vars(Some("ops@box"), None, None);
        h.m.apply_settings_text(r#"{"startingDir": "/Users/me/Code"}"#);
        assert_eq!(h.m.start_dir, "~");
        h.m.remote = None;
        h.m.apply_settings_text(r#"{"startingDir": "/Users/me/Code"}"#);
        assert_eq!(
            h.m.start_dir, "/Users/me/Code",
            "a local instance is unchanged"
        );
    }

    // #118: a remote instance has no browser cards, however one is asked for.
    #[test]
    fn a_remote_instance_refuses_browser_cards() {
        let mut h = Harness::new();
        let before = h.m.cards.len();
        h.m.remote = crate::remote_identity::RemoteIdentity::from_vars(Some("ops@box"), None, None);
        h.m.new_beside_active(CardKind::Browser, None, Some("https://a".into()));
        assert_eq!(h.m.cards.len(), before, "no card");
        assert!(h
            .m
            .notice
            .as_deref()
            .unwrap_or("")
            .contains("remote instance"));
        h.m.notice = None;
        h.run("card.new.browser");
        assert!(
            !h.m.omni.open,
            "the address bar does not open for a new card"
        );
        assert!(h.m.notice.is_some());
        let plan = crate::ift::OpenPlan::Browser {
            cwd: "/tmp".into(),
            url: "https://a".into(),
        };
        assert_eq!(h.m.open_in_card(plan, None), None);
        assert_eq!(h.m.cards.len(), before);
        // A terminal is still fine, and so is a browser once it is not remote.
        h.run("card.new.terminal");
        assert_eq!(h.m.cards.len(), before + 1);
        h.m.remote = None;
        h.m.new_beside_active(CardKind::Browser, None, Some("https://a".into()));
        assert_eq!(h.m.cards.len(), before + 2);
    }

    // #90: `app.about` opens the About window. It has no buttons, so
    // closing it asks for nothing, not even an update check.
    #[test]
    fn about_opens_and_closing_it_checks_nothing() {
        let mut h = Harness::new();
        h.run("app.about");
        assert!(h.m.prompt.is_open() && h.m.prompt.about);
        let (pending, text) = h.m.prompt.press_choice().unwrap();
        h.m.answer(pending, text, |_| true);
        assert!(!h.m.prompt.is_open());
        assert!(!h
            .m
            .effects
            .iter()
            .any(|e| matches!(e, Effect::CheckForUpdate)));
    }

    // Grouping moves the selection to a free block right of the grid, as one block.
    #[test]
    fn grouping_names_the_group_and_moves_the_cards_together() {
        let mut h = Harness::new();
        let a = h.focused().id.clone();
        h.run("card.new.terminal");
        let b = h.focused().id.clone();
        h.m.focus_extended(&b, vec![a.clone()]);
        let (ra, rb) = (h.m.card(&a).unwrap().rect, h.m.card(&b).unwrap().rect);
        h.run("group.new");
        assert!(h.m.prompt.is_open());
        assert_eq!(h.m.prompt.label, "group name");
        let (pending, text) = h.m.prompt.settle(Some("api")).unwrap();
        h.m.answer(pending, text, |_| true);
        assert_eq!(h.m.groups.len(), 1);
        assert_eq!(h.m.groups[0].name, "api");
        let (na, nb) = (h.m.card(&a).unwrap().rect, h.m.card(&b).unwrap().rect);
        assert!(na.x > ra.x, "moved right of the grid");
        // The same delta for both: the selection kept its shape.
        assert_eq!(nb.x - na.x, rb.x - ra.x);
        assert_eq!(nb.y - na.y, rb.y - ra.y);
        // Dissolving releases the cards where they stand.
        h.run("group.dissolve");
        assert!(h.m.groups.is_empty());
        assert_eq!(h.m.card(&a).unwrap().rect, na);
    }

    // The block starts right of the cards that are ACTUALLY there. It used
    // to start right of a hypothetical twelve-card grid, which was true
    // while placement used a fixed column count; cards now take the nearest
    // free slot and spread as far as they like, and the group would land in
    // a gap inside the loose cluster rather than clear of it.
    #[test]
    fn a_new_group_lands_clear_of_every_loose_card() {
        let mut h = Harness::new();
        let a = h.focused().id.clone();
        // A loose card far out to the right, further than any grid would go.
        h.run("card.new.terminal");
        let far = h.focused().id.clone();
        let mut rect = h.m.card(&far).unwrap().rect;
        rect.x = 40_000.;
        h.m.card_mut(&far).unwrap().rect = rect;

        h.m.set_focus(Some(&a));
        h.run("group.new");
        let (pending, text) = h.m.prompt.settle(Some("api")).unwrap();
        h.m.answer(pending, text, |_| true);

        let grouped = h.m.card(&a).unwrap().rect;
        assert!(
            grouped.x > rect.x + rect.w,
            "the group at {} is not clear of the loose card ending at {}",
            grouped.x,
            rect.x + rect.w
        );
    }

    #[test]
    fn a_blank_group_name_is_a_cancel() {
        let mut h = Harness::new();
        h.run("group.new");
        let (pending, text) = h.m.prompt.settle(Some("   ")).unwrap();
        h.m.answer(pending, text, |_| true);
        assert!(h.m.groups.is_empty());
    }

    // Switching workspaces saves the viewport you leave and restores the one you enter.
    #[test]
    fn workspaces_each_keep_their_viewport_and_focus() {
        let mut h = Harness::new();
        let first_ws = h.m.active_workspace.clone().unwrap();
        let first_card = h.focused().id.clone();
        h.m.viewport.x = 500.;
        h.run("workspace.new");
        let second_ws = h.m.active_workspace.clone().unwrap();
        assert_ne!(first_ws, second_ws);
        assert_eq!(h.m.viewport.x, 0.);
        assert_eq!(h.m.cards.len(), 2, "a new workspace opens with a terminal");
        assert_ne!(h.focused().id, first_card);
        h.run("workspace.prev");
        assert_eq!(h.m.active_workspace.as_deref(), Some(first_ws.as_str()));
        assert_eq!(h.m.viewport.x, 500.);
        assert_eq!(h.focused().id, first_card);
        // ctrl+2 goes straight to the second.
        h.run("workspace.show.2");
        assert_eq!(h.m.active_workspace.as_deref(), Some(second_ws.as_str()));
        // A tenth that does not exist does nothing.
        h.run("workspace.show.9");
        assert_eq!(h.m.active_workspace.as_deref(), Some(second_ws.as_str()));
    }

    #[test]
    fn the_last_workspace_stays_and_closing_one_asks_and_kills_its_shells() {
        let mut h = Harness::new();
        h.run("workspace.close");
        assert_eq!(h.m.notice.as_deref(), Some("the last workspace stays"));
        assert_eq!(h.m.workspaces.len(), 1);
        h.run("workspace.new");
        let doomed = h.focused().id.clone();
        h.m.card_mut(&doomed).unwrap().pane_id = Some(42);
        h.run("workspace.close");
        assert!(h.m.prompt.confirm);
        let (pending, text) = h.m.prompt.settle(Some("")).unwrap();
        h.m.answer(pending, text, |_| true);
        let effects = h.m.take_effects();
        assert!(effects.contains(&Effect::KillPane(42)));
        assert_eq!(h.m.workspaces.len(), 1);
        assert!(h.m.card(&doomed).is_none());
        assert!(
            h.focused().pane_id.is_none(),
            "back on the first workspace's card"
        );
    }

    // A single tab is exactly today's close: no dialog, because there is
    // nothing extra to lose. Several tabs on one card is several pages at
    // once, the same reasoning `workspace.close` already applies to several
    // shells; a cancel leaves every tab open, a confirm takes them all.
    #[test]
    fn closing_a_browser_card_with_several_tabs_asks_first() {
        let mut h = Harness::new();
        h.m.new_beside_active(CardKind::Browser, None, Some("https://a".into()));
        let single = h.focused().id.clone();
        h.run("card.close");
        assert!(h.m.card(&single).is_none(), "one tab closes right away");

        h.m.new_beside_active(CardKind::Browser, None, Some("https://a".into()));
        let id = h.focused().id.clone();
        h.m.card_mut(&id).unwrap().tabs = vec!["https://a".into(), "https://b".into()];
        h.run("card.close");
        assert!(h.m.card(&id).is_some(), "waiting on the confirm");
        assert!(h.m.prompt.confirm);

        let (pending, text) = h.m.prompt.settle(None).unwrap();
        h.m.answer(pending, text, |_| true);
        assert!(h.m.card(&id).is_some(), "a cancel keeps both tabs");

        h.run("card.close");
        let (pending, text) = h.m.prompt.settle(Some("")).unwrap();
        h.m.answer(pending, text, |_| true);
        assert!(h.m.card(&id).is_none());
    }

    // An arrow into an empty slot shows a PHANTOM; Enter asks what goes in it.
    #[test]
    fn an_arrow_into_empty_space_shows_a_phantom_and_enter_fills_it() {
        let mut h = Harness::new();
        let first = h.focused().rect;
        h.run("focus.move.right");
        assert_eq!(h.m.selection.focused_id, None);
        let phantom = h.m.selection.phantom.clone().expect("a phantom");
        assert!(phantom.rect.x > first.x);
        assert!(h.m.handle_bare_key(focus_cmd::BareKey::Enter));
        assert_eq!(h.m.palette.source, Some(Source::SlotKind));
        h.m.close_palette(true);
        h.m.palette_run(Source::SlotKind, "terminal");
        assert_eq!(h.m.cards.len(), 2);
        assert_eq!(h.focused().rect, phantom.rect);
        assert!(h.m.selection.phantom.is_none());
    }

    // "Browser" in the phantom's kind picker used to open the old text
    // prompt; it opens the omnibox now, and Enter has to land the result in
    // the SAME phantom rect the picker was opened from, not beside whatever
    // was active before the arrow key ever moved focus off it.
    #[test]
    fn choosing_browser_in_the_phantom_kind_picker_opens_the_omnibox_and_lands_in_the_phantom() {
        let mut h = Harness::new();
        h.run("focus.move.right");
        let phantom = h.m.selection.phantom.clone().expect("a phantom");
        h.m.handle_bare_key(focus_cmd::BareKey::Enter);
        h.m.close_palette(true);
        h.m.palette_run(Source::SlotKind, "browser");
        assert!(h.m.omni.open);
        assert!(h.m.omni.phantom);
        assert!(h.m.omni.target.is_none());
        h.m.omni_type("example.com");
        h.m.omni_enter();
        assert!(!h.m.omni.open);
        assert_eq!(h.m.cards.len(), 2);
        assert!(h.m.selection.phantom.is_none());
        let made = h.focused();
        assert_eq!(made.kind, CardKind::Browser);
        assert_eq!(made.url.as_deref(), Some("https://example.com"));
        assert_eq!(made.rect, phantom.rect);
    }

    #[test]
    fn escape_on_a_phantom_returns_to_the_nearest_card() {
        let mut h = Harness::new();
        let first = h.focused().id.clone();
        h.run("focus.move.right");
        assert!(h.m.handle_bare_key(focus_cmd::BareKey::Escape));
        assert_eq!(h.focused().id, first);
        assert!(h.m.selection.phantom.is_none());
        // Outside any mode, a bare key belongs to the shell.
        assert!(!h.m.handle_bare_key(focus_cmd::BareKey::Escape));
        assert!(!h.m.handle_bare_key(focus_cmd::BareKey::Char('a')));
    }

    // Hint mode: a letter on every card, the next bare key is a hint.
    #[test]
    fn hints_letter_the_cards_and_a_letter_focuses_one() {
        let mut h = Harness::new();
        let a = h.focused().id.clone();
        h.run("card.new.terminal");
        let b = h.focused().id.clone();
        h.run("focus.hint");
        assert_eq!(h.m.selection.hints.len(), 2);
        let key_a = h.m.selection.hints[&a];
        h.m.take_effects();
        assert!(h.m.handle_bare_key(focus_cmd::BareKey::Char(key_a)));
        assert_eq!(h.focused().id, a);
        assert!(h.m.selection.hints.is_empty());
        assert!(h.m.framing, "the jump fits the card, as Cmd+1 does");
        let _ = b;
    }

    // Shift+direction selects more; a plain move collapses; Escape collapses.
    #[test]
    fn extending_grows_the_selection_and_a_plain_move_empties_it() {
        let mut h = Harness::new();
        let a = h.focused().id.clone();
        h.run("card.new.terminal");
        let b = h.focused().id.clone();
        h.m.set_focus(Some(&a));
        h.run("focus.extend.right");
        assert_eq!(h.focused().id, b);
        assert_eq!(h.m.selection.extra, vec![a.clone()]);
        assert_eq!(h.m.selected_ids().len(), 2);
        h.run("focus.move.left");
        assert!(h.m.selection.extra.is_empty());
        h.run("focus.extend.right");
        assert!(h.m.handle_bare_key(focus_cmd::BareKey::Escape));
        assert!(h.m.selection.extra.is_empty());
    }

    // Several cards selected: Cmd+Alt+Shift+Arrow moves the selection as one
    // block, a block over, trading places with the cards there (#238).
    #[test]
    fn a_selection_moves_as_one_block_and_swaps_with_what_is_there() {
        let mut h = Harness::new();
        let a = h.focused().id.clone();
        h.run("card.new.terminal");
        let b = h.focused().id.clone();
        h.run("card.new.terminal");
        let c = h.focused().id.clone();
        let rects = |h: &Harness| -> Vec<crate::grid::Rect> {
            [&a, &b, &c]
                .iter()
                .map(|id| h.m.card(id).unwrap().rect)
                .collect()
        };
        let before = rects(&h);
        // a and b selected, b focused
        h.m.focus_extended(&b, vec![a.clone()]);
        assert_eq!(h.m.selected_ids().len(), 2);
        let effects = h.run("card.swap.right");
        let after = rects(&h);
        assert_ne!(after, before, "the selection moved");
        // the pair moved together: the gap between a and b is unchanged
        assert_eq!(after[1].x - after[0].x, before[1].x - before[0].x);
        assert_eq!(after[1].y - after[0].y, before[1].y - before[0].y);
        assert!(effects.iter().any(|e| matches!(e, Effect::MarkSwap(_))));
        // nothing overlaps, focus stayed, the selection is still two cards
        for (i, x) in after.iter().enumerate() {
            for y in &after[i + 1..] {
                assert!(!crate::layout::rects_overlap(*x, *y), "{x:?} over {y:?}");
            }
        }
        assert_eq!(h.focused().id, b);
        assert_eq!(h.m.selected_ids().len(), 2);
        // and it undoes in one step
        h.run("layout.undo");
        assert_eq!(rects(&h), before);
    }

    // The tab row's order (#242): a drag does it with the mouse, these keep it
    // from being mouse-only. The workspace that moved stays the active one.
    #[test]
    fn workspace_tabs_reorder_and_the_moved_one_stays_active() {
        let mut h = Harness::new();
        h.run("workspace.new");
        h.run("workspace.new");
        let ids: Vec<String> = h.m.workspaces.iter().map(|w| w.id.clone()).collect();
        let order =
            |h: &Harness| -> Vec<String> { h.m.workspaces.iter().map(|w| w.id.clone()).collect() };
        let last = ids[2].clone();
        assert_eq!(h.m.active_workspace.as_deref(), Some(last.as_str()));
        h.run("workspace.reorder.left");
        assert_eq!(
            order(&h),
            vec![ids[0].clone(), last.clone(), ids[1].clone()]
        );
        h.run("workspace.reorder.left");
        h.run("workspace.reorder.left");
        assert_eq!(
            order(&h),
            vec![last.clone(), ids[0].clone(), ids[1].clone()],
            "stops at the left end"
        );
        assert_eq!(h.m.active_workspace.as_deref(), Some(last.as_str()));
        h.run("workspace.reorder.right");
        assert_eq!(order(&h)[1], last);
        // what a drop does: the tab takes the index of the tab it lands on
        h.m.move_workspace(&last, 2);
        assert_eq!(
            order(&h),
            vec![ids[0].clone(), ids[1].clone(), last.clone()]
        );
        assert!(h.m.dirty_layout, "the order is saved");
    }

    // A swap trades whole rects; focus stays on the card that moved.
    #[test]
    fn a_swap_trades_rects_and_animates_both() {
        let mut h = Harness::new();
        let a = h.focused().id.clone();
        h.run("card.new.terminal");
        let b = h.focused().id.clone();
        let (ra, rb) = (h.m.card(&a).unwrap().rect, h.m.card(&b).unwrap().rect);
        h.m.set_focus(Some(&a));
        let effects = h.run("card.swap.right");
        assert_eq!(h.m.card(&a).unwrap().rect, rb);
        assert_eq!(h.m.card(&b).unwrap().rect, ra);
        assert_eq!(h.focused().id, a);
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::MarkSwap(ids) if ids.len() == 2 && ids[0].1 == ra)));
    }

    // Any canvas-level zoom drops out of maximise.
    #[test]
    fn maximise_toggles_and_a_fit_leaves_it() {
        let mut h = Harness::new();
        h.run("card.maximize.toggle");
        assert!(h.m.selection.maximized);
        let effects = h.run("canvas.zoom.actual");
        assert!(!h.m.selection.maximized);
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::AnimateFit(v) if v.scale == 1.)));
    }

    // Cmd+3 with several cards selected fits exactly those cards; with one
    // it fits that card's block.
    #[test]
    fn fit_group_fits_a_selection_of_cards() {
        let mut h = Harness::new();
        let ids = four_cards(&mut h);
        h.m.set_focus(Some(&ids[0]));
        h.m.extend_to(&ids[3]);
        let rects: Vec<_> = [&ids[0], &ids[3]]
            .iter()
            .map(|id| h.m.card(id).unwrap().rect)
            .collect();
        let want = crate::viewport::fit_rect(
            crate::viewport::bounding_rect(&rects).unwrap(),
            h.m.view_size,
        );
        let effects = h.run("canvas.zoom.fitGroup");
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::AnimateFit(v) if *v == want)));
    }

    // A closed terminal is watched, not timed: kept while it works, ended
    // after PARK_MS of quiet, kept while its agent waits for you (Alt+T
    // brings that one back), and a reopen takes the running program back.
    #[test]
    fn a_closed_terminal_runs_until_quiet_and_a_reopen_takes_it_back() {
        use crate::agent_state::AgentState::{Waiting, Working};
        use crate::backend::PaneEvent;
        use crate::model::lifecycle::PARK_MS;
        let mut h = Harness::new();
        h.m.can_park = true;
        let ids = four_cards(&mut h);
        for (i, id) in ids.iter().enumerate() {
            let c = h.m.card_mut(id).unwrap();
            c.pane_id = Some(100 + i as u32);
            c.session = Some(format!("s{i}"));
        }
        let kills = |h: &mut Harness| -> Vec<u32> {
            h.m.take_effects()
                .into_iter()
                .filter_map(|e| match e {
                    Effect::KillPane(p) => Some(p),
                    _ => None,
                })
                .collect()
        };
        // A build: output keeps it alive past the minute, quiet ends it.
        h.m.set_focus(Some(&ids[0]));
        let effects = h.run("card.close");
        assert!(!effects.iter().any(|e| matches!(e, Effect::KillPane(_))));
        let t = h.m.now_ms;
        h.m.tick(t + PARK_MS - 1.);
        h.m.apply_pane_event(100, &PaneEvent::Output(b"compiling".to_vec()));
        h.m.tick(t + PARK_MS + 1.);
        assert!(kills(&mut h).is_empty(), "still printing, still running");
        let later = h.m.now_ms;
        h.m.tick(later + PARK_MS);
        assert_eq!(kills(&mut h), vec![100], "a quiet minute ends it");
        assert!(h.m.parked.is_empty());

        // An agent waiting for you is kept however long, and Alt+T
        // reopens it with the state it has now.
        h.m.set_focus(Some(&ids[1]));
        h.m.card_mut(&ids[1]).unwrap().agent = Working;
        h.run("card.close");
        h.m.parked[0].card.agent = Waiting;
        let t = h.m.now_ms;
        h.m.tick(t + 10. * PARK_MS);
        assert!(kills(&mut h).is_empty());
        assert_eq!(
            h.m.waiting_parked().map(|c| c.id.clone()),
            Some(ids[1].clone())
        );
        // Another card closed since does not change what Alt+T brings.
        h.m.set_focus(Some(&ids[2]));
        h.run("card.close");
        assert!(h.m.reopen_waiting());
        let effects = h.m.take_effects();
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::ReleasePane(101))));
        assert_eq!(h.m.card(&ids[1]).unwrap().agent, Waiting);
        assert!(h.m.waiting_parked().is_none());
        // Cmd+Shift+T is still the card closed last.
        h.run("card.reopen");
        assert!(h.m.card(&ids[2]).is_some());
        // Undoing the waiting card's close now finds it back already.
        let before = h.m.cards.len();
        h.m.reopen_card(h.m.card(&ids[1]).unwrap().clone());
        assert_eq!(h.m.cards.len(), before, "no twin");

        // Cmd+Shift+T reopens, as in a browser; the placement menu moved.
        let chord = |c: &str| {
            h.m.keymap
                .iter()
                .find(|(k, _)| k == c)
                .map(|(_, id)| id.clone())
        };
        assert_eq!(chord("cmd+shift+t").as_deref(), Some("card.reopen"));
        assert_eq!(chord("cmd+ctrl+t").as_deref(), Some("card.place"));
    }

    // A rectangle selects the cards it touches, the nearest to where it
    // began leading; Shift keeps what was selected; Cmd+click toggles.
    #[test]
    fn a_marquee_selects_what_it_touches_and_cmd_click_toggles() {
        use crate::grid::{Point, Rect};
        let mut h = Harness::new();
        let ids = four_cards(&mut h);
        let r: Vec<Rect> = ids.iter().map(|id| h.m.card(id).unwrap().rect).collect();
        // From just outside card 0's top-left, across cards 0 and 1 only.
        let start = Point {
            x: r[0].x - 5.,
            y: r[0].y - 5.,
        };
        let span = Rect {
            x: start.x,
            y: start.y,
            w: (r[1].x + 10.) - start.x,
            h: 10.,
        };
        h.m.marquee_select(span, start, None);
        h.m.marquee_done();
        let mut got = h.m.selected_ids();
        got.sort();
        let mut want = vec![ids[0].clone(), ids[1].clone()];
        want.sort();
        assert_eq!(got, want);
        assert_eq!(
            h.m.selection.focused_id.as_deref(),
            Some(ids[0].as_str()),
            "nearest leads"
        );
        // Shift+drag over card 2 alone adds it.
        let kept = h.m.selected_ids();
        let c2 = Rect {
            x: r[2].x + 1.,
            y: r[2].y + 1.,
            w: 5.,
            h: 5.,
        };
        h.m.marquee_select(c2, Point { x: c2.x, y: c2.y }, Some(&kept));
        assert_eq!(h.m.selected_ids().len(), 3);
        // A drag over nothing, without Shift, selects nothing.
        let far = Rect {
            x: -1e6,
            y: -1e6,
            w: 5.,
            h: 5.,
        };
        h.m.marquee_select(far, Point { x: far.x, y: far.y }, None);
        assert!(h.m.selected_ids().is_empty());
        // Cmd+click: adds a card, takes a selected one out, and a lone
        // focused card is left to its body.
        h.m.set_focus(Some(&ids[0]));
        assert!(
            !h.m.toggle_selected(&ids[0]),
            "a link click on the card you are in"
        );
        assert!(h.m.toggle_selected(&ids[3]));
        assert_eq!(h.m.selected_ids().len(), 2);
        assert!(h.m.toggle_selected(&ids[0]));
        assert_eq!(h.m.selected_ids(), vec![ids[3].clone()]);
    }

    // A selection dragged together lands when the space is free, even when
    // its cards belong to different groups; onto another card it goes back.
    #[test]
    fn a_selection_across_groups_moves_as_one() {
        let mut h = Harness::new();
        let ids = four_cards(&mut h);
        let ga = h.m.add_group("a");
        let gb = h.m.add_group("b");
        h.m.card_mut(&ids[0]).unwrap().group_id = Some(ga);
        h.m.card_mut(&ids[1]).unwrap().group_id = Some(gb);
        let moved = [ids[0].clone(), ids[1].clone()];
        let start: Vec<_> = moved
            .iter()
            .map(|id| (id.clone(), h.m.card(id).unwrap().rect))
            .collect();
        let far = 100_000.;
        for id in &moved {
            h.m.card_mut(id).unwrap().rect.y += far;
        }
        assert!(h.m.end_gesture(&moved, &start), "free space: it lands");
        assert_eq!(h.m.card(&ids[0]).unwrap().rect.y, start[0].1.y + far);
        // Onto card 2: back where it started.
        let back: Vec<_> = moved
            .iter()
            .map(|id| (id.clone(), h.m.card(id).unwrap().rect))
            .collect();
        let target = h.m.card(&ids[2]).unwrap().rect;
        h.m.card_mut(&ids[0]).unwrap().rect = target;
        assert!(!h.m.end_gesture(&moved, &back));
        assert_eq!(h.m.card(&ids[0]).unwrap().rect, back[0].1);
    }

    // #91: opening the settings focuses the user's file alone; the defaults
    // card beside it is not part of a selection.
    #[test]
    fn opening_settings_selects_only_the_users_file() {
        let mut h = Harness::new();
        four_cards(&mut h);
        h.run("app.settings");
        let focused = h.m.focused().cloned().expect("a focused card");
        assert!(focused
            .path
            .as_deref()
            .unwrap_or("")
            .ends_with("settings.json"));
        assert_eq!(h.m.selected().len(), 1, "one card selected");
        assert_eq!(h.m.config_pairs.len(), 1, "the pair is still remembered");
    }

    // "Open a file" asks for the macOS panel, and what it picks opens as
    // `ift <path>` would: a file in an editor, a folder with its tree (#64).
    #[test]
    fn open_a_file_shows_the_panel_and_opens_the_pick() {
        let mut h = Harness::new();
        let effects = h.run("card.open.file");
        assert!(effects.iter().any(|e| matches!(e, Effect::PickFile { .. })));
        let dir = std::env::temp_dir().join(format!("ift-pick-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.md");
        std::fs::write(&file, "x").unwrap();
        let from = h.focused().id.clone();
        let id =
            h.m.open_picked(&file.to_string_lossy(), Some(&from))
                .unwrap();
        let card = h.m.card(&id).unwrap();
        assert_eq!(card.kind, CardKind::Editor);
        assert_eq!(card.path.as_deref(), Some(file.to_string_lossy().as_ref()));
        let id =
            h.m.open_picked(&dir.to_string_lossy(), Some(&from))
                .unwrap();
        assert!(
            h.m.card(&id).unwrap().explorer,
            "a folder opens with its tree"
        );
    }

    // A dialog owns the keyboard: the group-name prompt is modal, the
    // switcher is not (it runs on Ctrl held down).
    #[test]
    fn a_dialog_is_modal_and_the_switcher_is_not() {
        let mut h = Harness::new();
        let ids = four_cards(&mut h);
        h.m.set_focus(Some(&ids[0]));
        h.m.extend_to(&ids[1]);
        assert!(!h.m.modal_open());
        h.run("group.new");
        assert!(h.m.prompt.is_open(), "Cmd+G asks for the name");
        assert!(h.m.modal_open());
    }

    // Shift+click: the card clicked takes the focus, the one it left stays
    // selected, and a second Shift+click on a member takes it out.
    #[test]
    fn shift_click_builds_a_selection_and_takes_cards_out() {
        let mut h = Harness::new();
        let ids = four_cards(&mut h);
        h.m.set_focus(Some(&ids[0]));
        h.m.extend_to(&ids[1]);
        h.m.extend_to(&ids[2]);
        assert_eq!(h.m.selection.focused_id.as_deref(), Some(ids[2].as_str()));
        assert_eq!(h.m.selection.extra, vec![ids[0].clone(), ids[1].clone()]);
        h.m.tick(h.m.now_ms + 5_000.);
        assert_eq!(h.m.selection.extra.len(), 2, "nothing clears it on its own");
        h.m.extend_to(&ids[0]);
        assert_eq!(h.m.selection.extra, vec![ids[1].clone()]);
    }

    // Cmd+= / Cmd+- size the terminal font, one point a press, saved, and
    // stop at the ends; the canvas does not zoom.
    #[test]
    fn the_font_keys_size_the_terminal_font_and_save_it() {
        let mut h = Harness::new();
        h.m.config.terminal.font_size = 14.;
        let effects = h.run("terminal.font.bigger");
        assert_eq!(h.m.config.terminal.font_size, 15.);
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::SaveSetting { path, value } if path == "terminal.fontSize" && value == 15.
        )));
        assert!(!effects
            .iter()
            .any(|e| matches!(e, Effect::AnimateZoom { .. })));
        h.run("terminal.font.smaller");
        h.run("terminal.font.smaller");
        assert_eq!(h.m.config.terminal.font_size, 13.);
        h.m.config.terminal.font_size = 6.;
        let effects = h.run("terminal.font.smaller");
        assert_eq!(h.m.config.terminal.font_size, 6.);
        assert!(!effects
            .iter()
            .any(|e| matches!(e, Effect::SaveSetting { .. })));
        assert_eq!(
            h.m.keymap
                .iter()
                .find(|(c, _)| c == "cmd+=")
                .map(|(_, id)| id.as_str()),
            Some("terminal.font.bigger")
        );
    }

    // A held zoom key chains from the pending target, not the passing scale.
    #[test]
    fn zoom_chains_from_the_pending_scale() {
        let mut h = Harness::new();
        let effects = h.run("canvas.zoom.in");
        let Some(Effect::AnimateZoom { scale, .. }) = effects
            .iter()
            .find(|e| matches!(e, Effect::AnimateZoom { .. }))
        else {
            panic!()
        };
        assert!((scale - 1.2).abs() < 1e-9);
        h.m.pending_scale = Some(*scale);
        let effects = h.run("canvas.zoom.in");
        let Some(Effect::AnimateZoom { scale, .. }) = effects
            .iter()
            .find(|e| matches!(e, Effect::AnimateZoom { .. }))
        else {
            panic!()
        };
        assert!((scale - 1.44).abs() < 1e-9);
    }

    // Fit-card keeps framing, so the next arrow re-fits instead of nudging.
    #[test]
    fn fit_card_frames_and_the_next_focus_move_refits() {
        let mut h = Harness::new();
        h.run("card.new.terminal");
        h.run("canvas.zoom.fitCard");
        assert!(h.m.framing);
        let effects = h.run("focus.move.left");
        assert!(effects.iter().any(|e| matches!(e, Effect::AnimateFit(_))));
        assert!(h.m.framing);
    }

    // Cmd+3 on a loose card frames the block of cards around it (here a
    // split's pieces and the card beside them), not only a group.
    #[test]
    fn fit_group_frames_a_loose_cards_cluster() {
        let mut h = Harness::new();
        h.run("card.new.terminal");
        h.run("card.split.right");
        let rects: Vec<_> = h.m.cards.iter().map(|c| c.rect).collect();
        assert_eq!(rects.len(), 3);
        let want = crate::viewport::bounding_rect(&rects).unwrap();
        let effects = h.run("canvas.zoom.fitGroup");
        let Some(Effect::AnimateFit(v)) =
            effects.iter().find(|e| matches!(e, Effect::AnimateFit(_)))
        else {
            panic!("no fit")
        };
        assert_eq!(*v, crate::viewport::fit_rect(want, h.m.view_size));
    }

    // #87: `ui.fitPadding` is the margin around a fit and `ui.fitMagnify`
    // lets it pass 100%; the defaults are the old fixed 48 px and the cap.
    #[test]
    fn fit_settings_set_the_margin_and_the_ceiling() {
        use crate::viewport::{fit_rect_with, FIT_PADDING, MAX_FIT_SCALE, MAX_SCALE};
        let mut h = Harness::new();
        let ids = four_cards(&mut h);
        let rect = h.m.card(&ids[0]).unwrap().rect;
        let size = h.m.view_size;
        assert_eq!(
            h.m.fit_viewport(rect),
            fit_rect_with(rect, size, FIT_PADDING, MAX_FIT_SCALE),
            "defaults are the old fit"
        );
        h.m.apply_settings_text(r#"{"ui.fitPadding": 0, "ui.fitMagnify": true}"#);
        assert_eq!(
            h.m.fit_viewport(rect),
            fit_rect_with(rect, size, 0., MAX_SCALE)
        );
        // A card smaller than the window grows past 100% only when asked.
        let small = crate::grid::Rect {
            x: 0.,
            y: 0.,
            w: 100.,
            h: 100.,
        };
        assert!(h.m.fit_viewport(small).scale > 1.);
        h.m.apply_settings_text(r#"{"ui.fitPadding": 0}"#);
        assert_eq!(h.m.fit_viewport(small).scale, 1.);
    }

    // `ui.fitSplitSlot`: Cmd+1 on a split half frames both halves by default;
    // off, it fits the half itself.
    #[test]
    fn fit_split_slot_off_fits_the_half_alone() {
        let mut h = Harness::new();
        h.run("card.split.right");
        let half = h.m.focused().unwrap().rect;
        let slot = {
            let rects: Vec<_> = h.m.cards.iter().map(|c| c.rect).collect();
            crate::viewport::bounding_rect(&rects).unwrap()
        };
        let selected = h.m.selected();
        let rects: Vec<_> = selected.iter().map(|c| c.rect).collect();
        let groups: Vec<_> = selected.iter().map(|c| c.soft_group_id.clone()).collect();
        assert_eq!(h.m.slot_bounds(&rects, &groups), Some(slot));
        h.m.apply_settings_text(r#"{"ui.fitSplitSlot": false}"#);
        assert_eq!(h.m.slot_bounds(&rects, &groups), Some(half));
    }

    // Cmd+Ctrl+Enter on an empty slot (the phantom) grows it into the free
    // room, so a slot beside a quarter can become a card of any size up to the
    // default. A slot with nothing to grow into says so.
    #[test]
    fn fill_the_free_space_grows_the_phantom() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        let default = h.m.default_size();
        let quarter = h.m.card(&id).unwrap().rect;
        let small = crate::grid::Rect {
            w: default.w / 2.,
            h: default.h / 2.,
            ..quarter
        };
        h.m.card_mut(&id).unwrap().rect = small;
        h.run("focus.move.right");
        let phantom = h.m.selection.phantom.clone().expect("a slot");
        assert_eq!((phantom.rect.w, phantom.rect.h), (small.w, small.h));
        h.run("card.size.reset");
        let grown = h.m.selection.phantom.clone().expect("still a slot").rect;
        assert!(grown.w > small.w || grown.h > small.h, "{grown:?}");
        assert!(grown.w <= default.w && grown.h <= default.h);
        assert!(h.m.focused().is_none(), "no card was made or focused");
    }

    // `cards.gap` (#232) is the space every placement leaves: a split, a new
    // card, a tidy and a group frame all take it, where the constant used to
    // be 25. The nearest two cards are exactly one gap apart and none is closer.
    fn nearest_gap(h: &Harness) -> (f64, f64) {
        let rects: Vec<crate::grid::Rect> = h.m.here().iter().map(|c| c.rect).collect();
        let mut nearest = f64::MAX;
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                let dx = (b.x - (a.x + a.w)).max(a.x - (b.x + b.w));
                let dy = (b.y - (a.y + a.h)).max(a.y - (b.y + b.h));
                // beside each other (they overlap on the other axis), else a corner
                let d = if dy < 0. {
                    dx
                } else if dx < 0. {
                    dy
                } else {
                    f64::MAX
                };
                nearest = nearest.min(d);
            }
        }
        let overlapping = rects.iter().enumerate().any(|(i, a)| {
            rects[i + 1..]
                .iter()
                .any(|b| crate::layout::rects_overlap(*a, *b))
        });
        (nearest, if overlapping { 1. } else { 0. })
    }

    #[test]
    fn the_gap_setting_spaces_splits_new_cards_and_group_frames() {
        for gap in [0., 10., 60.] {
            let mut h = Harness::new();
            h.m.apply_settings_text(&format!("{{\"cards.gap\": {gap}}}"));
            assert_eq!(h.m.gap(), gap);
            assert_eq!(h.m.group_pad(), gap / 2.);
            h.run("card.split.right");
            h.run("card.new.terminal");
            h.run("card.new.terminal");
            h.run("card.new.terminal");
            let (nearest, overlapping) = nearest_gap(&h);
            assert_eq!(overlapping, 0., "gap {gap}: cards overlap");
            assert_eq!(
                nearest, gap,
                "gap {gap}: the nearest cards are one gap apart"
            );
            // Canvas: tidy puts them back in the block, still a gap apart.
            h.run("canvas.tidy");
            let (nearest, overlapping) = nearest_gap(&h);
            assert_eq!(overlapping, 0., "gap {gap}: tidy overlaps");
            assert_eq!(nearest, gap, "gap {gap}: tidy keeps one gap");
        }
    }

    #[test]
    fn the_gap_defaults_to_the_grid_and_is_clamped() {
        let mut h = Harness::new();
        assert_eq!(h.m.gap(), crate::cards::GUTTER);
        h.m.apply_settings_text(r#"{"cards.gap": 900}"#);
        assert_eq!(h.m.gap(), 200.);
        h.m.apply_settings_text(r#"{"cards.gap": -5}"#);
        assert_eq!(h.m.gap(), 0.);
    }

    fn edit_from(h: &mut Harness, card: Option<&str>, path: &str) -> crate::cli::CliReply {
        h.m.run_ift(
            &crate::cli::CliRequest {
                id: 77,
                cmd: "edit".into(),
                args: vec![path.into()],
                card_id: card.map(String::from),
            },
            &[],
        )
    }

    // `ift ~/.zshrc` in a terminal: the file opens over that card, same
    // rect, locked and focused, the request left for later; the terminal
    // drops out of what you can navigate to; closing answers the waiting
    // `ift` and hands the focus back, with nothing for Cmd+Z to bring back.
    #[test]
    fn edit_opens_in_place_and_closing_answers_the_waiting_ift() {
        let mut h = Harness::new();
        let term = h.focused().id.clone();
        let rect = h.focused().rect;
        let reply = edit_from(&mut h, Some(&term), "/Users/me/.zshrc");
        assert!(reply.ok);
        assert!(h.m.reply_deferred, "answered when it closes, not now");
        let cover = h.focused().clone();
        assert_ne!(cover.id, term);
        assert_eq!(
            (cover.kind, cover.rect, cover.locked),
            (CardKind::Editor, rect, true)
        );
        assert_eq!(cover.path.as_deref(), Some("/Users/me/.zshrc"));
        assert!(
            h.m.here().iter().all(|c| c.id != term),
            "the terminal is under it"
        );
        assert!(!h.m.save_text().unwrap().contains(&cover.id), "not saved");
        h.m.take_effects();
        let effects = h.run("card.close");
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::CliReply {
                id: 77,
                ok: true,
                ..
            }
        )));
        assert_eq!(h.focused().id, term, "back to the shell");
        assert!(h.m.card(&cover.id).is_none());
        assert!(h.m.covers.is_empty() && h.m.edit_waiters.is_empty());
        assert!(h.m.here().iter().any(|c| c.id == term));
        assert!(
            h.m.closed.iter().all(|c| c.id != cover.id),
            "no Cmd+Ctrl+T twin"
        );
    }

    // The pair is one card on screen: moving the cover takes the terminal
    // with it, and so does moving the terminal (a swap, an undo).
    #[test]
    fn a_cover_and_its_terminal_move_together() {
        let mut h = Harness::new();
        let term = h.focused().id.clone();
        edit_from(&mut h, Some(&term), "/tmp/x.txt");
        let cover = h.focused().id.clone();
        let mut to = h.focused().rect;
        to.x += 5000.;
        assert!(
            h.m.drop_card(&cover, to),
            "its own terminal is not in the way"
        );
        h.m.tick(h.m.now_ms + 16.);
        assert_eq!(
            h.m.card(&term).unwrap().rect,
            h.m.card(&cover).unwrap().rect
        );
        let mut back = to;
        back.y += 3000.;
        h.m.card_mut(&term).unwrap().rect = back;
        h.m.tick(h.m.now_ms + 16.);
        assert_eq!(h.m.card(&cover).unwrap().rect, back);
    }

    // No terminal to cover (run outside a card, or from an editor): an
    // ordinary card, answered at once.
    #[test]
    fn edit_without_a_terminal_opens_a_card_and_answers_now() {
        let mut h = Harness::new();
        let reply = edit_from(&mut h, None, "/tmp/x.txt");
        assert!(reply.ok && !h.m.reply_deferred);
        assert!(h.m.covers.is_empty());
        assert_eq!(h.focused().kind, CardKind::Editor);
        assert_ne!(h.focused().rect, h.m.cards[0].rect);
    }

    // A Claude card keeps its session's name across a relaunch, with no
    // hook arriving to say an agent is there: the saved session id is the
    // evidence and the saved title the name. A shell's title never is.
    #[test]
    fn a_claude_card_keeps_its_name_across_a_relaunch() {
        let mut h = Harness::new();
        h.run("card.new.terminal");
        h.m.cards[0].agent_session = Some("c0ffee".into());
        h.m.cards[0].osc_title = Some("fix the tab dots".into());
        h.m.cards[1].osc_title = Some("zsh: ~/Code".into());
        let text = h.m.save_text().expect("saveable");
        let mut again = Model::new();
        again.home = "/Users/me".into();
        again.load_layout(Some(&text));
        assert!(again
            .label_of(&again.cards[0].clone())
            .contains("fix the tab dots"));
        assert!(!again.label_of(&again.cards[1].clone()).contains("zsh"));
    }

    #[test]
    fn the_layout_round_trips_through_save_and_load() {
        let mut h = Harness::new();
        h.run("card.new.terminal");
        h.run("card.rename");
        let (pending, text) = h.m.prompt.settle(Some("deploy")).unwrap();
        h.m.answer(pending, text, |_| true);
        let text = h.m.save_text().expect("saveable");
        let mut again = Model::new();
        again.view_size = Size { w: 1600., h: 1000. };
        again.load_layout(Some(&text));
        assert_eq!(again.cards.len(), 2);
        assert_eq!(again.cards[1].title, "deploy");
        assert_eq!(again.workspaces, h.m.workspaces);
        assert_eq!(again.selection.focused_id, h.m.selection.focused_id);
        assert!(again.loaded && !again.read_only);
        assert_eq!(again.save_text().unwrap(), text);
        // Numbers come back as they were, and the next card takes the
        // lowest free one; a file with none (before the field) gets the
        // lowest free ones in file order.
        assert_eq!(
            again.cards.iter().map(|c| c.number).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(again.take_number(), 3);
        let mixed = text.replacen("\"number\": 1", "\"number\": 9", 1).replacen(
            "\"number\": 2",
            "\"nope\": 2",
            1,
        );
        let mut old = Model::new();
        old.view_size = Size { w: 1600., h: 1000. };
        old.load_layout(Some(&mixed));
        assert_eq!(
            old.cards.iter().map(|c| c.number).collect::<Vec<_>>(),
            [9, 1]
        );
        assert_eq!(old.take_number(), 2);
    }

    #[test]
    fn nothing_is_saved_before_loading_or_over_a_newer_file() {
        let mut m = Model::new();
        assert_eq!(m.save_text(), None);
        m.load_layout(Some(r#"{"version": 99, "cards": []}"#));
        assert!(m.read_only);
        assert_eq!(m.save_text(), None);
        assert!(m.notice.is_some());
    }

    #[test]
    fn ift_lists_names_and_groups() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        let req = |cmd: &str, args: &[&str], card: Option<&str>| crate::cli::CliRequest {
            id: 1,
            cmd: cmd.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            card_id: card.map(String::from),
        };
        let ls = h.m.run_ift(&req("ls", &[], None), &[]);
        assert!(ls.ok);
        assert!(ls.text.starts_with(&format!("{id}\t-\t~\tnone\t-")));
        assert!(!h.m.run_ift(&req("name", &["x"], None), &[]).ok);
        assert!(
            h.m.run_ift(&req("name", &["deploy", "box"], Some(&id)), &[])
                .ok
        );
        assert_eq!(h.focused().title, "deploy box");
        let g = h.m.run_ift(&req("group", &["api"], Some(&id)), &[]);
        assert!(g.ok);
        assert_eq!(h.m.groups[0].name, "api");
        // Joining the same name joins, never duplicates.
        h.run("card.new.ungrouped");
        let other = h.focused().id.clone();
        let g2 = h.m.run_ift(&req("group", &["api"], Some(&other)), &[]);
        assert_eq!(g2.text, g.text);
        assert_eq!(h.m.groups.len(), 1);
        assert!(!h.m.run_ift(&req("nope", &[], None), &[]).ok);
    }

    // Hook reports are authoritative; a missed Stop is caught by the sweep.
    #[test]
    fn hooks_set_agent_state_and_the_sweep_catches_a_missed_stop() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        let report = |event: &str| crate::hooks::HookReport {
            card_id: id.clone(),
            event: event.into(),
            tool: None,
            transcript: Some("/s/x.jsonl".into()),
            session: Some("sess-1".into()),
            agent: None,
        };
        h.m.apply_hook(&report("PreToolUse"));
        assert_eq!(h.focused().agent, crate::agent_state::AgentState::Working);
        assert_eq!(h.focused().transcript_path.as_deref(), Some("/s/x.jsonl"));
        // The session id is kept for `claude --resume` after a reboot, and
        // its arrival is a layout change so it reaches the save file.
        assert_eq!(h.focused().agent_session.as_deref(), Some("sess-1"));
        assert!(h.m.dirty_layout);
        h.m.apply_hook(&report("Notification"));
        assert_eq!(h.focused().agent, crate::agent_state::AgentState::Waiting);
        assert_eq!(h.focused().notified_at, h.m.now_ms);
        h.m.apply_hook(&report("UserPromptSubmit"));
        // Silence short of the window leaves it working: an agent thinking
        // for a minute is not a crashed one.
        h.m.tick(h.m.now_ms + crate::agent_state::STALE_MS - 1_000.);
        h.m.sweep_stale();
        assert_eq!(h.focused().agent, crate::agent_state::AgentState::Working);
        h.m.tick(h.m.now_ms + 2_000.);
        h.m.sweep_stale();
        assert_eq!(h.focused().agent, crate::agent_state::AgentState::None);
    }

    // Four cards dragged all over the canvas come back as a 2x2 in the
    // order you read them, sizes kept, and Cmd+Z scatters them again.
    #[test]
    fn tidy_puts_scattered_cards_back_into_the_block() {
        let mut h = Harness::new();
        for _ in 0..3 {
            h.run("card.new.terminal");
        }
        let scattered = [(9000., 40.), (25., 5000.), (4000., 4000.), (25., 25.)];
        for (c, (x, y)) in h.m.cards.iter_mut().zip(scattered) {
            c.rect.x = x;
            c.rect.y = y;
        }
        let before: Vec<_> = h.m.cards.iter().map(|c| c.rect).collect();
        let effects = h.run("canvas.tidy");
        assert!(
            effects.iter().any(|e| matches!(e, Effect::MarkSwap(_))),
            "glides"
        );
        let r: Vec<_> = h.m.cards.iter().map(|c| c.rect).collect();
        let (w, g) = (r[0].w, crate::cards::GUTTER);
        // Reading order was card 3 (top-left), 0, 2, 1, on the placement
        // grid (anchored at the half cell), not wherever card 3 happened to be.
        let o = crate::grid::HALF_CELL;
        assert_eq!((r[3].x, r[3].y), (o, o));
        assert_eq!((r[0].x, r[0].y), (o + w + g, o));
        assert_eq!((r[2].x, r[2].y), (o, o + r[0].h + g));
        assert_eq!((r[1].x, r[1].y), (o + w + g, o + r[0].h + g));
        assert!(r.iter().zip(&before).all(|(a, b)| (a.w, a.h) == (b.w, b.h)));
        h.run("layout.undo");
        let back: Vec<_> = h.m.cards.iter().map(|c| c.rect).collect();
        assert_eq!(back, before);
    }

    // Clearing a card's state greys its ring and dot until it has something
    // new to say; a card with nothing to say stays as it is.
    #[test]
    fn a_cards_state_colour_can_be_cleared_by_hand() {
        use crate::agent_state::AgentState::*;
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        h.m.card_mut(&id).unwrap().agent = Failed;
        let effects = h.run("card.clearState");
        assert_eq!(h.focused().agent, None);
        assert!(effects.iter().any(|e| matches!(e, Effect::AgentLog(_))));
        let effects = h.run("card.clearState");
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::AgentLog(_))),
            "nothing to log"
        );
    }

    // A shell command that runs long colours its card from the marks in
    // its output: working after five seconds, done when it ends well, and
    // a quiet build is not taken for a crashed agent by the sweep. A
    // replay of the same bytes changes nothing.
    #[test]
    fn a_long_shell_command_colours_its_card() {
        use crate::agent_state::AgentState::*;
        use crate::backend::{PaneEvent, PaneId};
        let mut h = Harness::new();
        let pane: PaneId = 7;
        h.m.cards[0].pane_id = Some(pane);
        let out = |b: &[u8]| PaneEvent::Output(b.to_vec());
        let t0 = h.m.now_ms;
        h.m.apply_pane_event(pane, &out(b"cargo build\r\n\x1b]133;C\x07"));
        h.m.tick(t0 + 1_000.);
        assert_eq!(h.focused().agent, None);
        h.m.tick(t0 + crate::program_state::LONG_MS);
        assert_eq!(h.focused().agent, Working);
        h.m.tick(t0 + crate::agent_state::STALE_MS * 2.);
        h.m.sweep_stale();
        assert_eq!(
            h.focused().agent,
            Working,
            "ten silent minutes is still a build"
        );
        h.m.apply_pane_event(pane, &out(b"\x1b]133;D;0\x07"));
        assert_eq!(h.focused().agent, Done);
        // Every mark reaches agent.log, the end with how long it ran.
        let logged: Vec<String> =
            h.m.take_effects()
                .into_iter()
                .filter_map(|e| match e {
                    Effect::AgentLog(l) => Some(l),
                    _ => Option::None,
                })
                .collect();
        assert!(
            logged
                .iter()
                .any(|l| l.contains("[shell] CommandEnd(Some(0)) after 600.0s working -> done")),
            "{logged:?}"
        );
        h.m.apply_pane_event(pane, &PaneEvent::Replay(b"\x1b]133;C\x07".to_vec()));
        assert_eq!(h.focused().agent, Done, "a replay is old news");
        h.m.apply_pane_event(pane, &out(b"\x1b]133;C\x07"));
        assert_eq!(h.focused().agent, None, "the next command clears it");
    }

    // The keymap dispatches through the editor and browser exceptions.
    #[test]
    fn chords_dispatch_through_the_keymap_with_the_editor_and_browser_exceptions() {
        let mut h = Harness::new();
        assert!(handle_chord(&mut h.m, &h.r, "cmd+t"));
        assert_eq!(h.m.cards.len(), 2);
        assert!(!handle_chord(&mut h.m, &h.r, "cmd+alt+shift+z"), "unbound");
        // An editor you are IN keeps Cmd+F; one you only arrowed to gets
        // its search from the app. A terminal opens the find bar; Cmd+J
        // letters the cards.
        h.m.take_effects();
        let id = h.focused().id.clone();
        h.m.card_mut(&id).unwrap().kind = CardKind::Editor;
        h.m.card_mut(&id).unwrap().locked = true;
        assert!(!handle_chord(&mut h.m, &h.r, "cmd+f"));
        h.m.card_mut(&id).unwrap().locked = false;
        assert!(handle_chord(&mut h.m, &h.r, "cmd+f"));
        assert!(h.m.take_effects().iter().any(|e| matches!(
            e,
            Effect::Editor {
                action: EditorAction::Find,
                ..
            }
        )));
        h.m.card_mut(&id).unwrap().kind = CardKind::Terminal;
        assert!(handle_chord(&mut h.m, &h.r, "cmd+f"));
        assert!(h.m.find.open && h.m.find.card_id.as_deref() == Some(id.as_str()));
        h.m.close_find();
        assert!(handle_chord(&mut h.m, &h.r, "cmd+j"));
        assert!(!h.m.selection.hints.is_empty());
        h.m.selection.hints.clear();
        assert!(handle_chord(&mut h.m, &h.r, "cmd+e"));
        assert!(h
            .m
            .take_effects()
            .contains(&Effect::FindSelection(id.clone())));
        // A browser takes the zoom chords for the page.
        h.m.card_mut(&id).unwrap().kind = CardKind::Browser;
        h.m.card_mut(&id).unwrap().url = Some("https://x".into());
        assert!(handle_chord(&mut h.m, &h.r, "cmd+="));
        assert!((h.focused().zoom.unwrap() - 1.1).abs() < 1e-9);
        assert!(h
            .m
            .take_effects()
            .iter()
            .all(|e| !matches!(e, Effect::AnimateZoom { .. })));
    }

    // A locked editor card's Cmd+T is a tab, its Cmd+W the tab; unlocked,
    // both are the app's. Cmd+F is the editor's while locked, and Ctrl+1
    // the workspace's.
    #[test]
    fn a_locked_editor_card_gets_tab_chords_an_unlocked_one_does_not() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        h.m.card_mut(&id).unwrap().kind = CardKind::Editor;
        h.m.card_mut(&id).unwrap().path = Some("/h/a.rs".into());
        assert!(handle_chord(&mut h.m, &h.r, "cmd+t"));
        assert_eq!(h.m.cards.len(), 2, "unlocked: a new card");
        h.m.set_focus(Some(&id));
        h.m.card_mut(&id).unwrap().locked = true;
        assert!(handle_chord(&mut h.m, &h.r, "cmd+t"));
        assert_eq!(h.m.cards.len(), 2, "locked: no new card");
        assert_eq!(h.m.card(&id).unwrap().tabs.len(), 2, "a new tab instead");
        assert!(handle_chord(&mut h.m, &h.r, "cmd+w"));
        assert_eq!(
            h.m.card(&id).unwrap().tabs.len(),
            1,
            "the tab closed, the card stays"
        );
        assert_eq!(h.m.cards.len(), 2);
        assert!(
            !handle_chord(&mut h.m, &h.r, "cmd+f"),
            "find is the editor's"
        );
        assert!(
            handle_chord(&mut h.m, &h.r, "ctrl+1"),
            "workspaces switch through a lock"
        );
    }

    // Cmd+L is the omnibox on a locked BROWSER card but not on a locked
    // EDITOR one: there it is the editor's "select the line" (Batch 1,
    // 2026-09-24), so it must fall through to the body rather than open
    // the address bar. Ctrl+G is `editor.goToLine`, a command (it opens
    // the model's prompt), unlike the line, bracket and selection chords,
    // which are pure body key handling and need no entry here at all.
    #[test]
    fn a_locked_editor_does_not_carve_out_cmd_l_but_does_bind_ctrl_g() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        h.m.card_mut(&id).unwrap().kind = CardKind::Editor;
        h.m.card_mut(&id).unwrap().path = Some("/h/a.rs".into());
        h.m.card_mut(&id).unwrap().locked = true;
        assert!(
            !handle_chord(&mut h.m, &h.r, "cmd+l"),
            "falls to the body instead of opening the omnibox"
        );
        assert!(!h.m.omni.open);
        assert!(handle_chord(&mut h.m, &h.r, "ctrl+g"));
        assert!(h.m.prompt.is_open());
    }

    // A card dropped over another goes back where it was: nothing may end
    // up behind anything.
    #[test]
    fn a_drag_that_ends_on_another_card_is_put_back() {
        let mut h = Harness::new();
        let a = h.focused().id.clone();
        h.run("card.new.terminal");
        let b = h.focused().id.clone();
        let (ra, rb) = (h.m.card(&a).unwrap().rect, h.m.card(&b).unwrap().rect);
        h.m.card_mut(&b).unwrap().rect = ra;
        assert!(h.m.gesture_overlaps(std::slice::from_ref(&b)));
        assert!(!h
            .m
            .end_gesture(std::slice::from_ref(&b), &[(b.clone(), rb)]));
        assert_eq!(h.m.card(&b).unwrap().rect, rb);
        assert_eq!(h.m.notice.as_deref(), Some("cards cannot overlap"));
        // Clear ground, and an off-grid rect, snap and stand.
        h.m.card_mut(&b).unwrap().rect = crate::grid::Rect {
            x: rb.x + 3000.3,
            y: rb.y,
            w: rb.w,
            h: rb.h,
        };
        assert!(h
            .m
            .end_gesture(std::slice::from_ref(&b), &[(b.clone(), rb)]));
        assert_eq!(
            h.m.card(&b).unwrap().rect,
            crate::grid::snap_rect(crate::grid::Rect {
                x: rb.x + 3000.3,
                ..rb
            })
        );
    }

    // A notice says why a command declined, and expires.
    #[test]
    fn a_refused_command_leaves_a_notice_that_expires() {
        let mut h = Harness::new();
        h.run("card.transcript");
        assert_eq!(
            h.m.notice.as_deref(),
            Some("no agent transcript for this card yet")
        );
        h.m.tick(h.m.now_ms + NOTICE_MS - 1.);
        assert!(h.m.notice.is_some());
        h.m.tick(h.m.now_ms + 1.);
        assert!(h.m.notice.is_none());
    }

    // "move card" in the palette, Enter, then the workspace: the card goes
    // there, into a free slot (never onto the card already there), you stay
    // where you were, and going there lands on it.
    #[test]
    fn a_card_moves_to_another_workspace_and_you_stay() {
        let mut h = Harness::new();
        let here = h.m.active_workspace.clone().unwrap();
        let kept = h.focused().id.clone();
        h.run("card.new.terminal");
        let mover = h.focused().id.clone();
        let there = h.m.add_workspace(Some("Acme"));
        let resident = h.m.add_card(
            "/Users/me",
            NewCard {
                workspace_id: Some(there.clone()),
                ..Default::default()
            },
        );
        h.m.set_focus(Some(&mover));
        h.run("card.moveToWorkspace");
        assert_eq!(h.m.palette.source, Some(Source::MoveTo));
        let rows = h.m.palette_items(Source::MoveTo, &[]);
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Acme", "New workspace"], "not the one you are on");
        assert_eq!(rows[0].hint.as_deref(), Some("1 card"));
        h.m.palette_run(Source::MoveTo, &there);
        let moved = h.m.card(&mover).unwrap().clone();
        assert_eq!(moved.workspace_id, there);
        let r = h.m.card(&resident).unwrap().rect;
        assert!(!crate::layout::rects_overlap(moved.rect, r), "a free slot");
        assert_eq!(
            h.m.active_workspace.as_deref(),
            Some(here.as_str()),
            "you stay"
        );
        assert_eq!(h.focused().id, kept);
        assert_eq!(
            h.m.notice.as_deref(),
            Some(format!("moved #{} to Acme", moved.number).as_str())
        );
        let ws = h.m.workspaces.iter().find(|w| w.id == there).unwrap();
        assert_eq!(
            ws.focused.as_deref(),
            Some(mover.as_str()),
            "lands on it there"
        );
    }

    #[test]
    fn a_card_can_move_to_a_new_workspace() {
        let mut h = Harness::new();
        h.run("card.new.terminal");
        let mover = h.focused().id.clone();
        let before = h.m.workspaces.len();
        h.run("card.moveToWorkspace");
        h.m.palette_run(
            Source::MoveTo,
            super::super::workspaces_cmd::NEW_WORKSPACE_ROW,
        );
        assert_eq!(h.m.workspaces.len(), before + 1);
        let new_ws = h.m.workspaces.last().unwrap().id.clone();
        assert_eq!(h.m.card(&mover).unwrap().workspace_id, new_ws);
    }

    // An untitled editor's first save: the field starts on the card's
    // directory and untitled.txt with the stem selected; a new path saves;
    // a missing directory sends you back to the field with what you typed;
    // an existing file asks before replacing it.
    #[test]
    fn saving_an_untitled_buffer_names_it_like_the_mac_panel() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        {
            let c = h.m.card_mut(&id).unwrap();
            c.kind = CardKind::Editor;
            c.path = None;
            c.cwd = "/Users/me/Code".into();
        }
        let disk = ["/Users/me/Code", "/Users/me/Code/old.txt"];
        let exists = |p: &str| disk.contains(&p);
        let submit = |h: &mut Harness, typed: &str| {
            let (pending, text) = h.m.prompt.settle(Some(typed)).unwrap();
            h.m.answer(pending, text, exists);
            h.m.take_effects()
        };
        h.run("card.save");
        assert_eq!(h.m.prompt.value, "~/Code/untitled.txt");
        assert_eq!(h.m.prompt.select, Some((7, 15)));

        submit(&mut h, "~/Code/nope/a.txt");
        assert!(h.m.prompt.open, "back to the field");
        assert_eq!(h.m.prompt.value, "~/Code/nope/a.txt");
        assert!(h
            .m
            .notice
            .as_deref()
            .unwrap()
            .contains("no such directory: ~/Code/nope"));

        submit(&mut h, "~/Code/old.txt");
        assert!(h.m.prompt.confirm && h.m.prompt.label.contains("old.txt exists"));
        let (pending, text) = h.m.prompt.settle(None).unwrap();
        h.m.answer(pending, text, exists);
        assert_eq!(h.m.card(&id).unwrap().path, None, "not replaced");

        h.run("card.save");
        let effects = submit(&mut h, "~/Code/notes.md");
        assert_eq!(
            h.m.card(&id).unwrap().path.as_deref(),
            Some("/Users/me/Code/notes.md")
        );
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::Editor {
                action: EditorAction::Save,
                ..
            }
        )));
    }

    // Closing an untitled buffer: Save in the sheet asks for the name, and
    // the card closes once that save lands.
    #[test]
    fn saving_an_untitled_buffer_from_the_close_sheet_names_it_then_closes() {
        use crate::prompt::ButtonKind;
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        {
            let c = h.m.card_mut(&id).unwrap();
            c.kind = CardKind::Editor;
            c.path = None;
            c.cwd = "/Users/me".into();
            c.dirty = true;
        }
        h.run("card.close");
        let (pending, text) = h.m.prompt.press(ButtonKind::Primary).unwrap();
        h.m.answer(pending, text, |_| true);
        assert!(h.m.prompt.open && h.m.prompt.label == "save as");
        let (pending, text) = h.m.prompt.settle(Some("/Users/me/n.txt")).unwrap();
        h.m.answer(pending, text, |p: &str| p == "/Users/me");
        assert_eq!(h.m.cards.len(), 1, "waits for the save");
        h.m.card_mut(&id).unwrap().dirty = false;
        h.m.tick(h.m.now_ms + 16.);
        assert!(h.m.cards.is_empty());
    }

    // A dirty editor asks the save sheet; a second Cmd+W only asks again.
    // Cancel keeps it; Don't Save discards; Save saves and closes once the
    // editor reports the file clean, and a save that never lands closes
    // nothing.
    #[test]
    fn a_dirty_editor_asks_save_dont_save_or_cancel() {
        use crate::prompt::ButtonKind;
        let dirty_editor = |h: &mut Harness| {
            let id = h.focused().id.clone();
            let c = h.m.card_mut(&id).unwrap();
            c.kind = CardKind::Editor;
            c.path = Some("/tmp/x.txt".into());
            c.dirty = true;
            id
        };
        let press = |h: &mut Harness, k: ButtonKind| {
            let (pending, text) = h.m.prompt.press(k).unwrap();
            h.m.answer(pending, text, |_| true);
            h.m.take_effects()
        };
        let mut h = Harness::new();
        let id = dirty_editor(&mut h);
        h.run("card.close");
        h.run("card.close");
        assert_eq!(h.m.cards.len(), 1, "a double press is not an answer");
        assert!(h.m.prompt.open && h.m.prompt.label.starts_with("save changes to"));
        press(&mut h, ButtonKind::Cancel);
        assert_eq!(h.m.cards.len(), 1, "Cancel keeps it");

        h.run("card.close");
        let effects = press(&mut h, ButtonKind::Primary);
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::Editor {
                action: EditorAction::Save,
                ..
            }
        )));
        assert_eq!(h.m.cards.len(), 1, "not before the save lands");
        h.m.card_mut(&id).unwrap().dirty = false; // the editor's mirror
        h.m.tick(h.m.now_ms + 16.);
        assert!(h.m.cards.is_empty(), "saved, then closed");

        let mut h = Harness::new();
        dirty_editor(&mut h);
        h.run("card.close");
        press(&mut h, ButtonKind::Primary);
        h.m.tick(h.m.now_ms + 3000.);
        assert_eq!(
            h.m.cards.len(),
            1,
            "a save that never landed closes nothing"
        );

        let mut h = Harness::new();
        let id = dirty_editor(&mut h);
        h.run("card.close");
        let effects = press(&mut h, ButtonKind::Alt);
        assert!(h.m.cards.is_empty(), "Don't Save discards");
        assert!(effects.contains(&Effect::DraftDelete(id)));
    }

    // The wheel asks this one question before it touches the canvas: a
    // scroll inside an overlay must not reach the card under the pointer.
    #[test]
    fn every_overlay_reports_itself_as_open() {
        let mut h = Harness::new();
        assert!(!h.m.overlay_open());
        h.run("app.palette");
        assert!(h.m.overlay_open());
        h.m.close_palette(false);
        assert!(!h.m.overlay_open());
        h.run("app.shortcuts");
        assert!(h.m.overlay_open());
        h.m.shortcuts_open = false;
        h.run("card.rename");
        assert!(h.m.overlay_open(), "a prompt is an overlay too");
    }

    // Find in page: the bar belongs to one card, typing searches, Enter
    // steps, and closing always clears the highlights behind it.
    #[test]
    fn the_find_bar_belongs_to_the_card_it_was_opened_on() {
        let mut h = Harness::new();
        let id = h.m.cards[0].id.clone();
        h.m.set_focus(Some(&id));
        // An editor has its own search; the bar is for pages and terminals.
        h.m.cards[0].kind = CardKind::Editor;
        h.run("browser.find");
        assert_eq!(h.m.notice.as_deref(), Some("nothing to find in here"));
        assert!(!h.m.find.open);

        h.m.cards[0].kind = CardKind::Browser;
        h.run("browser.find");
        assert!(h.m.find.open);
        assert_eq!(h.m.find.card_id.as_deref(), Some(id.as_str()));

        h.m.find_type("rope");
        let effects = h.m.take_effects();
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::Find { request: Some(r), .. }
            if r.text == "rope" && !r.next)));

        h.m.find_step(false);
        assert!(h
            .m
            .take_effects()
            .iter()
            .any(|e| matches!(e, Effect::Find { request: Some(r), .. }
            if r.next && !r.forward)));

        // Chromium's count reaches the bar only for the card it is on.
        h.m.find_result(&id, 12, 3);
        assert_eq!((h.m.find.matches, h.m.find.active), (12, 3));
        h.m.find_result("someone-else", 1, 1);
        assert_eq!((h.m.find.matches, h.m.find.active), (12, 3));

        // Closing clears: a highlight that outlives the bar cannot be got
        // rid of.
        h.m.close_find();
        assert!(!h.m.find.open);
        assert!(h
            .m
            .take_effects()
            .iter()
            .any(|e| matches!(e, Effect::Find { request: None, .. })));
    }

    #[test]
    fn looking_at_another_card_closes_the_find_bar() {
        let mut h = Harness::new();
        h.m.cards[0].kind = CardKind::Browser;
        let first = h.m.cards[0].id.clone();
        h.m.set_focus(Some(&first));
        h.run("browser.find");
        h.m.find_type("rope");
        h.m.take_effects();
        let effects = h.run("card.new.terminal");
        assert!(!h.m.find.open, "the bar went with the card");
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::Find { request: None, .. })));
    }

    // Splitting while framing used to drop the view onto the new half and
    // put the other one off screen. A split subdivides where you already
    // are; it does not move you somewhere else.
    #[test]
    fn splitting_while_framing_keeps_both_halves_in_view() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        let whole = h.m.card(&id).unwrap().rect;
        h.m.view_size = crate::grid::Size { w: 1400., h: 900. };
        h.run("canvas.zoom.fitCard");
        assert!(h.m.framing);
        h.run("card.split.down");
        assert_eq!(h.m.cards.len(), 2);
        // The frame covers the WHOLE original slot, so the top half is
        // still on screen.
        let rects: Vec<_> = h.m.cards.iter().map(|c| c.rect).collect();
        let both = crate::viewport::bounding_rect(&rects).unwrap();
        assert!(
            (both.h - whole.h).abs() < 1.,
            "the two halves still add up to the slot"
        );
        let framed =
            h.m.slot_bounds(
                &[h.m.focused().unwrap().rect],
                &[h.m.focused().unwrap().soft_group_id.clone()],
            )
            .unwrap();
        assert!(
            (framed.h - whole.h).abs() < 1.,
            "framing the new half frames the slot, not the half: got {framed:?}"
        );
    }

    // A half dragged away from its partner is not a slot any more, and
    // framing the pair would zoom out to nothing.
    #[test]
    fn a_half_that_has_wandered_is_framed_alone() {
        let mut h = Harness::new();
        h.m.view_size = crate::grid::Size { w: 1400., h: 900. };
        h.run("card.split.down");
        let moved = h.m.cards[1].id.clone();
        h.m.card_mut(&moved).unwrap().rect.x += 20_000.;
        let card = h.m.cards[0].clone();
        let framed =
            h.m.slot_bounds(&[card.rect], std::slice::from_ref(&card.soft_group_id))
                .unwrap();
        assert_eq!(
            framed, card.rect,
            "the runaway half is not part of the slot"
        );
    }

    // Closing has to be undoable or nobody closes anything, which is how a
    // canvas ends up as bad as a tab bar.
    #[test]
    fn a_closed_card_comes_back_where_it_was() {
        let mut h = Harness::new();
        let id = h.m.cards[0].id.clone();
        let rect = h.m.cards[0].rect;
        h.m.cards[0].cwd = "/tmp/somewhere".into();
        h.run("card.close");
        assert!(h.m.cards.is_empty());
        h.run("card.reopen");
        assert_eq!(h.m.cards.len(), 1);
        assert_eq!(h.m.cards[0].id, id, "the same card, not a new one");
        assert_eq!(h.m.cards[0].rect, rect, "its own space was still free");
        assert_eq!(h.m.cards[0].cwd, "/tmp/somewhere");
        assert_eq!(h.m.selection.focused_id.as_deref(), Some(id.as_str()));
        // The ring is empty again: reopening is not a copy machine.
        h.run("card.reopen");
        assert_eq!(h.m.cards.len(), 1);
        assert_eq!(h.m.notice.as_deref(), Some("nothing to reopen"));
    }

    // A restored card is a fresh shell in the same directory, exactly as it
    // is when the save file is read: nothing resurrects an agent.
    #[test]
    fn a_reopened_card_carries_no_runtime_state() {
        let mut h = Harness::new();
        h.m.cards[0].command = Some("claude".into());
        h.m.cards[0].agent = crate::agent_state::AgentState::Working;
        h.m.cards[0].transcript_path = Some("/s/x.jsonl".into());
        h.m.cards[0].pane_id = Some(7);
        h.run("card.close");
        h.run("card.reopen");
        let back = &h.m.cards[0];
        assert_eq!(back.command, None);
        assert_eq!(back.agent, crate::agent_state::AgentState::None);
        assert_eq!(back.transcript_path, None);
        assert_eq!(back.pane_id, None);
    }

    // Somewhere else is now standing where it was: it comes back beside
    // what is there rather than on top of it.
    #[test]
    fn a_reopened_card_avoids_whatever_took_its_place() {
        let mut h = Harness::new();
        let rect = h.m.cards[0].rect;
        h.run("card.close");
        h.run("card.new.terminal");
        let taken = h.m.cards[0].rect;
        assert_eq!(taken, rect, "the new card took the free slot");
        h.run("card.reopen");
        assert_eq!(h.m.cards.len(), 2);
        assert_ne!(h.m.cards[1].rect, rect);
    }

    // Back and forward reach the PAGE: the model keeps no history of its
    // own, and a card that is not a browser says so rather than doing
    // nothing anybody can see.
    #[test]
    fn back_and_forward_go_to_the_page_and_only_from_a_browser_card() {
        let mut h = Harness::new();
        let id = h.m.cards[0].id.clone();
        h.m.set_focus(Some(&id));
        let effects = h.run("browser.back");
        assert_eq!(h.m.notice.as_deref(), Some("not a browser card"));
        assert!(!effects.iter().any(|e| matches!(e, Effect::Browser { .. })));

        h.m.cards[0].kind = CardKind::Browser;
        assert!(h.run("browser.back").contains(&Effect::Browser {
            card_id: id.clone(),
            action: crate::model::BrowserAction::Back,
        }));
        assert!(h.run("browser.forward").contains(&Effect::Browser {
            card_id: id,
            action: crate::model::BrowserAction::Forward,
        }));
    }

    // Locking a browser card hands Chrome's own tab shortcuts to the page's
    // chrome instead of the app's: Cmd+T stops making a new card and starts
    // a tab on the one that is locked.
    #[test]
    fn a_locked_browser_card_gets_chrome_shortcuts_an_unlocked_one_does_not() {
        let mut h = Harness::new();
        let id = h.m.cards[0].id.clone();
        h.m.cards[0].kind = CardKind::Browser;
        h.m.set_focus(Some(&id));

        // Unlocked: cmd+t is the app's "new card", not a tab.
        let before = h.m.cards.len();
        assert!(handle_chord(&mut h.m, &h.r, "cmd+t"));
        assert_eq!(
            h.m.cards.len(),
            before + 1,
            "unlocked cmd+t made a new card"
        );
        h.m.cards.pop(); // undo, keep the harness on the browser card for the next part
        h.m.set_focus(Some(&id));

        // Locked: cmd+t opens a tab on the SAME card instead.
        h.m.cards[0].locked = true;
        assert!(handle_chord(&mut h.m, &h.r, "cmd+t"));
        assert_eq!(h.m.cards.len(), before, "no new card");
        assert_eq!(h.m.card(&id).unwrap().tabs.len(), 2, "a tab instead");
    }

    // The keycast labels a chord with what `resolve_chord` says, so on a
    // locked card it must name the tab command, not the keymap's card one.
    #[test]
    fn resolve_chord_names_the_command_a_locked_card_runs() {
        let mut h = Harness::new();
        let id = h.m.cards[0].id.clone();
        h.m.set_focus(Some(&id));
        assert_eq!(
            resolve_chord(&h.m, "cmd+t").as_deref(),
            Some("card.new.terminal")
        );
        h.m.cards[0].kind = CardKind::Browser;
        h.m.cards[0].locked = true;
        assert_eq!(
            resolve_chord(&h.m, "cmd+t").as_deref(),
            Some("browser.tab.new")
        );
        h.m.cards[0].kind = CardKind::Editor;
        assert_eq!(
            resolve_chord(&h.m, "cmd+t").as_deref(),
            Some("editor.tab.new")
        );
        // Unknown to every table: the body's.
        assert_eq!(resolve_chord(&h.m, "cmd+shift+alt+ctrl+9"), None);
    }

    // An arrowed-to editor is a card: the canvas's chords are the canvas's
    // until you are in it. Locked, the same chords are the editor's.
    #[test]
    fn an_editor_keeps_its_chords_only_once_you_are_in_it() {
        let mut h = Harness::new();
        let id = h.m.cards[0].id.clone();
        h.m.set_focus(Some(&id));
        h.m.cards[0].kind = CardKind::Editor;
        h.m.cards[0].locked = false;
        assert_eq!(
            resolve_chord(&h.m, "cmd+/").as_deref(),
            Some("app.shortcuts")
        );
        assert_eq!(resolve_chord(&h.m, "cmd+z").as_deref(), Some("layout.undo"));
        h.m.cards[0].locked = true;
        assert_eq!(resolve_chord(&h.m, "cmd+/"), None, "the comment toggle");
        assert_eq!(resolve_chord(&h.m, "cmd+z"), None, "the buffer's undo");
        assert_eq!(resolve_chord(&h.m, "cmd+s").as_deref(), Some("card.save"));
    }

    // Ctrl+digit is workspace switching, never a Chrome shortcut, so lock
    // does not touch it.
    #[test]
    fn ctrl_digit_still_switches_workspaces_while_locked() {
        let mut h = Harness::new();
        let id = h.m.cards[0].id.clone();
        h.m.cards[0].kind = CardKind::Browser;
        h.m.cards[0].locked = true;
        h.m.set_focus(Some(&id));
        assert!(handle_chord(&mut h.m, &h.r, "ctrl+2"));
    }

    // A chord neither table knows falls through to the page instead of
    // being swallowed: `browser_body::key` forwards a Cmd chord to
    // `Surface::edit_chord` (copy, paste, cut, select-all, undo, redo), and
    // that is real Chrome parity for a locked card, matching an unlocked
    // one. `handle_chord` itself has no page to hand it to, so the contract
    // is just that it reports "not consumed" and lets `key_down` carry on
    // to `body.key()`.
    #[test]
    fn a_locked_card_lets_an_unmatched_chord_fall_through_to_the_page() {
        let mut h = Harness::new();
        let id = h.m.cards[0].id.clone();
        h.m.cards[0].kind = CardKind::Browser;
        h.m.cards[0].locked = true;
        h.m.set_focus(Some(&id));
        let before = h.m.cards.len();
        assert!(
            !handle_chord(&mut h.m, &h.r, "cmd+shift+z"),
            "not consumed: falls through to the body"
        );
        assert_eq!(h.m.cards.len(), before, "the app's keymap did not run");
    }

    // Cmd+C (and the rest of edit_chord's set) must reach the body the same
    // way on a locked card: `handle_chord` returning true here would mean
    // copy/paste/cut/select-all/undo/redo silently stop working the moment
    // a browser card locks, which is the opposite of "real Chrome".
    #[test]
    fn a_locked_card_lets_edit_chords_fall_through_to_the_page() {
        let mut h = Harness::new();
        let id = h.m.cards[0].id.clone();
        h.m.cards[0].kind = CardKind::Browser;
        h.m.cards[0].locked = true;
        h.m.set_focus(Some(&id));
        for chord in ["cmd+c", "cmd+v", "cmd+x", "cmd+a", "cmd+z", "cmd+shift+z"] {
            assert!(
                !handle_chord(&mut h.m, &h.r, chord),
                "{chord} must fall through to the body, not be swallowed"
            );
        }
    }

    // Two chords stay the app's even on a locked card: the omnibox (an app
    // overlay, not the page) and the way out of a locked page
    // (`browser.leave`, `cmd+escape` in the default keymap). Neither is in
    // `lock_override` or `browser_override`, so without an explicit
    // carve-out they would fall into the "unmatched, swallowed" branch
    // before this fix and now would fall through to the page instead of
    // running: both wrong, and this is the regression guard for either.
    #[test]
    fn cmd_l_and_browser_leave_stay_the_apps_even_while_locked() {
        let mut h = Harness::new();
        // A second card for `browser.leave` to hand focus back to.
        h.run("card.new.terminal");
        let other = h.focused().id.clone();
        let id = h.m.cards[0].id.clone();
        h.m.cards[0].kind = CardKind::Browser;
        h.m.cards[0].url = Some("https://a.example".into());
        h.m.cards[0].locked = true;
        h.m.set_focus(Some(&id));

        assert!(handle_chord(&mut h.m, &h.r, "cmd+l"), "the omnibox opened");
        assert!(h.m.omni.open);
        h.m.close_omnibox();
        h.m.set_focus(Some(&id));

        let leave_chord = crate::keymap::default_keymap()
            .iter()
            .find(|(_, cmd_id)| cmd_id == "browser.leave")
            .map(|(c, _)| c.clone())
            .expect("browser.leave is bound");
        assert!(
            handle_chord(&mut h.m, &h.r, &leave_chord),
            "browser.leave ran, reaching the app's keymap rather than the page"
        );
        assert_eq!(
            h.m.selection.focused_id.as_deref(),
            Some(other.as_str()),
            "left the locked card's page for the nearest other card"
        );
    }

    #[test]
    fn workspace_switch_detection_is_ctrl_plus_one_digit_only() {
        assert!(is_workspace_switch("ctrl+5"));
        assert!(!is_workspace_switch("ctrl+55"));
        assert!(!is_workspace_switch("cmd+5"));
        assert!(!is_workspace_switch("ctrl+shift+5"));
    }

    // Cmd+L on a browser card edits THAT card's address; anywhere else it
    // makes a card. The prefill is the whole reason the field is not empty.
    #[test]
    fn the_omnibox_prefills_from_a_browser_card_and_navigates_it() {
        let mut h = Harness::new();
        let id = h.m.cards[0].id.clone();
        h.m.cards[0].kind = CardKind::Browser;
        h.m.cards[0].url = Some("https://a.example/one".into());
        h.m.set_focus(Some(&id));
        h.run("card.omnibox");
        assert!(h.m.omni.open);
        assert_eq!(h.m.omni.query, "https://a.example/one");
        assert_eq!(h.m.omni.target.as_deref(), Some(id.as_str()));
        h.m.omni_type("b.example");
        h.m.omni_enter();
        assert!(!h.m.omni.open);
        assert_eq!(h.m.cards.len(), 1, "the focused browser card was navigated");
        assert_eq!(
            h.m.card(&id).unwrap().url.as_deref(),
            Some("https://b.example")
        );
    }

    #[test]
    fn alt_enter_opens_a_new_tab_instead_of_navigating_in_place() {
        let mut m = Model::new();
        m.home = "/h".into();
        m.start_dir = "/h".into();
        let id = m.add_card(
            "/h",
            NewCard {
                kind: CardKind::Browser,
                url: Some("https://a.example".into()),
                ..Default::default()
            },
        );
        m.set_focus(Some(&id));
        m.open_omnibox();
        m.omni_type("https://b.example");
        m.omni_enter_new_tab();
        assert!(!m.omni.open);
        assert_eq!(
            m.card(&id).unwrap().tabs.len(),
            2,
            "opened as a tab, not navigated in place"
        );
        assert_eq!(
            m.card(&id).unwrap().url.as_deref(),
            Some("https://b.example")
        );
    }

    #[test]
    fn the_omnibox_makes_a_card_when_no_browser_is_focused() {
        let mut h = Harness::new();
        h.run("card.omnibox");
        assert_eq!(h.m.omni.query, "", "nothing to prefill from a terminal");
        assert!(h.m.omni.target.is_none());
        h.m.omni_type("example.com");
        h.m.omni_enter();
        assert_eq!(h.m.cards.len(), 2);
        let made = h.m.cards.last().unwrap();
        assert_eq!(made.kind, CardKind::Browser);
        assert_eq!(made.url.as_deref(), Some("https://example.com"));
    }

    // The placement menu's "browser" always makes a NEW card, even when a
    // browser card is the one focused: `card.omnibox` (Cmd+L) would edit it
    // in place, which is the wrong thing here, so this is its own command
    // rather than `card.omnibox` reused with different state.
    #[test]
    fn card_new_browser_never_edits_the_focused_browser_card() {
        let mut h = Harness::new();
        let editing = h.m.cards[0].id.clone();
        h.m.cards[0].kind = CardKind::Browser;
        h.m.cards[0].url = Some("https://a.example".into());
        h.m.set_focus(Some(&editing));
        h.run("card.new.browser");
        assert!(h.m.omni.open);
        assert_eq!(h.m.omni.query, "");
        assert!(h.m.omni.target.is_none());
        assert!(!h.m.omni.phantom);
        h.m.omni_type("b.example");
        h.m.omni_enter();
        assert_eq!(h.m.cards.len(), 2, "a second card, the first left alone");
        assert_eq!(
            h.m.card(&editing).unwrap().url.as_deref(),
            Some("https://a.example")
        );
        assert_eq!(h.focused().url.as_deref(), Some("https://b.example"));
    }

    // A card result focuses, it does not navigate: the page is already open.
    #[test]
    fn choosing_an_open_card_focuses_it() {
        let mut h = Harness::new();
        let first = h.m.cards[0].id.clone();
        h.m.new_beside_active(
            CardKind::Browser,
            None,
            Some("https://news.ycombinator.com".into()),
        );
        let browser = h.m.cards.last().unwrap().id.clone();
        h.m.card_mut(&browser).unwrap().title = "Hacker News".into();
        h.m.set_focus(Some(&first));
        h.run("card.omnibox");
        h.m.omni_type("hacker");
        let response = h.m.omni_response();
        let cards = response
            .sections
            .iter()
            .find(|s| s.heading == "cards")
            .expect("a cards section");
        assert_eq!(
            cards.results[0].action,
            crate::omni::OmniAction::FocusCard(browser.clone())
        );
    }

    #[test]
    fn tab_scopes_and_unscoping_leaves() {
        let mut h = Harness::new();
        h.run("card.omnibox");
        h.m.omni_type("git");
        assert_eq!(
            h.m.omni_response().offer,
            Some(("github.com".into(), "GitHub".into()))
        );
        h.m.omni_tab();
        assert_eq!(h.m.omni.scope.as_deref(), Some("github.com"));
        assert_eq!(h.m.omni.query, "", "the field clears for the query");
        h.m.omni_unscope();
        assert!(h.m.omni.scope.is_none());
    }

    // A response for a query that has moved on must never reach the screen.
    #[test]
    fn a_stale_suggestion_response_is_dropped() {
        let mut h = Harness::new();
        h.m.config.browser.suggestions = true;
        h.run("card.omnibox");
        h.m.omni_type("ru");
        let first = h.m.omni.query_id;
        h.m.omni_type("rus");
        h.m.omni_suggestions(first, vec!["stale".into()]);
        assert!(h.m.omni.suggestions.is_empty());
        let current = h.m.omni.query_id;
        h.m.omni_suggestions(current, vec!["rust book".into()]);
        assert_eq!(h.m.omni.suggestions, vec!["rust book".to_string()]);
    }

    #[test]
    fn typing_asks_for_suggestions_only_when_the_setting_is_on() {
        let mut h = Harness::new();
        h.run("card.omnibox");
        h.m.omni_type("rust");
        assert!(!h
            .m
            .take_effects()
            .iter()
            .any(|e| matches!(e, Effect::FetchSuggestions { .. })));
        h.m.config.browser.suggestions = true;
        h.m.omni_type("rust h");
        assert!(h
            .m
            .take_effects()
            .iter()
            .any(|e| matches!(e, Effect::FetchSuggestions { .. })));
    }

    // Palette: the theme picker previews as the selection moves and puts the
    // old theme back on dismissal; use is recorded on run only.
    #[test]
    fn the_theme_picker_previews_and_restores() {
        let mut h = Harness::new();
        h.m.theme_names = vec!["A".into(), "B".into(), "Current".into()];
        h.m.theme_current = Some("Current".into());
        h.run("theme.pick");
        assert_eq!(h.m.palette.source, Some(Source::Themes));
        let items = h.m.palette_items(Source::Themes, &[]);
        assert_eq!(items[0].id, "Current", "the theme in force is listed first");
        assert_eq!(items[0].hint.as_deref(), Some("active"), "and says so");
        assert!(items[1..].iter().all(|i| i.hint.is_none()));
        h.m.palette_preview(Source::Themes, Some("A"));
        assert!(h.m.take_effects().contains(&Effect::LoadTheme("A".into())));
        // The preview makes A the theme in force; the rows must not move
        // under the highlight because of it (#67).
        h.m.theme_current = Some("A".into());
        let ids: Vec<String> =
            h.m.palette_items(Source::Themes, &[])
                .into_iter()
                .map(|i| i.id)
                .collect();
        assert_eq!(
            ids,
            ["Current", "A", "B"],
            "the order holds while previewing"
        );
        h.m.close_palette(false);
        assert!(h
            .m
            .take_effects()
            .contains(&Effect::LoadTheme("Current".into())));
        assert!(h.m.usage.is_empty());
        h.m.note_use(Source::Themes, "A");
        h.m.palette_run(Source::Themes, "A");
        assert!(h
            .m
            .take_effects()
            .iter()
            .any(|e| matches!(e, Effect::SaveSetting { path, .. } if path == "theme")));
        assert!(
            h.m.usage.is_empty(),
            "the theme list keeps its order, so a pick is not recorded (#72)"
        );
    }

    // A menu is muscle memory. The placement menu and the phantom's kind
    // picker keep the order they were designed in, and using them leaves
    // no record that could reorder them next time.
    #[test]
    fn menus_keep_their_order_and_record_no_use() {
        let mut h = Harness::new();
        assert!(Source::Placement.keeps_its_order());
        assert!(Source::SlotKind.keeps_its_order());
        assert!(
            !Source::Commands.keeps_its_order(),
            "hundreds of commands: recency helps"
        );
        assert!(
            Source::Themes.keeps_its_order(),
            "the active theme on top, then A to Z (#72)"
        );
        h.m.note_use(Source::Placement, "claude");
        h.m.note_use(Source::SlotKind, "browser");
        assert!(h.m.usage.is_empty(), "a menu choice is not remembered");
        h.m.note_use(Source::Commands, "canvas.zoom.in");
        assert_eq!(h.m.usage.len(), 1, "a command choice still is");
    }

    #[test]
    fn the_command_source_lists_every_command_but_the_palette_with_its_chord() {
        let h = Harness::new();
        let labels: Vec<(&str, &str)> =
            h.r.all()
                .iter()
                .map(|c| (c.id.as_str(), c.label.as_str()))
                .collect();
        let items = h.m.palette_items(Source::Commands, &labels);
        assert!(items.iter().all(|i| i.id != "app.palette"));
        let close = items.iter().find(|i| i.id == "card.close").unwrap();
        assert_eq!(close.hint.as_deref(), Some("Cmd W"));
    }

    // The emoji panel has one owner: our chord, our command, one effect
    // the frame turns into the call. Not a gpui binding, not macOS's own
    // handling, which between them made it work sometimes.
    #[test]
    fn the_emoji_panel_is_a_command_with_its_chord() {
        let mut h = Harness::new();
        let effects = h.run("app.emoji");
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::ShowCharacterPalette)));
        assert_eq!(
            crate::keymap::default_keymap()
                .iter()
                .find(|(_, id)| id == "app.emoji")
                .map(|(c, _)| c.as_str()),
            Some("cmd+ctrl+space")
        );
    }

    #[test]
    fn full_screen_is_a_command_on_the_systems_chord() {
        let mut h = Harness::new();
        let effects = h.run("app.fullscreen");
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::ToggleFullScreen)));
        assert_eq!(
            crate::keymap::default_keymap()
                .iter()
                .find(|(_, id)| id == "app.fullscreen")
                .map(|(c, _)| c.as_str()),
            Some("cmd+ctrl+f")
        );
    }

    // Switching back to a workspace lands on the card you were working
    // in there, not on whichever card was made first.
    #[test]
    fn a_workspace_remembers_which_card_was_focused() {
        let mut h = Harness::new();
        let ws1 = h.m.active_workspace.clone().unwrap();
        let first = h.focused().id.clone();
        h.run("card.new.terminal");
        let second = h.focused().id.clone();
        assert_ne!(first, second);
        // Working in the second card, leave for a new workspace.
        h.run("workspace.new");
        let ws2 = h.m.active_workspace.clone().unwrap();
        assert_ne!(ws1, ws2);
        assert_ne!(h.focused().id, second, "the new workspace has its own card");
        // And come back.
        h.m.show_workspace(&ws1);
        assert_eq!(
            h.focused().id,
            second,
            "back where we were, not on the first card"
        );
        // A remembered card that has since closed falls back to the first.
        h.m.show_workspace(&ws2);
        h.m.workspaces
            .iter_mut()
            .find(|w| w.id == ws1)
            .unwrap()
            .focused = Some("gone".into());
        h.m.show_workspace(&ws1);
        assert_eq!(h.focused().id, first);
    }

    // The palette is also the way to a card by name, across workspaces:
    // every card is a row, a card elsewhere says where, and Enter switches
    // there, focuses it and fits it, which is Cmd+1 once you have arrived.
    #[test]
    fn the_palette_lists_cards_and_workspaces_and_enter_goes_there() {
        use super::super::palette_state::{CARD_ROW, WORKSPACE_ROW};
        let mut h = Harness::new();
        let home_ws = h.m.active_workspace.clone().unwrap();
        let first = h.focused().id.clone();
        h.m.card_mut(&first).unwrap().title = "embers".into();
        h.run("workspace.new");
        let other_ws = h.m.active_workspace.clone().unwrap();
        assert_ne!(other_ws, home_ws);
        h.m.workspaces
            .iter_mut()
            .find(|w| w.id == other_ws)
            .unwrap()
            .name = "side".into();

        let labels: Vec<(&str, &str)> =
            h.r.all()
                .iter()
                .map(|c| (c.id.as_str(), c.label.as_str()))
                .collect();
        let items = h.m.palette_items(Source::Commands, &labels);
        let row = items
            .iter()
            .find(|i| i.id == format!("{CARD_ROW}{first}"))
            .expect("the card is a row");
        assert!(row.label.starts_with("Card: #1 embers"), "{}", row.label);
        assert!(
            row.label.contains("workspace 1") || row.label.contains("\u{b7}"),
            "a card elsewhere says which workspace: {}",
            row.label
        );
        assert!(
            items
                .iter()
                .any(|i| i.id == format!("{WORKSPACE_ROW}{home_ws}")),
            "the other workspace is a row"
        );
        assert!(
            !items
                .iter()
                .any(|i| i.id == format!("{WORKSPACE_ROW}{other_ws}")),
            "the one you are in is not: choosing it would do nothing"
        );

        h.m.palette_run(Source::Commands, &format!("{CARD_ROW}{first}"));
        assert_eq!(
            h.m.active_workspace.as_deref(),
            Some(home_ws.as_str()),
            "switched back"
        );
        assert_eq!(h.focused().id, first, "and focused it");
        assert!(
            h.m.take_effects()
                .iter()
                .any(|e| matches!(e, Effect::RunCommand(c) if c == "canvas.zoom.fitCard")),
            "and fitted it, as Cmd+1 would"
        );

        h.m.palette_run(Source::Commands, &format!("{WORKSPACE_ROW}{other_ws}"));
        assert_eq!(h.m.active_workspace.as_deref(), Some(other_ws.as_str()));
    }

    // An untitled editor's save asks where; the answer names the card and
    // the save then runs. Relative to the card's directory, `~` allowed.
    #[test]
    fn saving_an_untitled_editor_asks_for_a_path_then_saves() {
        let mut h = Harness::new();
        h.run("card.new.editor");
        let id = h.focused().id.clone();
        assert_eq!(h.focused().kind, crate::saved_layout::CardKind::Editor);
        assert!(h.focused().path.is_none());
        h.run("card.save");
        assert_eq!(h.m.prompt.label, "save as");
        assert!(!h
            .m
            .take_effects()
            .iter()
            .any(|e| matches!(e, Effect::Editor { .. })));
        let (pending, text) = h.m.prompt.settle(Some("notes/todo.md")).unwrap();
        // The directory is there, the file is not: a plain save.
        h.m.answer(pending, text, |p: &str| p.ends_with("/notes"));
        let card = h.m.card(&id).unwrap();
        assert!(card.path.as_deref().unwrap().ends_with("/notes/todo.md"));
        assert!(card.cwd.ends_with("/notes"));
        assert!(h.m.take_effects().iter().any(|e| matches!(
            e,
            Effect::Editor { card_id, action: EditorAction::Save } if card_id == &id
        )));
        // Named now: the next save goes straight through.
        h.run("card.save");
        assert!(!h.m.prompt.is_open());
    }

    #[test]
    fn go_to_line_asks_and_puts_the_number_on_the_card() {
        let mut h = Harness::new();
        h.run("card.new.editor");
        let id = h.focused().id.clone();
        h.run("editor.goToLine");
        assert_eq!(h.m.prompt.label, "go to line");
        let (pending, text) = h.m.prompt.settle(Some(" 42 ")).unwrap();
        h.m.answer(pending, text, |_| true);
        assert_eq!(h.m.card(&id).unwrap().line, Some(42));
        assert!(h.m.take_effects().iter().any(|e| matches!(
            e,
            Effect::Editor {
                action: EditorAction::GoToLine,
                ..
            }
        )));
        // Not a number: nothing happens.
        h.run("editor.goToLine");
        let (pending, text) = h.m.prompt.settle(Some("abc")).unwrap();
        h.m.answer(pending, text, |_| true);
        assert!(!h
            .m
            .take_effects()
            .iter()
            .any(|e| matches!(e, Effect::Editor { .. })));
    }
}
