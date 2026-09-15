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

    #[test]
    fn a_new_card_inherits_the_group_and_the_directory_the_ungrouped_one_only_the_directory() {
        let mut h = Harness::new();
        let first = h.focused().id.clone();
        h.m.card_mut(&first).unwrap().cwd = "/Users/me/Code/api".into();
        let g = h.m.add_group("api");
        h.m.card_mut(&first).unwrap().group_id = Some(g.clone());
        h.run("card.new.terminal");
        assert_eq!(h.focused().group_id.as_deref(), Some(g.as_str()));
        assert_eq!(h.focused().cwd, "/Users/me/Code/api");
        h.m.set_focus(Some(&first));
        h.run("card.new.ungrouped");
        assert_eq!(h.focused().group_id, None);
        assert_eq!(h.focused().cwd, "/Users/me/Code/api");
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
        assert_eq!(h.focused().agent, crate::agent_state::AgentState::Idle);
        assert_eq!(h.focused().notified_at, h.m.now_ms);
        h.m.apply_hook(&report("UserPromptSubmit"));
        h.m.tick(h.m.now_ms + 61_000.);
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
}
