//! `ift` verbs, answered from the model because everything they ask about
//! lives here. Port of `iftHandler.ts`. Every verb that performs an action
//! runs the SAME path the palette does; UI actions are not reachable from
//! the CLI except `dev-run`, the harness's handle in development builds.
use super::{Effect, Model};
use crate::cli::{CliReply, CliRequest};
use crate::ift::{
    diff_plan, format_agents, format_card_list, open_plan, parse_card_ref, parse_read, parse_send,
    CardRef, ListedCard, PathKind,
};
use crate::omni::OmniAction;

fn kind_of(s: &str) -> PathKind {
    if s == "directory" {
        PathKind::Directory
    } else {
        PathKind::File
    }
}

impl Model {
    /// The id of the card a script named by number or id.
    pub(super) fn resolve_card_ref(&self, arg: &str) -> Option<String> {
        match parse_card_ref(arg)? {
            CardRef::Number(n) => self.cards.iter().find(|c| c.number == n),
            CardRef::Id(id) => self.card(&id),
        }
        .map(|c| c.id.clone())
    }

    /// The live pane of a terminal card, or why there is none.
    fn terminal_pane(&self, arg: &str) -> Result<crate::backend::PaneId, String> {
        let id = self
            .resolve_card_ref(arg)
            .ok_or_else(|| format!("no card {arg}"))?;
        let card = self.card(&id).ok_or_else(|| format!("no card {arg}"))?;
        if card.kind != crate::saved_layout::CardKind::Terminal {
            return Err("that card is not a terminal".into());
        }
        card.pane_id
            .ok_or_else(|| "that card has no running shell".into())
    }

    /// `commands` is the registry as `(id, label)`, since the model does
    /// not hold the registry: `ift commands` lists it and `dev-run` checks it.
    pub fn run_ift(&mut self, req: &CliRequest, commands: &[(&str, &str)]) -> CliReply {
        let ok = |text: String| CliReply { ok: true, text };
        let err = |text: &str| CliReply {
            ok: false,
            text: text.to_string(),
        };
        // The card that ran `ift`. Absent when run from a terminal outside
        // infiniterm, which `ls` and `open` do not mind and the rest do.
        let from = req
            .card_id
            .as_deref()
            .filter(|id| self.card(id).is_some())
            .map(String::from);
        match req.cmd.as_str() {
            // `ift ls --agents`: the cards that have an agent, with the session
            // id a script needs to bring it back (`claude --resume <id>`).
            "ls" if req.args.iter().any(|a| a == "--agents") => {
                let rows: Vec<(u32, &str, &str, &str, &str)> = self
                    .cards
                    .iter()
                    .filter(|c| c.agent_kind.is_some() || c.agent_session.is_some())
                    .map(|c| {
                        (
                            c.number,
                            c.agent_kind.as_deref().unwrap_or("claude"),
                            c.agent_session.as_deref().unwrap_or("-"),
                            c.agent.name(),
                            c.cwd.as_str(),
                        )
                    })
                    .collect();
                ok(format_agents(&rows))
            }
            "ls" => {
                let listed: Vec<ListedCard> = self
                    .cards
                    .iter()
                    .map(|c| ListedCard {
                        id: &c.id,
                        cwd: &c.cwd,
                        group_id: c.group_id.as_deref(),
                        agent: c.agent.name(),
                        remote: c.remote.as_deref(),
                        number: c.number,
                    })
                    .collect();
                ok(format_card_list(
                    &listed,
                    |id| self.group(id).map(|g| g.name.clone()),
                    &self.home,
                ))
            }
            // `ift commands`: every registered command with its chord, the
            // list keybindings.default.json only shows the bound part of.
            // From the running app, so it cannot drift from the palette.
            // `ift usage [days]`: what you ran, and what you never did.
            "usage" => {
                let days = req.args.first().and_then(|d| d.parse().ok()).unwrap_or(30);
                let text =
                    std::fs::read_to_string(crate::paths::usage_log_path()).unwrap_or_default();
                ok(crate::usage_log::report(
                    &text,
                    self.now_ms as u64,
                    days,
                    commands,
                ))
            }
            "commands" => {
                let mut rows: Vec<(String, String, String)> = commands
                    .iter()
                    .map(|(id, label)| {
                        let chord = self
                            .keymap
                            .iter()
                            .find(|(_, cid)| cid == id)
                            .map(|(chord, _)| crate::shortcuts::format_chord(chord))
                            .unwrap_or_default();
                        (id.to_string(), label.to_string(), chord)
                    })
                    .collect();
                rows.sort();
                ok(rows
                    .iter()
                    .map(|(id, label, chord)| format!("{id}\t{label}\t{chord}"))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            // `ift omni <term>`: what Cmd+L would show, ranked against the
            // live history and the cards actually open, so a result nobody
            // can explain has a repro that is not a screenshot. Reads only.
            "omni" => {
                let term = req.args.join(" ");
                let saved = self.omni.query.clone();
                self.omni.query = term;
                let response = self.omni_response();
                self.omni.query = saved;
                ok(render_omni(&response))
            }
            "open" => {
                let path = req.args.first().map(String::as_str).unwrap_or("");
                let kind = kind_of(req.args.get(1).map(String::as_str).unwrap_or("file"));
                let line = req.args.get(2).and_then(|l| l.parse().ok());
                match self.open_in_card(open_plan(path, kind, line), from.as_deref()) {
                    Some(id) => ok(id),
                    None => err(&self.notice.clone().unwrap_or_default()),
                }
            }
            // `ift <file>` from inside a terminal card: the file opens in
            // place over that card (cover.rs) and this request is answered
            // when it closes, which is what `ift` is waiting for. From
            // anywhere else there is no card to cover, so it opens a card.
            "edit" => {
                let path = req.args.first().map(String::as_str).unwrap_or("");
                let line = req.args.get(1).and_then(|l| l.parse().ok());
                let base = from.as_deref().filter(|id| {
                    self.card(id)
                        .is_some_and(|c| c.kind == crate::saved_layout::CardKind::Terminal)
                });
                match base.map(String::from) {
                    Some(base) => match self.open_cover(path, line, &base, req.id) {
                        Some(_) => {
                            self.reply_deferred = true;
                            ok(String::new())
                        }
                        None => err("could not open it here"),
                    },
                    None => match self.open_in_card(
                        open_plan(path, PathKind::File, line.map(|l| l as f64)),
                        from.as_deref(),
                    ) {
                        Some(id) => ok(id),
                        None => err(&self.notice.clone().unwrap_or_default()),
                    },
                }
            }
            "diff" => {
                let path = req.args.first().map(String::as_str).unwrap_or("");
                let kind = kind_of(req.args.get(1).map(String::as_str).unwrap_or("directory"));
                match self.open_in_card(diff_plan(path, kind), from.as_deref()) {
                    Some(id) => ok(id),
                    None => err(&self.notice.clone().unwrap_or_default()),
                }
            }
            "name" => {
                let Some(from) = from else {
                    return err("not running inside an infiniterm card");
                };
                let name = req.args.join(" ").trim().to_string();
                if name.is_empty() {
                    return err("nothing to name it");
                }
                if let Some(c) = self.card_mut(&from) {
                    c.title = name;
                }
                self.dirty_layout = true;
                ok(String::new())
            }
            "group" => {
                let Some(from) = from else {
                    return err("not running inside an infiniterm card");
                };
                let name = req.args.join(" ").trim().to_string();
                if name.is_empty() {
                    return err("nothing to call the group");
                }
                // An existing group of that name is JOINED rather than
                // duplicated: two groups called acme.dev is never what anyone
                // typing this meant.
                let existing = self
                    .groups
                    .iter()
                    .find(|g| g.name == name)
                    .map(|g| g.id.clone());
                let group_id = existing.clone().unwrap_or_else(|| self.add_group(&name));
                if let Some(c) = self.card_mut(&from) {
                    c.group_id = Some(group_id.clone());
                }
                if existing.is_none() {
                    self.move_to_free_block(std::slice::from_ref(&from));
                }
                self.prune_empty_groups();
                self.dirty_layout = true;
                ok(group_id)
            }
            // `ift send <card> text [--enter] [--key NAME]`: types into a
            // card's shell, as keys, without moving the focus or the view.
            "send" => {
                let (target, bytes) = match parse_send(&req.args) {
                    Ok(p) => p,
                    Err(e) => return err(&e),
                };
                match self.terminal_pane(&target) {
                    Ok(pane) => {
                        self.effects.push(Effect::WritePane(pane, bytes));
                        ok(String::new())
                    }
                    Err(e) => err(&e),
                }
            }
            // `ift read <card> [--lines N] [--all]`: what the card shows, or
            // its history too. The ui builds the text, so the reply waits.
            "read" => {
                let (target, last, scrollback) = match parse_read(&req.args) {
                    Ok(p) => p,
                    Err(e) => return err(&e),
                };
                match self.resolve_card_ref(&target) {
                    Some(id) if self.terminal_pane(&target).is_ok() => {
                        self.effects.push(Effect::ReadCard {
                            request_id: req.id,
                            card_id: id,
                            last,
                            scrollback,
                        });
                        self.reply_deferred = true;
                        ok(String::new())
                    }
                    Some(_) => err("that card has no terminal to read"),
                    None => err(&format!("no card {target}")),
                }
            }
            // `ift close <card>`: Cmd+W for that card, parking and the
            // protected lock included.
            "close" => {
                let Some(target) = req.args.first() else {
                    return err("close takes a card");
                };
                let Some(id) = self.resolve_card_ref(target) else {
                    return err(&format!("no card {target}"));
                };
                if self.card(&id).is_some_and(|c| c.protected) {
                    return err("card is locked: Cmd+Shift+L to unlock it");
                }
                self.close_card_with(&id, false, true);
                ok(String::new())
            }
            // `ift run <command-id>`: any registered command, as the palette
            // would run it on the card in focus (`ift commands` lists them).
            "run" => {
                let id = req.args.first().map(String::as_str).unwrap_or("");
                if !commands.iter().any(|(i, _)| *i == id) {
                    return err(&format!("no command {id}"));
                }
                self.effects.push(Effect::RunCommand(id.to_string()));
                ok(String::new())
            }
            "dev-run" => {
                if !self.dev_build {
                    return err("not a development build");
                }
                let id = req.args.first().map(String::as_str).unwrap_or("");
                if !commands.iter().any(|(i, _)| *i == id) {
                    return err(&format!("no command {id}"));
                }
                self.effects.push(Effect::RunCommand(id.to_string()));
                ok(String::new())
            }
            other => err(&format!("unknown command: {other}")),
        }
    }
}

/// One heading per line, its results indented, each with the action spelled
/// out: "why did Enter go there" is the question this answers.
fn render_omni(response: &crate::omni::rank::OmniResponse) -> String {
    let mut out = String::new();
    if let Some(c) = &response.completion {
        out.push_str(&format!("completion\t{c}\n"));
    }
    if let Some((_, name)) = &response.offer {
        out.push_str(&format!("tab\tsearch {name}\n"));
    }
    for section in &response.sections {
        out.push_str(&format!("\n{}\n", section.heading));
        for r in &section.results {
            let action = match &r.action {
                OmniAction::Navigate(url) => format!("open {url}"),
                OmniAction::Search(q) => format!("search {q}"),
                OmniAction::FocusCard(id) => format!("focus {id}"),
            };
            out.push_str(&format!("  {}\t{}\n", r.title, action));
        }
    }
    if response.sections.is_empty() {
        out.push_str("\nnothing\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Model;

    // `ift omni` must not disturb a box somebody has open: it borrows the
    // query, ranks, and puts the old one back.
    #[test]
    fn omni_ranks_a_term_without_touching_the_open_box() {
        let mut m = Model::new();
        m.history.record("https://news.ycombinator.com", 1.);
        m.history
            .set_title("https://news.ycombinator.com", "Hacker News");
        m.omni.query = "left alone".into();
        let reply = m.run_ift(
            &CliRequest {
                id: 1,
                cmd: "omni".into(),
                args: vec!["news".into()],
                card_id: None,
            },
            &[],
        );
        assert!(reply.ok);
        assert!(reply.text.contains("Hacker News"), "{}", reply.text);
        assert!(reply.text.contains("open https://news.ycombinator.com"));
        assert_eq!(m.omni.query, "left alone");
    }

    #[test]
    fn commands_lists_every_registered_command_with_its_chord() {
        let mut m = Model::new();
        m.keymap = vec![("cmd+t".into(), "card.new.terminal".into())];
        let reply = m.run_ift(
            &CliRequest {
                id: 1,
                cmd: "commands".into(),
                args: vec![],
                card_id: None,
            },
            &[
                ("card.new.terminal", "Card: new terminal"),
                ("app.keycast", "App: keycast"),
            ],
        );
        assert!(reply.ok);
        let lines: Vec<&str> = reply.text.lines().collect();
        assert_eq!(
            lines[0], "app.keycast\tApp: keycast\t",
            "sorted, unbound is blank"
        );
        assert!(lines[1].starts_with("card.new.terminal\tCard: new terminal\t"));
        assert!(lines[1].contains('T'), "{}", lines[1]);
    }

    #[test]
    fn omni_says_so_when_nothing_matches() {
        let mut m = Model::new();
        let reply = m.run_ift(
            &CliRequest {
                id: 1,
                cmd: "omni".into(),
                args: vec![],
                card_id: None,
            },
            &[],
        );
        assert!(reply.text.contains("nothing"));
    }

    fn ask(m: &mut Model, cmd: &str, args: &[&str]) -> CliReply {
        m.run_ift(
            &CliRequest {
                id: 7,
                cmd: cmd.into(),
                args: args.iter().map(|a| a.to_string()).collect(),
                card_id: None,
            },
            &[("canvas.zoom.fitAll", "Canvas: fit all cards")],
        )
    }

    /// A model with two terminal cards, the first with a live pane 5.
    fn two_cards() -> (Model, String, String) {
        let mut m = Model::new();
        let ws = m.add_workspace(Some("w"));
        m.show_workspace(&ws);
        let add = |m: &mut Model| {
            m.add_card(
                "/tmp",
                crate::model::NewCard {
                    workspace_id: Some(ws.clone()),
                    ..Default::default()
                },
            )
        };
        let a = add(&mut m);
        let b = add(&mut m);
        m.card_mut(&a).unwrap().pane_id = Some(5);
        (m, a, b)
    }

    // #127: a script types into a card by number or id and the focus stays.
    #[test]
    fn send_writes_keys_to_the_cards_pane() {
        let (mut m, a, _b) = two_cards();
        let number = m.card(&a).unwrap().number;
        m.effects.clear();
        let r = ask(&mut m, "send", &[&format!("#{number}"), "/exit", "--enter"]);
        assert!(r.ok, "{}", r.text);
        assert!(m
            .effects
            .iter()
            .any(|e| matches!(e, Effect::WritePane(5, b) if b == b"/exit\r")));
        let r = ask(&mut m, "send", &[&a, "x"]);
        assert!(r.ok, "an id works too");
    }

    #[test]
    fn send_says_why_it_cannot() {
        let (mut m, _a, b) = two_cards();
        let r = ask(&mut m, "send", &["999", "x"]);
        assert!(!r.ok && r.text.contains("no card 999"), "{}", r.text);
        let r = ask(&mut m, "send", &[&b, "x"]);
        assert!(!r.ok && r.text.contains("no running shell"), "{}", r.text);
        let r = ask(&mut m, "send", &["1"]);
        assert!(!r.ok, "nothing to send");
    }

    // `read` is answered by the ui, so the reply is deferred and an effect
    // carries the request.
    #[test]
    fn read_asks_the_ui_and_defers_the_reply() {
        let (mut m, a, b) = two_cards();
        let number = m.card(&a).unwrap().number;
        m.effects.clear();
        let r = ask(
            &mut m,
            "read",
            &[&number.to_string(), "--lines", "5", "--all"],
        );
        assert!(r.ok);
        assert!(std::mem::take(&mut m.reply_deferred));
        assert!(m.effects.iter().any(|e| matches!(
            e,
            Effect::ReadCard { request_id: 7, card_id, last: Some(5), scrollback: true } if *card_id == a
        )));
        let r = ask(&mut m, "read", &[&b]);
        assert!(!r.ok, "a card with no shell cannot be read");
        assert!(!m.reply_deferred);
    }

    #[test]
    fn close_closes_the_named_card_unless_it_is_locked() {
        let (mut m, a, b) = two_cards();
        m.card_mut(&b).unwrap().protected = true;
        let r = ask(&mut m, "close", &[&b]);
        assert!(!r.ok && r.text.contains("locked"), "{}", r.text);
        assert!(m.card(&b).is_some());
        let r = ask(&mut m, "close", &[&a]);
        assert!(r.ok);
        assert!(m.card(&a).is_none());
        assert!(!ask(&mut m, "close", &["999"]).ok);
    }

    #[test]
    fn run_runs_only_a_registered_command() {
        let (mut m, _a, _b) = two_cards();
        m.effects.clear();
        assert!(ask(&mut m, "run", &["canvas.zoom.fitAll"]).ok);
        assert!(m
            .effects
            .iter()
            .any(|e| matches!(e, Effect::RunCommand(c) if c == "canvas.zoom.fitAll")));
        let r = ask(&mut m, "run", &["nope.nothing"]);
        assert!(!r.ok && r.text.contains("no command"), "{}", r.text);
    }

    #[test]
    fn ls_agents_lists_the_cards_with_a_session_for_a_restart_script() {
        let (mut m, a, _b) = two_cards();
        let number = m.card(&a).unwrap().number;
        {
            let c = m.card_mut(&a).unwrap();
            c.agent_kind = Some("claude".into());
            c.agent_session = Some("abc-123".into());
        }
        let r = ask(&mut m, "ls", &["--agents"]);
        assert!(r.ok);
        let line = r.text.lines().next().unwrap();
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols[0], number.to_string());
        assert_eq!(&cols[1..3], ["claude", "abc-123"]);
        assert_eq!(r.text.lines().count(), 1, "only the card with an agent");
    }
}
