//! The hosts this Mac has joined (#891): what "Join: connect to host…" lists
//! and joins again, and what "Join: forget host…" removes.
//!
//! The list is the viewer folders `ift join` already makes, one per host
//! (`<home>/.infiniterm/joins/<folder>`, under `INFINITERM_DATA_DIR` when that
//! is set). `ift join` makes a folder only after the host passed its check
//! (ssh with no prompt, `ift` and `iftd` there, the sessions folder answering),
//! so every folder is a host known to work, and the palette asks no host
//! anything when it opens. A host that stopped working since says so when it
//! is picked, through join's own sentence.
//!
//! The folder name is a slug of the host (`connect::folder_name` in the cli),
//! which cannot always give back what was typed (`user@host`, a long name cut
//! with a hash), so `ift join` writes `RECORD` into the folder with the flags
//! to join again. A folder from a build before that is listed by its name,
//! which is the host as typed for a plain ssh alias.
//!
//! Called by `infiniterm-cli/src/join.rs` (`joins_root`, `record`) and the ui
//! (`runtime.rs`: `list` on `Effect::RefreshJoinedHosts`, `forget` on
//! `Effect::ForgetJoinedHost`); the palette rows are `model/palette_state.rs`'s.
//! Every function takes the folder as an argument so a test never touches the
//! real one.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The file in a viewer folder that says how the host was joined.
pub const RECORD: &str = "join.json";

/// How a host was joined: enough to run `ift join` again the same way.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRecord {
    /// The host as typed: an ssh alias or `user@host`.
    pub host: String,
    /// `--name`: what the window and the Dock call it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `--ssh-args`: extra ssh arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<String>,
    /// `--ift`: where `ift` is on the host, when it is not on ssh's PATH.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ift: Option<String>,
    /// When it was last joined, in seconds since 1970: the list's order.
    #[serde(default)]
    pub joined: u64,
}

impl JoinRecord {
    /// The arguments after `ift join` that join this host again.
    pub fn args(&self) -> Vec<String> {
        let mut args = vec![self.host.clone()];
        for (flag, value) in [
            ("--name", &self.name),
            ("--ssh-args", &self.ssh),
            ("--ift", &self.ift),
        ] {
            if let Some(v) = value {
                args.push(flag.into());
                args.push(v.clone());
            }
        }
        args
    }
}

/// One row of the list: the viewer folder's name and how to join it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinedHost {
    pub folder: String,
    pub record: JoinRecord,
}

impl JoinedHost {
    /// What the palette shows: the name given at join, else the host.
    pub fn label(&self) -> &str {
        self.record.name.as_deref().unwrap_or(&self.record.host)
    }
}

/// Where viewers keep their folders: beside the remote instances', under the
/// scratch data directory when `INFINITERM_DATA_DIR` is set.
pub fn joins_root(data_override: Option<&Path>, home: &Path) -> PathBuf {
    match data_override {
        Some(dir) => dir.join("joins"),
        None => home.join(".infiniterm").join("joins"),
    }
}

/// Write `record` into the viewer folder `dir`, which exists.
pub fn record(dir: &Path, record: &JoinRecord) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(record).map_err(std::io::Error::other)?;
    std::fs::write(dir.join(RECORD), text + "\n")
}

/// Every host under `root`, the last joined first, then by label. A folder
/// with no readable record is the host of its own name, joined at no known
/// time, so it sorts after the ones that have one. A missing `root` is an
/// empty list.
pub fn list(root: &Path) -> Vec<JoinedHost> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut hosts: Vec<JoinedHost> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let folder = e.file_name().to_str()?.to_string();
            if folder.starts_with('.') {
                return None;
            }
            let record = std::fs::read_to_string(e.path().join(RECORD))
                .ok()
                .and_then(|t| serde_json::from_str::<JoinRecord>(&t).ok())
                .filter(|r| !r.host.is_empty())
                .unwrap_or_else(|| JoinRecord {
                    host: folder.clone(),
                    ..JoinRecord::default()
                });
            Some(JoinedHost { folder, record })
        })
        .collect();
    hosts.sort_by(|a, b| {
        b.record
            .joined
            .cmp(&a.record.joined)
            .then_with(|| a.label().to_lowercase().cmp(&b.label().to_lowercase()))
    });
    hosts
}

/// The one line the status bar shows for a finished `ift join`: its last
/// line on stdout when it worked ("opening a viewer window", "already open"),
/// else its last line on stderr (the check's sentence), without the `ift: `
/// every error starts with.
pub fn outcome_line(ok: bool, stdout: &str, stderr: &str) -> String {
    let last = |text: &str| {
        text.lines()
            .rev()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| l.strip_prefix("ift: ").unwrap_or(l).to_string())
    };
    let said = if ok {
        last(stdout)
    } else {
        last(stderr).or_else(|| last(stdout))
    };
    said.unwrap_or_else(|| {
        if ok {
            "joined".into()
        } else {
            "ift join failed".into()
        }
    })
}

/// Remove the viewer folder `folder` under `root`: its window's place, colour
/// and config go with it. Refused while that host's window is open (its app
/// holds the folder), and for a name that is not one folder directly under
/// `root`. The error is a sentence for a person.
pub fn forget(root: &Path, folder: &str) -> Result<(), String> {
    if folder.is_empty() || folder.starts_with('.') || folder.contains(['/', '\\']) {
        return Err(format!("\"{folder}\" is not a joined host"));
    }
    let dir = root.join(folder);
    if !dir.is_dir() {
        return Err(format!("\"{folder}\" is not a joined host"));
    }
    if window_open(&dir) {
        return Err(format!("{folder}: its window is open; close it first"));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("could not forget {folder}: {e}"))
}

/// The viewer app for this folder is running: its socket answers. A socket
/// file left by a window that quit does not (`update::live_viewer_dirs`
/// asks the same way).
#[cfg(unix)]
fn window_open(dir: &Path) -> bool {
    let sock = crate::paths::socket_path_for(Some(dir.as_os_str()));
    std::os::unix::net::UnixStream::connect(sock).is_ok()
}

/// `ift join` is unix only, so no viewer folder holds a live window here.
#[cfg(not(unix))]
fn window_open(_dir: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How long a closed listener may take to stop answering in a test
    /// (40 x 50 ms = 2 s): the child of another test can hold it that long.
    const SETTLE_TRIES: u32 = 40;
    const SETTLE_MS: u64 = 50;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ift-joined-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn host(root: &Path, folder: &str, rec: Option<JoinRecord>) {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(r) = rec {
            record(&dir, &r).unwrap();
        }
    }

    #[test]
    fn the_root_is_beside_the_remote_instances_or_under_the_scratch_dir() {
        let home = Path::new("/Users/me");
        assert_eq!(
            joins_root(None, home),
            Path::new("/Users/me/.infiniterm/joins")
        );
        assert_eq!(
            joins_root(Some(Path::new("/tmp/x")), home),
            Path::new("/tmp/x/joins")
        );
    }

    #[test]
    fn the_last_joined_come_first_and_a_folder_with_no_record_is_its_own_host() {
        let root = scratch();
        host(&root, "old-box", None);
        host(&root, "alpha", None);
        host(
            &root,
            "me-mini",
            Some(JoinRecord {
                host: "me@mini".into(),
                joined: 100,
                ..JoinRecord::default()
            }),
        );
        host(
            &root,
            "cg-dev",
            Some(JoinRecord {
                host: "cg-dev".into(),
                joined: 200,
                ..JoinRecord::default()
            }),
        );
        // Not hosts: a file, and a hidden folder.
        std::fs::write(root.join("stray.txt"), "x").unwrap();
        std::fs::create_dir_all(root.join(".cache")).unwrap();

        let rows: Vec<(String, String)> = list(&root)
            .into_iter()
            .map(|h| (h.folder, h.record.host))
            .collect();
        assert_eq!(
            rows,
            [
                ("cg-dev".to_string(), "cg-dev".to_string()),
                ("me-mini".into(), "me@mini".into()),
                ("alpha".into(), "alpha".into()),
                ("old-box".into(), "old-box".into()),
            ]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_root_is_an_empty_list_and_a_broken_record_falls_back_to_the_folder() {
        let root = scratch();
        assert!(list(&root.join("nope")).is_empty());
        host(&root, "box", None);
        std::fs::write(root.join("box").join(RECORD), "not json").unwrap();
        let rows = list(&root);
        assert_eq!(rows[0].record.host, "box");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_record_gives_back_the_flags_it_was_joined_with() {
        let r = JoinRecord {
            host: "me@mini".into(),
            name: Some("Mini".into()),
            ssh: Some("-p 2222".into()),
            ift: Some("/opt/bin/ift".into()),
            joined: 5,
        };
        assert_eq!(
            r.args(),
            [
                "me@mini",
                "--name",
                "Mini",
                "--ssh-args",
                "-p 2222",
                "--ift",
                "/opt/bin/ift"
            ]
        );
        let root = scratch();
        host(&root, "me-mini", Some(r.clone()));
        let row = &list(&root)[0];
        assert_eq!(row.record, r);
        assert_eq!(row.label(), "Mini");
        assert_eq!(
            JoinRecord {
                host: "box".into(),
                ..JoinRecord::default()
            }
            .args(),
            ["box"]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_status_line_is_what_join_said_last() {
        assert_eq!(
            outcome_line(true, "box: opening a viewer window\n", ""),
            "box: opening a viewer window"
        );
        assert_eq!(
            outcome_line(
                false,
                "",
                "ift: no infiniterm is running on box; starting a canvas host there (ift serve)\nift: box: ift is not installed on the server\n\n"
            ),
            "box: ift is not installed on the server"
        );
        assert_eq!(outcome_line(false, "", ""), "ift join failed");
        assert_eq!(outcome_line(true, "", ""), "joined");
    }

    #[test]
    fn forget_removes_one_folder_and_nothing_outside_it() {
        let root = scratch();
        host(&root, "box", None);
        host(&root, "keep", None);
        assert!(forget(&root, "box").is_ok());
        assert!(!root.join("box").exists());
        assert!(root.join("keep").exists());
        for bad in ["", "..", "../keep", "a/b", ".hidden", "missing"] {
            assert!(forget(&root, bad).is_err(), "{bad:?}");
        }
        assert!(root.join("keep").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn forget_refuses_a_host_whose_window_is_open() {
        // A short folder: the Mac's temp dir is too long for a socket path
        // (104 bytes).
        let root = PathBuf::from(format!("/tmp/ijh-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        host(&root, "box", None);
        let listener = std::os::unix::net::UnixListener::bind(crate::paths::socket_path_for(Some(
            root.join("box").as_os_str(),
        )))
        .unwrap();
        assert!(forget(&root, "box").unwrap_err().contains("window is open"));
        drop(listener);
        // The socket file a quit window leaves behind does not count. Asked
        // a few times: macOS marks a new socket close-on-exec only after it
        // is made, so a child another test spawns in that moment (`tar`,
        // `git`) can hold the listener open until it exits.
        let mut forgot = forget(&root, "box");
        for _ in 0..SETTLE_TRIES {
            if forgot.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(SETTLE_MS));
            forgot = forget(&root, "box");
        }
        assert!(forgot.is_ok(), "{forgot:?}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
