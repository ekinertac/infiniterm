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
    super::groups_cmd::register(r);
    super::dev_cmd::register(r);
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
/// chords are the page's; then the keymap. True when a command ran.
pub fn handle_chord(m: &mut Model, r: &CommandRegistry<Model>, chord: &str) -> bool {
    let kind = m.focused().map(|c| c.kind);
    if kind == Some(crate::saved_layout::CardKind::Editor)
        && crate::editor_keys::editor_keeps(chord)
    {
        return false;
    }
    let override_ = (kind == Some(crate::saved_layout::CardKind::Browser))
        .then(|| crate::browser_keys::browser_override(chord))
        .flatten();
    let Some(id) = override_
        .map(String::from)
        .or_else(|| crate::keymap::lookup(&m.keymap, chord).map(String::from))
    else {
        return false;
    };
    run_with_effects(m, r, &id);
    true
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
            m.home = "/Users/me".into();
            m.start_dir = "/Users/me".into();
            m.now_ms = 1_000_000.;
            m.load_layout(None);
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

    // Focus after a close goes to the NEAREST card by geometry, not the next in the list.
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
        let effects = h.run("card.close");
        assert_eq!(h.focused().id, a);
        assert!(h.m.card(&b).is_none());
        // No pane yet, so nothing to kill; the log line is there.
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::Log(l) if l.starts_with("close "))));
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
        assert!(h.m.handle_bare_key(focus_cmd::BareKey::Char(key_a)));
        assert_eq!(h.focused().id, a);
        assert!(h.m.selection.hints.is_empty());
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
        };
        h.m.apply_hook(&report("PreToolUse"));
        assert_eq!(h.focused().agent, crate::agent_state::AgentState::Working);
        assert_eq!(h.focused().transcript_path.as_deref(), Some("/s/x.jsonl"));
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

    // The keymap dispatches through the editor and browser exceptions.
    #[test]
    fn chords_dispatch_through_the_keymap_with_the_editor_and_browser_exceptions() {
        let mut h = Harness::new();
        assert!(handle_chord(&mut h.m, &h.r, "cmd+t"));
        assert_eq!(h.m.cards.len(), 2);
        assert!(!handle_chord(&mut h.m, &h.r, "cmd+shift+z"), "unbound");
        // An editor keeps Cmd+F; a terminal gives it to hints.
        h.m.take_effects();
        let id = h.focused().id.clone();
        h.m.card_mut(&id).unwrap().kind = CardKind::Editor;
        assert!(!handle_chord(&mut h.m, &h.r, "cmd+f"));
        h.m.card_mut(&id).unwrap().kind = CardKind::Terminal;
        assert!(handle_chord(&mut h.m, &h.r, "cmd+f"));
        assert!(!h.m.selection.hints.is_empty());
        h.m.selection.hints.clear();
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

    // A dirty editor closes on the second press, within the window.
    #[test]
    fn a_dirty_editor_takes_two_presses_to_close() {
        let mut h = Harness::new();
        let id = h.focused().id.clone();
        let c = h.m.card_mut(&id).unwrap();
        c.kind = CardKind::Editor;
        c.dirty = true;
        h.run("card.close");
        assert_eq!(h.m.cards.len(), 1);
        assert!(h.m.notice.as_deref().unwrap().contains("unsaved"));
        let effects = h.run("card.close");
        assert!(h.m.cards.is_empty());
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
        h.run("browser.find");
        assert_eq!(h.m.notice.as_deref(), Some("not a browser card"));
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
        h.m.palette_preview(Source::Themes, Some("A"));
        assert!(h.m.take_effects().contains(&Effect::LoadTheme("A".into())));
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
        assert_eq!(h.m.usage.len(), 1);
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
            !Source::Themes.keeps_its_order(),
            "hundreds of themes: recency helps"
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
        assert!(row.label.starts_with("Card: embers"), "{}", row.label);
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
        h.m.answer(pending, text, |_| true);
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
