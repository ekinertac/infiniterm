//! What each pane is actually doing: its directory, what is running in it, and
//! whether that is an SSH session.
//!
//! Answered from the process table rather than from the terminal title. A title
//! is whatever a shell's config felt like writing — often `user@host`, sometimes
//! nothing — so it says more about someone's dotfiles than about the pane. The
//! process table is the same thing iTerm2 reads, and it is true regardless.
//!
//! Polled rather than pushed: there is no notification when a process starts
//! under a shell. One `ps` covers every pane at once, and one `lsof` with a pid
//! list covers every directory, so the cost does not grow with the number of
//! cards.
//!
//! Everything here follows the FOREGROUND process group, and the name and
//! directory come from that group's LEADER — the command the shell actually
//! started.
//!
//! Not the deepest descendant, which is what this did first and which an agent
//! harness breaks immediately: `claude` spawns its MCP servers as children, they
//! inherit its process group, and the deepest one was a python in
//! `~/Code/blender-mcp`. Every card running an agent reported that as its
//! directory, and new cards inherited it as the place to open.
//!
//! The leader is what a shell means by "the foreground job". The cost is that
//! `sudo vim` reports `sudo` and `zsh -c vim` reports `zsh` — honest, and far
//! rarer than running an agent.
//!
//! SSH detection still scans every foreground descendant, because an `ssh` under
//! a `sudo` is still an ssh session. The foreground test there is what keeps a
//! backgrounded tunnel (`ssh -fN`) from painting a card red for hours.
//!
//! From the Tauri app's inspect.rs unchanged but for the glue: the poller
//! takes the backend's pid source and an mpsc sender. macOS only (`ps` and
//! `lsof`); Linux reads `/proc` and Windows the ConPTY API behind the same
//! `PaneStatus` when they come. `card_label.rs` turns a status into a name.

use crate::backend::PaneId;
use std::collections::HashMap;
use std::sync::mpsc::Sender;

/// One `ps` plus one `lsof` measures ~60ms, so this is about 8% of one core.
///
/// Paid rather than backed off when idle: `cd` is a shell BUILTIN and spawns
/// nothing, so there is no cheap signal that a directory changed — an adaptive
/// interval would be slowest exactly when you had just changed directory, which
/// is the one moment the label matters.
const POLL_MS: u64 = 750;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PaneStatus {
    pub pane: PaneId,
    /// The deepest foreground process that is not the shell itself, by name.
    /// `None` when the pane is sitting at its prompt.
    pub proc: Option<String>,
    /// Where that process is, or where the shell is when nothing is running.
    pub cwd: Option<String>,
    /// The ssh destination when this pane is remote, or just "ssh".
    pub remote: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    /// Process group. A process whose `pid == pgid` leads its group, which for a
    /// foreground group means it is the job the shell started.
    pub pgid: u32,
    /// True when the process is in its terminal's foreground process group.
    pub foreground: bool,
    pub args: String,
}

/// Parses `ps -axo pid=,ppid=,pgid=,stat=,args=`.
///
/// `split_whitespace`, not `splitn(4, char::is_whitespace)`: ps pads its columns,
/// and splitn treats each space in a run as its own separator, so every line came
/// back with empty fields where the padding was and nothing parsed at all.
///
/// Rejoining the remainder with single spaces collapses any run inside the
/// command line too. That is lossless for what this reads — argv[0] and the ssh
/// destination — and no caller sees the string otherwise.
pub fn parse_ps(output: &str) -> Vec<Proc> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let ppid = parts.next()?.parse().ok()?;
            let pgid = parts.next()?.parse().ok()?;
            let stat = parts.next()?;
            let args = parts.collect::<Vec<_>>().join(" ");
            Some(Proc {
                pid,
                ppid,
                pgid,
                // BSD ps marks foreground-process-group members with a trailing
                // `+`, which saves a tcgetpgrp on a raw master fd.
                foreground: stat.contains('+'),
                args,
            })
        })
        .collect()
}

fn is_ssh(args: &str) -> bool {
    // argv[0] only. A command merely MENTIONING ssh — `git push`, `vim
    // ~/.ssh/config` — is not an ssh session, and matching anywhere in the line
    // would paint those red.
    let argv0 = args.split_whitespace().next().unwrap_or("");
    let name = crate::paths::base_name(argv0);
    name == "ssh"
}

/// The destination from an ssh command line, if one can be picked out.
///
/// Deliberately rough. It skips flags and the values of the flags that take one,
/// then takes the first thing left. Anything it cannot make sense of falls back
/// to a bare "ssh" at the call site, which is still the information that matters.
pub fn ssh_destination(args: &str) -> Option<String> {
    // The short options that consume the following argument, so a port number or
    // an identity file is never mistaken for the host.
    const TAKES_VALUE: &str = "bcDEeFIiJLlmOopQRSWw";
    let mut words = args.split_whitespace().skip(1);
    while let Some(word) = words.next() {
        if let Some(flags) = word.strip_prefix('-') {
            // Bundled flags: only the LAST letter can consume a value (-tp 22).
            if flags
                .chars()
                .last()
                .is_some_and(|c| TAKES_VALUE.contains(c))
            {
                words.next();
            }
            continue;
        }
        return Some(word.to_string());
    }
    None
}

/// The basename of a command line's argv[0], with a login shell's leading dash
/// removed (`-zsh` is how a login shell names itself, not a flag).
pub fn proc_name(args: &str) -> String {
    let argv0 = args.split_whitespace().next().unwrap_or("");
    let base = crate::paths::base_name(argv0);
    base.trim_start_matches('-').to_string()
}

/// What each pane is doing, given its shell pid and the process table.
///
/// Returns each status WITH the pid whose directory it should report, rather than
/// leaving the caller to compute a parallel array. It was a parallel array, and
/// the two fell out of step the moment this function started sorting its output:
/// the pids came back in the pane map's arbitrary order, so every card was shown
/// another card's directory.
///
/// Walks DOWN from each shell rather than up from every process: a shell has a
/// handful of descendants while the table has hundreds.
///
/// Follows the FOREGROUND group throughout, and reports the DEEPEST foreground
/// process rather than the first. `zsh -c 'cd x && vim'` and a shell function
/// that wraps a command both nest, and the outer layer is the uninteresting one —
/// you want to be told `vim`, not `zsh`.
pub fn pane_statuses(shells: &[(u32, u32)], procs: &[Proc]) -> Vec<(PaneStatus, u32)> {
    let mut children: HashMap<u32, Vec<&Proc>> = HashMap::new();
    for p in procs {
        children.entry(p.ppid).or_default().push(p);
    }

    let mut out = Vec::new();
    for (pane, shell) in shells {
        // The shallowest foreground process that LEADS its group: the job the
        // shell started, rather than whatever that job spawned underneath.
        let mut job: Option<(usize, &Proc)> = None;
        // Kept as a fallback for a foreground tree with no leader in it at all.
        let mut deepest: Option<(usize, &Proc)> = None;
        let mut remote = None;
        // (pid, depth). Bounded so a pid cycle in a malformed table cannot spin.
        let mut stack = vec![(*shell, 0usize)];
        let mut seen = 0;
        while let Some((pid, depth)) = stack.pop() {
            seen += 1;
            if seen > 500 {
                break;
            }
            let Some(kids) = children.get(&pid) else {
                continue;
            };
            for kid in kids {
                if !kid.foreground {
                    continue;
                }
                if remote.is_none() && is_ssh(&kid.args) {
                    remote = Some(
                        ssh_destination(&kid.args)
                            .map(|d| format!("ssh {d}"))
                            .unwrap_or_else(|| "ssh".to_string()),
                    );
                }
                if kid.pid == kid.pgid && job.is_none_or(|(d, _)| depth + 1 < d) {
                    job = Some((depth + 1, kid));
                }
                if deepest.is_none_or(|(d, _)| depth + 1 > d) {
                    deepest = Some((depth + 1, kid));
                }
                stack.push((kid.pid, depth + 1));
            }
        }

        let reported = job.or(deepest);
        out.push((
            PaneStatus {
                pane: *pane,
                proc: reported.map(|(_, p)| proc_name(&p.args)),
                // Filled in by the caller, which batches one lsof for every pane.
                cwd: None,
                remote,
            },
            // The foreground job's directory, or the shell's when nothing is
            // running — so a `cd` still shows.
            reported.map_or(*shell, |(_, p)| p.pid),
        ));
    }
    out.sort_by_key(|(r, _)| r.pane);
    out
}

/// Parses `lsof -a -d cwd -Fpn`: `p<pid>` lines followed by `n<path>` lines.
///
/// One call for every pane rather than one per pane. lsof accepts a comma
/// separated pid list, which turns twenty forks per poll into one — and lsof is
/// the only way to read another process's directory on macOS without a native
/// dependency.
pub fn parse_lsof(output: &str) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let mut pid = None;
    for line in output.lines() {
        let (tag, rest) = line.split_at(line.char_indices().nth(1).map_or(line.len(), |(i, _)| i));
        match tag {
            "p" => pid = rest.parse::<u32>().ok(),
            "n" if rest.starts_with('/') => {
                if let Some(p) = pid {
                    out.entry(p).or_insert_with(|| rest.to_string());
                }
            }
            _ => {}
        }
    }
    out
}

fn cwds(pids: &[u32]) -> HashMap<u32, String> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    std::process::Command::new("lsof")
        .args(["-a", "-d", "cwd", "-p", &list, "-Fpn"])
        .output()
        .ok()
        .map(|o| parse_lsof(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

fn snapshot() -> Vec<Proc> {
    std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,pgid=,stat=,args="])
        .output()
        .ok()
        .map(|o| parse_ps(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// Sends the status of every pane whenever any of it changes.
///
/// The WHOLE list each time rather than deltas, so the consumer can assign
/// it wholesale and a missed message cannot leave a card showing a directory
/// it left. Only sent on change, so an idle canvas costs nothing. The thread
/// ends when the receiver is dropped.
pub fn pane_status_poll(
    shell_pids: impl Fn() -> Vec<(PaneId, u32)> + Send + 'static,
    tx: Sender<Vec<PaneStatus>>,
) {
    std::thread::spawn(move || {
        let mut last: Option<Vec<PaneStatus>> = None;
        loop {
            let shells = shell_pids();
            let reports = if shells.is_empty() {
                Vec::new()
            } else {
                let procs = snapshot();
                let mut reports = pane_statuses(&shells, &procs);
                let dirs = cwds(&reports.iter().map(|(_, pid)| *pid).collect::<Vec<_>>());
                for (report, pid) in reports.iter_mut() {
                    report.cwd = dirs.get(pid).cloned();
                }
                reports.into_iter().map(|(r, _)| r).collect()
            };
            if last.as_ref() != Some(&reports) {
                if tx.send(reports.clone()).is_err() {
                    break;
                }
                last = Some(reports);
            }
            std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A group leader by default, which is what a shell's foreground job is.
    fn proc(pid: u32, ppid: u32, foreground: bool, args: &str) -> Proc {
        Proc {
            pid,
            ppid,
            pgid: pid,
            foreground,
            args: args.to_string(),
        }
    }

    /// A child of a job: same group, not the leader. An agent's MCP servers look
    /// like this, and following them is what put every card in ~/Code/blender-mcp.
    fn child_of(pid: u32, parent: &Proc, args: &str) -> Proc {
        Proc {
            pid,
            ppid: parent.pid,
            pgid: parent.pgid,
            foreground: true,
            args: args.to_string(),
        }
    }

    #[test]
    fn parses_real_ps_output() {
        let out =
            "    1     0     1 Ss   /sbin/launchd\n 4242  4200  4242 S+   ssh -p 2222 me@box\n";
        assert_eq!(
            parse_ps(out),
            vec![
                proc(1, 0, false, "/sbin/launchd"),
                proc(4242, 4200, true, "ssh -p 2222 me@box"),
            ]
        );
    }

    #[test]
    fn ignores_lines_that_are_not_processes() {
        assert!(parse_ps("PID PPID PGID STAT ARGS\n\n  nonsense\n").is_empty());
    }

    /// A command merely mentioning ssh is not an ssh session.
    #[test]
    fn only_argv0_counts_as_ssh() {
        assert!(is_ssh("ssh box"));
        assert!(is_ssh("/usr/bin/ssh box"));
        assert!(!is_ssh("git push origin main"));
        assert!(!is_ssh("vim /Users/me/.ssh/config"));
        assert!(!is_ssh("sshd: me [priv]"));
    }

    #[test]
    fn finds_the_destination_past_the_flags() {
        assert_eq!(ssh_destination("ssh box"), Some("box".into()));
        assert_eq!(ssh_destination("ssh -p 2222 me@box"), Some("me@box".into()));
        assert_eq!(
            ssh_destination("ssh -i ~/.ssh/id_ed25519 -A box"),
            Some("box".into())
        );
        assert_eq!(ssh_destination("ssh -tp 22 box"), Some("box".into()));
        assert_eq!(ssh_destination("ssh box uptime"), Some("box".into()));
        assert_eq!(ssh_destination("ssh"), None);
    }

    #[test]
    fn reports_a_pane_whose_shell_is_running_ssh() {
        let procs = vec![proc(200, 100, true, "ssh box")];
        let out: Vec<_> = pane_statuses(&[(7, 100)], &procs)
            .into_iter()
            .map(|(r, _)| r)
            .collect();
        assert_eq!(out[0].remote.as_deref(), Some("ssh box"));
        assert_eq!(out[0].proc.as_deref(), Some("ssh"));
    }

    /// ssh under a sudo under the shell still counts.
    #[test]
    fn walks_the_whole_descendant_tree() {
        let procs = vec![
            proc(200, 100, true, "sudo -i"),
            proc(300, 200, true, "ssh box"),
        ];
        assert_eq!(
            pane_statuses(&[(7, 100)], &procs)[0].0.remote.as_deref(),
            Some("ssh box")
        );
    }

    /// `ssh -fN` tunnels would otherwise paint a card red for hours.
    #[test]
    fn ignores_a_backgrounded_ssh() {
        let procs = vec![proc(200, 100, false, "ssh -fN -L 5432:localhost:5432 box")];
        assert_eq!(pane_statuses(&[(7, 100)], &procs)[0].0.remote, None);
    }

    #[test]
    fn ignores_another_panes_processes() {
        let procs = vec![proc(200, 999, true, "ssh box")];
        let out: Vec<_> = pane_statuses(&[(7, 100)], &procs)
            .into_iter()
            .map(|(r, _)| r)
            .collect();
        assert_eq!(out[0].remote, None);
        assert_eq!(out[0].proc, None);
    }

    #[test]
    fn falls_back_to_a_bare_label_when_the_destination_is_unreadable() {
        let procs = vec![proc(200, 100, true, "ssh -p")];
        assert_eq!(
            pane_statuses(&[(7, 100)], &procs)[0].0.remote.as_deref(),
            Some("ssh")
        );
    }

    /// A pane sitting at its prompt is running nothing worth naming.
    #[test]
    fn reports_no_process_for_an_idle_shell() {
        assert_eq!(pane_statuses(&[(7, 100)], &[])[0].0.proc, None);
    }

    /// The bug this exists for: `claude` spawns MCP servers as children in other
    /// directories, they inherit its process group, and following the deepest one
    /// reported every agent card as living in ~/Code/blender-mcp.
    #[test]
    fn reports_the_job_the_shell_started_not_what_it_spawned() {
        let claude = proc(200, 100, true, "claude --dangerously-skip-permissions");
        let uv = child_of(
            300,
            &claude,
            "uv --directory /Users/me/Code/blender-mcp run blender-mcp",
        );
        let py = child_of(
            400,
            &uv,
            "/Users/me/Code/blender-mcp/.venv/bin/python blender-mcp",
        );
        let out = pane_statuses(&[(7, 100)], &[claude, uv, py]);
        assert_eq!(out[0].0.proc.as_deref(), Some("claude"));
        assert_eq!(
            out[0].1, 200,
            "the directory must come from claude, not its MCP server"
        );
    }

    /// A foreground tree with no leader under the shell still reports something
    /// rather than falling back to the shell's own directory.
    #[test]
    fn falls_back_to_the_deepest_when_no_child_leads_its_group() {
        let shellish = Proc {
            pid: 200,
            ppid: 100,
            pgid: 100,
            foreground: true,
            args: "odd".into(),
        };
        assert_eq!(pane_statuses(&[(7, 100)], &[shellish])[0].1, 200);
    }

    #[test]
    fn a_backgrounded_process_does_not_name_the_pane() {
        let procs = vec![proc(200, 100, false, "npm run watch")];
        assert_eq!(pane_statuses(&[(7, 100)], &procs)[0].0.proc, None);
    }

    /// A login shell names itself `-zsh`; the dash is not a flag.
    #[test]
    fn proc_name_strips_paths_and_the_login_dash() {
        assert_eq!(proc_name("/usr/bin/vim a.rs"), "vim");
        assert_eq!(proc_name("-zsh"), "zsh");
        assert_eq!(proc_name(""), "");
    }

    /// A malformed table with a pid cycle must not spin forever.
    #[test]
    fn survives_a_cycle_in_the_process_table() {
        let procs = vec![proc(100, 200, true, "a"), proc(200, 100, true, "b")];
        assert!(!pane_statuses(&[(7, 100)], &procs).is_empty());
    }

    #[test]
    fn parses_lsof_pid_and_path_records() {
        let out = "p981\nfcwd\nn/Users/me/Code/api\np3862\nfcwd\nn/tmp\n";
        let dirs = parse_lsof(out);
        assert_eq!(
            dirs.get(&981).map(String::as_str),
            Some("/Users/me/Code/api")
        );
        assert_eq!(dirs.get(&3862).map(String::as_str), Some("/tmp"));
    }

    #[test]
    fn ignores_lsof_records_that_are_not_paths() {
        assert!(parse_lsof("p1\nfcwd\nnsomething odd\n").is_empty());
        assert!(parse_lsof("").is_empty());
    }

    /// The shell's own directory when nothing is running, the foreground job's
    /// when something is.
    #[test]
    fn the_cwd_pid_follows_the_deepest_foreground_process() {
        let pid_for = |procs: &[Proc]| pane_statuses(&[(7, 100)], procs)[0].1;
        assert_eq!(pid_for(&[]), 100);
        // Two jobs deep, both leaders: the shallower one is what the shell ran.
        assert_eq!(
            pid_for(&[proc(200, 100, true, "zsh"), proc(300, 200, true, "vim")]),
            200
        );
        assert_eq!(pid_for(&[proc(200, 100, false, "npm")]), 100);
    }

    /// The bug this exists for: statuses are sorted by pane and the directories
    /// were gathered in the pane map's arbitrary order, so every card was shown
    /// another card's directory.
    #[test]
    fn each_status_carries_its_own_cwd_pid_whatever_order_the_shells_arrive_in() {
        let procs = vec![proc(200, 11, true, "vim"), proc(300, 22, true, "less")];
        let out = pane_statuses(&[(9, 22), (3, 11)], &procs);
        assert_eq!(out[0].0.pane, 3);
        assert_eq!(out[0].1, 200);
        assert_eq!(out[1].0.pane, 9);
        assert_eq!(out[1].1, 300);
    }
}
