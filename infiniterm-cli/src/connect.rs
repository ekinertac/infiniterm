//! `ift connect <[user@]host>`: open a SECOND infiniterm for a host (#118).
//!
//! The new instance is a separate process with its own window, Dock tile and
//! canvas. Its terminal cards are shells kept by `iftd` on the host, reached
//! over ssh (`infiniterm-core/src/backend/remote.rs`), and its canvas and
//! settings live in a folder per host, so connecting again brings the same
//! cards back and the local instance is never touched.
//!
//! What this does: checks that the host can serve (`remote::check`: ssh with
//! no prompt, `ift` and `iftd` installed), makes `<data>/remotes/<host>` and a
//! `config` folder in it seeded ONCE from the local settings and keybindings,
//! and starts the app with `open -n` and the environment that puts it in
//! remote mode. If that host's instance is already running it says so instead.
//!
//! Called by `main.rs`. Related: `proxy.rs` (the server half), `tools/drive/
//! remote.sh` (the app in remote mode, headless).
//!
//! Non-obvious constraints:
//! - The host is handed to ssh as the destination, so one that starts with `-`
//!   is refused: `-oProxyCommand=...` would be an option, not a host.
//! - The folder name comes from the host name; anything outside letters,
//!   digits and `._@-` becomes `_`, and a name that would start with a dot is
//!   prefixed, so a host can never name a path outside `remotes/`.
//! - `open -n` needs the .app: found from this binary's own real path, which
//!   is `<app>/Contents/MacOS/ift` whether it was run through the
//!   `~/.local/bin` symlink or not.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use infiniterm_core::backend::remote::{self, RemoteHost};
use infiniterm_core::paths;

/// What `ift connect` was asked.
#[derive(Debug, PartialEq, Eq, Default)]
pub struct Parsed {
    pub host: String,
    /// A name for the window and the Dock; the host when none is given.
    pub name: Option<String>,
    /// `RRGGBB`, without a `#`; the app picks one from the host name when absent.
    pub color: Option<String>,
    pub ssh_args: Option<String>,
    pub ift: Option<String>,
    pub check_only: bool,
}

/// A host ssh would take as a destination and not as an option.
pub fn valid_host(host: &str) -> bool {
    !host.is_empty() && !host.starts_with('-') && !host.chars().any(char::is_whitespace)
}

fn valid_color(c: &str) -> bool {
    c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit())
}

pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut p = Parsed::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} takes a value"))
        };
        match a.as_str() {
            "--check" => p.check_only = true,
            "--name" => p.name = Some(value("--name")?),
            "--color" => {
                let c = value("--color")?;
                let c = c.trim_start_matches('#').to_string();
                if !valid_color(&c) {
                    return Err(format!("not a colour: {c} (six hex digits, like 3b82f6)"));
                }
                p.color = Some(c.to_ascii_lowercase());
            }
            "--ssh-args" => p.ssh_args = Some(value("--ssh-args")?),
            "--ift" => p.ift = Some(value("--ift")?),
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            host => {
                if !p.host.is_empty() {
                    return Err("ift connect takes one host".into());
                }
                p.host = host.to_string();
            }
        }
    }
    if p.host.is_empty() {
        return Err("ift connect takes a host: user@host or an ssh alias".into());
    }
    if !valid_host(&p.host) {
        return Err(format!("not a host: {}", p.host));
    }
    Ok(p)
}

/// The folder name for a host.
pub fn slug(host: &str) -> String {
    let s: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._@-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.starts_with('.') {
        format!("h{s}")
    } else {
        s
    }
}

/// Copies the local `settings.json` and `keybindings.json` into `to` the FIRST
/// time (when `to` does not exist), so a host starts from your settings and
/// then keeps its own. Returns whether it seeded.
pub fn seed_config(from: &Path, to: &Path) -> std::io::Result<bool> {
    if to.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(to)?;
    for name in ["settings.json", "keybindings.json"] {
        let src = from.join(name);
        if src.is_file() {
            std::fs::copy(&src, to.join(name))?;
        }
    }
    Ok(true)
}

/// The environment that puts the app in remote mode for this host.
pub fn launch_env(p: &Parsed, data: &Path, config: &Path) -> Vec<(String, String)> {
    let mut env = vec![
        ("INFINITERM_DATA_DIR".to_string(), data.display().to_string()),
        ("INFINITERM_CONFIG_DIR".to_string(), config.display().to_string()),
        ("INFINITERM_REMOTE".to_string(), p.host.clone()),
        (
            "INFINITERM_REMOTE_NAME".to_string(),
            p.name.clone().unwrap_or_else(|| p.host.clone()),
        ),
    ];
    if let Some(c) = &p.color {
        env.push(("INFINITERM_REMOTE_COLOR".into(), c.clone()));
    }
    if let Some(a) = &p.ssh_args {
        env.push(("INFINITERM_REMOTE_ARGS".into(), a.clone()));
    }
    if let Some(i) = &p.ift {
        env.push(("INFINITERM_REMOTE_IFT".into(), i.clone()));
    }
    // Only a test sets this; the instance inherits it from here.
    if let Ok(ssh) = std::env::var("INFINITERM_REMOTE_SSH") {
        env.push(("INFINITERM_REMOTE_SSH".into(), ssh));
    }
    env
}

/// `<app>.app` from this binary's real path (`.../infiniterm.app/Contents/MacOS/ift`).
fn app_bundle() -> Option<PathBuf> {
    let exe = std::fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    let app = exe.parent()?.parent()?.parent()?.to_path_buf();
    (app.extension().is_some_and(|e| e == "app")).then_some(app)
}

pub fn run(args: &[String]) -> ExitCode {
    let p = match parse(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ift: {e}");
            return ExitCode::from(2);
        }
    };
    let host: RemoteHost = remote::from_vars(
        Some(&p.host),
        p.ssh_args.as_deref(),
        p.ift.as_deref(),
        std::env::var("INFINITERM_REMOTE_SSH").ok().as_deref(),
    )
    .expect("a parsed host is never empty");
    if let Err(e) = remote::check(&host) {
        eprintln!("ift: {e}");
        return ExitCode::from(1);
    }
    if p.check_only {
        println!("{}: ready (ift and iftd are installed)", p.host);
        return ExitCode::SUCCESS;
    }

    let data = paths::app_support_dir().join("remotes").join(slug(&p.host));
    let config = data.join("config");
    if let Err(e) = std::fs::create_dir_all(&data) {
        eprintln!("ift: {}: {e}", data.display());
        return ExitCode::from(1);
    }
    // Already open? The instance owns this folder's socket.
    if std::os::unix::net::UnixStream::connect(data.join("infiniterm.sock")).is_ok() {
        println!("{}: already connected (its window is open)", p.host);
        return ExitCode::SUCCESS;
    }
    match seed_config(&paths::config_dir(), &config) {
        Ok(true) => println!("settings for {} start from your current ones", p.host),
        Ok(false) => {}
        Err(e) => {
            eprintln!("ift: could not make {}: {e}", config.display());
            return ExitCode::from(1);
        }
    }
    let Some(app) = app_bundle() else {
        eprintln!("ift: connect needs the app bundle (run the ift inside infiniterm.app)");
        return ExitCode::from(1);
    };
    let mut open = Command::new("open");
    open.arg("-n").arg(&app);
    for (k, v) in launch_env(&p, &data, &config) {
        open.arg("--env").arg(format!("{k}={v}"));
    }
    match open.status() {
        Ok(s) if s.success() => {
            println!("{}: opening a window", p.host);
            ExitCode::SUCCESS
        }
        Ok(s) => {
            eprintln!("ift: open exited with {s}");
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("ift: could not run open: {e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_host_and_its_options_are_parsed() {
        let p = parse(&a(&[
            "root@100.1.1.1", "--name", "build box", "--color", "#3B82F6", "--ssh-args", "-p 2222",
            "--ift", "/opt/ift", "--check",
        ]))
        .unwrap();
        assert_eq!(p.host, "root@100.1.1.1");
        assert_eq!(p.name.as_deref(), Some("build box"));
        assert_eq!(p.color.as_deref(), Some("3b82f6"));
        assert_eq!(p.ssh_args.as_deref(), Some("-p 2222"));
        assert_eq!(p.ift.as_deref(), Some("/opt/ift"));
        assert!(p.check_only);
    }

    #[test]
    fn bad_use_is_refused_with_a_reason() {
        assert!(parse(&a(&[])).is_err());
        assert!(parse(&a(&["a", "b"])).is_err());
        assert!(parse(&a(&["a", "--color", "red"])).is_err());
        assert!(parse(&a(&["a", "--name"])).is_err());
        assert!(parse(&a(&["a", "--nope"])).is_err());
    }

    #[test]
    fn a_host_that_ssh_would_read_as_an_option_is_refused() {
        assert!(!valid_host("-oProxyCommand=touch /tmp/x"));
        assert!(parse(&a(&["-oProxyCommand=x"])).is_err());
        assert!(!valid_host("a b"));
        assert!(!valid_host(""));
        assert!(valid_host("root@100.1.1.1"));
        assert!(valid_host("mini-m4"));
    }

    #[test]
    fn the_folder_name_cannot_leave_the_remotes_folder() {
        assert_eq!(slug("root@100.1.1.1"), "root@100.1.1.1");
        assert_eq!(slug("a/b"), "a_b");
        assert_eq!(slug("../etc"), "h.._etc");
        assert_eq!(slug("..") , "h..");
        assert_eq!(slug("my host:22"), "my_host_22");
    }

    #[test]
    fn settings_are_copied_once_and_then_left_alone() {
        let root = std::env::temp_dir().join(format!("ift-conn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let from = root.join("local");
        let to = root.join("remote").join("config");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("settings.json"), "{\"theme\":\"x\"}").unwrap();
        std::fs::write(from.join("settings.default.json"), "generated").unwrap();
        assert!(seed_config(&from, &to).unwrap(), "the first time seeds");
        assert_eq!(std::fs::read_to_string(to.join("settings.json")).unwrap(), "{\"theme\":\"x\"}");
        assert!(!to.join("settings.default.json").exists(), "generated files are not copied");
        assert!(!to.join("keybindings.json").exists(), "a missing file is skipped");
        std::fs::write(to.join("settings.json"), "{\"theme\":\"mine\"}").unwrap();
        std::fs::write(from.join("settings.json"), "{\"theme\":\"changed\"}").unwrap();
        assert!(!seed_config(&from, &to).unwrap(), "the second time does nothing");
        assert_eq!(std::fs::read_to_string(to.join("settings.json")).unwrap(), "{\"theme\":\"mine\"}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_launch_environment_puts_the_app_in_remote_mode() {
        let p = parse(&a(&["srv", "--color", "ff8800", "--ssh-args", "-p 2222"])).unwrap();
        let env = launch_env(&p, Path::new("/d"), Path::new("/d/config"));
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("INFINITERM_DATA_DIR"), Some("/d"));
        assert_eq!(get("INFINITERM_CONFIG_DIR"), Some("/d/config"));
        assert_eq!(get("INFINITERM_REMOTE"), Some("srv"));
        assert_eq!(get("INFINITERM_REMOTE_NAME"), Some("srv"));
        assert_eq!(get("INFINITERM_REMOTE_COLOR"), Some("ff8800"));
        assert_eq!(get("INFINITERM_REMOTE_ARGS"), Some("-p 2222"));
        assert_eq!(get("INFINITERM_REMOTE_IFT"), None);
    }
}
