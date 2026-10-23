//! `ift` verbs, answered from the model because everything they ask about
//! lives here. Port of `iftHandler.ts`. Every verb that performs an action
//! runs the SAME path the palette does; UI actions are not reachable from
//! the CLI except `dev-run`, the harness's handle in development builds.
use super::{Effect, Model};
use crate::cli::{CliReply, CliRequest};
use crate::ift::{
    diff_plan, format_agents, format_card_list, format_workspace_list, open_plan, parse_card_ref,
    parse_read, parse_send, parse_terminal_args, resolve_workspace, CardRef, ListedCard, PathKind,
    WorkspaceTarget,
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
            // `ift workspaces`: the tabs left to right, with the one on screen
            // marked, for a launcher that asks where to open a folder.
            "workspaces" => {
                let rows: Vec<(&str, &str, usize, bool)> = self
                    .workspaces
                    .iter()
                    .map(|w| {
                        (
                            w.id.as_str(),
                            w.name.as_str(),
                            self.cards_on(Some(&w.id)).len(),
                            self.active_workspace.as_deref() == Some(w.id.as_str()),
                        )
                    })
                    .collect();
                ok(format_workspace_list(&rows))
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
                            .map(|(chord, _)| {
                                crate::shortcuts::format_chord(chord, self.modifier_labels())
                            })
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
            // `ift -t <file> [card]`: the file as a tab of an editor card (#265).
            "tab" => {
                let path = req.args.first().map(String::as_str).unwrap_or("");
                let target = req
                    .args
                    .get(1)
                    .map(String::as_str)
                    .filter(|t| !t.is_empty());
                match self.open_file_tab(path, target, from.as_deref()) {
                    Ok(id) => ok(id),
                    Err(e) => err(&e),
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
            // `ift focus <card>`: the palette's "go to that card" (#552):
            // its workspace is shown if it is another, the card is focused
            // and fitted, so the result is the same however you got there.
            "focus" => {
                let Some(target) = req.args.first() else {
                    return err("focus takes a card");
                };
                let Some(id) = self.resolve_card_ref(target) else {
                    return err(&format!("no card {target}"));
                };
                self.go_to_card(&id);
                ok(String::new())
            }
            // `ift terminal <dir>...`: a terminal card in each directory, on
            // the workspace on screen, the last one focused (the Finder
            // service's path, from a script). Absolute paths only: the cli
            // resolves them, the app's own working directory means nothing.
            // `--workspace <id|name|new> [--name <text>] [--]` first picks
            // the workspace and shows it; everything is checked before
            // anything is made, so an error leaves nothing behind.
            "terminal" => {
                let parsed = match parse_terminal_args(&req.args) {
                    Ok(p) => p,
                    Err(e) => return err(&e),
                };
                if parsed.dirs.is_empty() {
                    return err("terminal takes a directory");
                }
                let known: Vec<(&str, &str)> = self
                    .workspaces
                    .iter()
                    .map(|w| (w.id.as_str(), w.name.as_str()))
                    .collect();
                let target = match parsed.workspace.map(|v| resolve_workspace(&known, v)) {
                    Some(Ok(t)) => Some(t),
                    Some(Err(e)) => return err(&e),
                    None => None,
                };
                if parsed.name.is_some() && target != Some(WorkspaceTarget::New) {
                    return err("--name goes with --workspace new");
                }
                // A workspace made for directories that then fail would stay
                // behind empty, so the checks open_terminal_at makes run first.
                for dir in &parsed.dirs {
                    let path = std::path::Path::new(dir);
                    if !path.is_absolute() {
                        return err(&format!("not an absolute path: {dir}"));
                    }
                    if !path.is_dir() {
                        return err(&format!("not a directory: {dir}"));
                    }
                }
                match target {
                    Some(WorkspaceTarget::Existing(id)) => self.show_workspace(&id),
                    Some(WorkspaceTarget::New) => {
                        let id = self.add_workspace(parsed.name);
                        self.show_workspace(&id);
                    }
                    None => {}
                }
                let mut made = Vec::new();
                for dir in &parsed.dirs {
                    match self.open_terminal_at(dir) {
                        Ok(id) => made.push(id),
                        Err(e) => return err(&e),
                    }
                }
                ok(made.join("\n"))
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

    // #265: the `tab` verb opens a file as a tab of an editor card and says
    // why it cannot.
    #[test]
    fn tab_opens_a_file_in_an_editor_card_and_says_why_it_cannot() {
        let (mut m, a, _b) = two_cards();
        let number = m.card(&a).unwrap().number;
        // `a` is a terminal: refused by number.
        let r = ask(&mut m, "tab", &["/tmp/x.rs", &number.to_string()]);
        assert!(!r.ok && r.text.contains("not an editor"), "{}", r.text);
        // With no editor focused, a card of its own; then a tab of it.
        let r = ask(&mut m, "tab", &["/tmp/x.rs"]);
        assert!(r.ok, "{}", r.text);
        let editor = r.text.clone();
        let r = ask(&mut m, "tab", &["/tmp/y.rs", ""]);
        assert!(r.ok && r.text == editor, "{}", r.text);
        assert!(m
            .card(&editor)
            .unwrap()
            .tabs
            .contains(&"/tmp/y.rs".to_string()));
        let r = ask(&mut m, "tab", &["/tmp/z.rs", "999"]);
        assert!(!r.ok && r.text.contains("no card 999"), "{}", r.text);
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

    // `ift terminal`: a card per directory, and a refusal says why and makes
    // nothing.
    #[test]
    fn terminal_opens_a_card_per_directory() {
        let (mut m, _a, _b) = two_cards();
        let before = m.cards.len();
        // Two directories that exist on any platform: `/tmp` is not one on
        // Windows, and `/` there has no drive, so it is not absolute.
        let tmp = std::env::temp_dir().to_string_lossy().into_owned();
        let here = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let r = ask(&mut m, "terminal", &[tmp.as_str(), here.as_str()]);
        assert!(r.ok, "{}", r.text);
        assert_eq!(m.cards.len(), before + 2);
        let r = ask(&mut m, "terminal", &["relative"]);
        assert!(!r.ok);
        assert!(!ask(&mut m, "terminal", &[]).ok);
        assert_eq!(m.cards.len(), before + 2);
    }

    /// Two workspaces, "a" on screen and "b" behind it, plus a directory that exists.
    fn two_ws() -> (Model, String, String, String) {
        let mut m = Model::new();
        let a = m.add_workspace(Some("a"));
        let b = m.add_workspace(Some("b"));
        m.show_workspace(&a);
        let dir = std::env::temp_dir().canonicalize().unwrap();
        (m, a, b, dir.to_string_lossy().into_owned())
    }

    #[test]
    fn workspaces_lists_tabs_with_card_counts_and_the_one_on_screen() {
        let (mut m, a, b, dir) = two_ws();
        assert!(ask(&mut m, "terminal", &[&dir]).ok);
        let r = ask(&mut m, "workspaces", &[]);
        assert!(r.ok);
        assert_eq!(r.text, format!("{a}\ta\t1\t*\n{b}\tb\t0\t-"));
        m.show_workspace(&b);
        let r = ask(&mut m, "workspaces", &[]);
        assert_eq!(r.text, format!("{a}\ta\t1\t-\n{b}\tb\t0\t*"));
    }

    #[test]
    fn terminal_by_id_or_name_shows_that_workspace_and_opens_there() {
        let (mut m, a, b, dir) = two_ws();
        let r = ask(&mut m, "terminal", &["--workspace", &b, &dir]);
        assert!(r.ok, "{}", r.text);
        assert_eq!(m.active_workspace.as_deref(), Some(b.as_str()));
        assert_eq!(m.card(&r.text).unwrap().workspace_id, b);
        let r = ask(&mut m, "terminal", &["--workspace", "a", &dir]);
        assert!(r.ok, "{}", r.text);
        assert_eq!(m.active_workspace.as_deref(), Some(a.as_str()));
        assert_eq!(m.card(&r.text).unwrap().workspace_id, a);
        assert_eq!(m.focused().map(|c| c.id.clone()), Some(r.text));
    }

    #[test]
    fn terminal_workspace_new_makes_and_names_one() {
        let (mut m, _a, _b, dir) = two_ws();
        let r = ask(
            &mut m,
            "terminal",
            &["--workspace", "new", "--name", "demo", &dir],
        );
        assert!(r.ok, "{}", r.text);
        assert_eq!(m.workspaces.len(), 3);
        let ws = m.workspaces.last().unwrap();
        assert_eq!(ws.name, "demo");
        assert_eq!(m.active_workspace.as_deref(), Some(ws.id.as_str()));
        assert_eq!(m.card(&r.text).unwrap().workspace_id, ws.id);
        // No name: the model's own default.
        assert!(ask(&mut m, "terminal", &["--workspace", "new", &dir]).ok);
        assert_eq!(m.workspaces.len(), 4);
        assert!(m.workspaces.last().unwrap().name.starts_with("workspace "));
    }

    #[test]
    fn terminal_workspace_errors_leave_nothing_behind() {
        let (mut m, a, _b, dir) = two_ws();
        m.workspaces.iter_mut().find(|w| w.id == a).unwrap().name = "b".into();
        let before = (m.workspaces.len(), m.cards.len());
        let r = ask(&mut m, "terminal", &["--workspace", "b", &dir]);
        assert!(!r.ok && r.text.contains("ambiguous"), "{}", r.text);
        let r = ask(&mut m, "terminal", &["--workspace", "zzz", &dir]);
        assert!(!r.ok && r.text.contains("no workspace zzz"), "{}", r.text);
        let r = ask(&mut m, "terminal", &["--name", "x", &dir]);
        assert!(!r.ok && r.text.contains("--name"), "{}", r.text);
        let r = ask(
            &mut m,
            "terminal",
            &["--workspace", &a, "--name", "x", &dir],
        );
        assert!(!r.ok, "{}", r.text);
        // A bad folder must not leave the workspace it was to open in.
        let r = ask(&mut m, "terminal", &["--workspace", "new", "relative"]);
        assert!(!r.ok);
        let r = ask(&mut m, "terminal", &["--workspace", "new"]);
        assert!(!r.ok && r.text.contains("takes a directory"), "{}", r.text);
        assert_eq!((m.workspaces.len(), m.cards.len()), before);
    }

    #[test]
    fn terminal_double_dash_ends_the_flags() {
        let (mut m, _a, _b, dir) = two_ws();
        let r = ask(&mut m, "terminal", &["--", "--workspace"]);
        assert!(
            !r.ok && r.text.contains("not an absolute path: --workspace"),
            "{}",
            r.text
        );
        let r = ask(&mut m, "terminal", &["--workspace", "new", "--", &dir]);
        assert!(r.ok, "{}", r.text);
    }

    /// "a" on screen with one card, "b" behind it with one card.
    fn card_on_each() -> (Model, String, String, String, String) {
        let (mut m, a, b, dir) = two_ws();
        let on_a = ask(&mut m, "terminal", &[&dir]).text;
        let on_b = ask(&mut m, "terminal", &["--workspace", &b, &dir]).text;
        m.show_workspace(&a);
        m.effects.clear();
        (m, a, b, on_a, on_b)
    }

    fn fits(m: &Model) -> usize {
        m.effects
            .iter()
            .filter(|e| matches!(e, Effect::RunCommand(c) if c == "canvas.zoom.fitCard"))
            .count()
    }

    #[test]
    fn focus_goes_to_a_card_on_this_workspace_and_fits_it_once() {
        let (mut m, a, _b, on_a, _on_b) = card_on_each();
        m.set_focus(None);
        let r = ask(&mut m, "focus", &[&on_a]);
        assert!(r.ok && r.text.is_empty(), "{}", r.text);
        assert_eq!(m.focused().map(|c| c.id.clone()), Some(on_a));
        assert_eq!(m.active_workspace.as_deref(), Some(a.as_str()));
        assert_eq!(fits(&m), 1);
    }

    #[test]
    fn focus_switches_to_the_workspace_of_a_card_in_another() {
        // By id, by number and by #number all reach the same card. Each
        // round has a model of its own, so the argument is made from it.
        let spell: [fn(&str, u32) -> String; 3] = [
            |id, _| id.to_string(),
            |_, n| n.to_string(),
            |_, n| format!("#{n}"),
        ];
        for (i, arg_of) in spell.iter().enumerate() {
            let (mut m, _a, b, _on_a, on_b) = card_on_each();
            let arg = arg_of(&on_b, m.card(&on_b).unwrap().number);
            let r = ask(&mut m, "focus", &[&arg]);
            assert!(r.ok, "{i} {arg}: {}", r.text);
            assert_eq!(m.active_workspace.as_deref(), Some(b.as_str()), "{i}");
            assert_eq!(m.focused().map(|c| c.id.clone()), Some(on_b), "{i}");
            assert_eq!(fits(&m), 1, "{i}");
        }
    }

    #[test]
    fn focus_refuses_an_unknown_card_and_changes_nothing() {
        let (mut m, a, _b, _on_a, on_b) = card_on_each();
        let focus = m.focused().map(|c| c.id.clone());
        let r = ask(&mut m, "focus", &["999"]);
        assert!(!r.ok && r.text == "no card 999", "{}", r.text);
        let r = ask(&mut m, "focus", &[]);
        assert!(!r.ok && r.text == "focus takes a card", "{}", r.text);
        assert_eq!(m.active_workspace.as_deref(), Some(a.as_str()));
        assert_eq!(m.focused().map(|c| c.id.clone()), focus);
        assert_eq!(fits(&m), 0);
        // A closed card is gone by its old id.
        assert!(ask(&mut m, "close", &[&on_b]).ok);
        let r = ask(&mut m, "focus", &[&on_b]);
        assert!(!r.ok && r.text.starts_with("no card"), "{}", r.text);
    }

    // The lowest free number is handed out again, so a number names whichever
    // card holds it now, not the one that closed.
    #[test]
    fn focus_by_a_reused_number_goes_to_the_newer_card() {
        let (mut m, _a, _b, on_a, on_b) = card_on_each();
        let number = m.card(&on_b).unwrap().number;
        assert!(ask(&mut m, "close", &[&on_b]).ok);
        let dir = std::env::temp_dir().canonicalize().unwrap();
        let newer = ask(&mut m, "terminal", &[&dir.to_string_lossy()]).text;
        assert_eq!(m.card(&newer).unwrap().number, number);
        m.set_focus(Some(&on_a));
        assert!(ask(&mut m, "focus", &[&number.to_string()]).ok);
        assert_eq!(m.focused().map(|c| c.id.clone()), Some(newer));
    }

    #[test]
    fn terminal_without_flags_stays_on_the_workspace_on_screen() {
        let (mut m, a, _b, dir) = two_ws();
        let r = ask(&mut m, "terminal", &[&dir]);
        assert!(r.ok);
        assert_eq!(m.workspaces.len(), 2);
        assert_eq!(m.card(&r.text).unwrap().workspace_id, a);
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
