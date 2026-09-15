//! `ift` verbs, answered from the model because everything they ask about
//! lives here. Port of `iftHandler.ts`. Every verb that performs an action
//! runs the SAME path the palette does; UI actions are not reachable from
//! the CLI except `dev-run`, the harness's handle in development builds.
use super::{Effect, Model};
use crate::cli::{CliReply, CliRequest};
use crate::ift::{diff_plan, format_card_list, open_plan, ListedCard, PathKind};

fn kind_of(s: &str) -> PathKind {
    if s == "directory" {
        PathKind::Directory
    } else {
        PathKind::File
    }
}

impl Model {
    pub fn run_ift(&mut self, req: &CliRequest, command_ids: &[&str]) -> CliReply {
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
            "ls" => {
                let listed: Vec<ListedCard> = self
                    .cards
                    .iter()
                    .map(|c| ListedCard {
                        id: &c.id,
                        cwd: &c.cwd,
                        group_id: c.group_id.as_deref(),
                        agent: match c.agent {
                            crate::agent_state::AgentState::None => "none",
                            crate::agent_state::AgentState::Working => "working",
                            crate::agent_state::AgentState::Idle => "idle",
                        },
                        remote: c.remote.as_deref(),
                    })
                    .collect();
                ok(format_card_list(
                    &listed,
                    |id| self.group(id).map(|g| g.name.clone()),
                    &self.home,
                ))
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
                // duplicated: two groups called humbl.ai is never what anyone
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
            "dev-run" => {
                if !self.dev_build {
                    return err("not a development build");
                }
                let id = req.args.first().map(String::as_str).unwrap_or("");
                if !command_ids.contains(&id) {
                    return err(&format!("no command {id}"));
                }
                self.effects.push(Effect::RunCommand(id.to_string()));
                ok(String::new())
            }
            other => err(&format!("unknown command: {other}")),
        }
    }
}
