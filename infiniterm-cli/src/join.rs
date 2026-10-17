//! `ift join <[user@]host>`: open a viewer window onto the canvas of the
//! infiniterm already running on `host` (#423).
//!
//! Not `ift connect`: that opens a window whose NEW cards run on a host. This
//! one never starts anything there. It is a second app on this Mac, in its own
//! folder (`<home>/.infiniterm/joins/<host>`), that shows the host app's
//! cards, live, and types into them. The host app is the hub: a viewer reaches
//! it, not `iftd`, so the host's own window is never displaced.
//!
//! What this does: checks that the host can serve (`remote::check`: ssh with
//! no prompt, `ift` installed) and starts the app with `open -n` and the
//! environment that puts it in viewer mode (`INFINITERM_JOIN`). If that host's
//! viewer is already open it says so instead.
//!
//! Called by `main.rs`; it reuses `connect.rs`'s parsing, folder naming and
//! launch plumbing. The host half is `proxy.rs` (`ift proxy join`) and the app's
//! join socket (`infiniterm-core/src/backend/join.rs`).
//!
//! Non-obvious constraints:
//! - The same rules as connect for the host (never an ssh option) and the
//!   folder (short, so its socket path fits).
//! - Nothing is installed on the host: `--install` belongs to `ift connect`,
//!   and a host that cannot serve is said so.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use infiniterm_core::backend::remote::{self, RemoteHost};
use infiniterm_core::paths;

use crate::connect::{ensure_config, folder_name, parse, socket_fits, Parsed};

/// Where viewers keep their folders: beside the remote instances', under the
/// scratch data directory when `INFINITERM_DATA_DIR` is set.
pub fn joins_root(data_override: Option<&Path>, home: &Path) -> PathBuf {
    match data_override {
        Some(dir) => dir.join("joins"),
        None => home.join(".infiniterm").join("joins"),
    }
}

/// The environment that puts the app in viewer mode for this host. The name
/// and colour are read by the same code a remote instance's are.
pub fn launch_env(p: &Parsed, data: &Path, config: &Path) -> Vec<(String, String)> {
    let mut env = vec![
        ("INFINITERM_DATA_DIR".to_string(), data.display().to_string()),
        ("INFINITERM_CONFIG_DIR".to_string(), config.display().to_string()),
        ("INFINITERM_JOIN".to_string(), p.host.clone()),
        (
            "INFINITERM_REMOTE_NAME".to_string(),
            p.name.clone().unwrap_or_else(|| p.host.clone()),
        ),
    ];
    if let Some(c) = &p.color {
        env.push(("INFINITERM_REMOTE_COLOR".into(), c.clone()));
    }
    if let Some(a) = &p.ssh_args {
        env.push(("INFINITERM_JOIN_ARGS".into(), a.clone()));
    }
    if let Some(i) = &p.ift {
        env.push(("INFINITERM_JOIN_IFT".into(), i.clone()));
    }
    // Only a test or a scratch run sets this; the viewer inherits it from here.
    if let Ok(ssh) = std::env::var("INFINITERM_JOIN_SSH") {
        env.push(("INFINITERM_JOIN_SSH".into(), ssh));
    }
    env
}

/// `<app>.app` from this binary's real path, as `connect.rs` finds it.
fn app_bundle() -> Option<PathBuf> {
    let exe = std::fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    let app = exe.parent()?.parent()?.parent()?.to_path_buf();
    (app.extension().is_some_and(|e| e == "app")).then_some(app)
}

pub fn run(args: &[String]) -> ExitCode {
    let mut p = match parse(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ift: {e}");
            return ExitCode::from(2);
        }
    };
    if p.install || p.from.is_some() || p.platform.is_some() {
        eprintln!("ift: join installs nothing; `ift connect {} --install` puts ift on a server", p.host);
        return ExitCode::from(2);
    }
    let mut host: RemoteHost = remote::from_vars(
        Some(&p.host),
        p.ssh_args.as_deref(),
        p.ift.as_deref(),
        std::env::var("INFINITERM_JOIN_SSH").ok().as_deref(),
    )
    .expect("a parsed host is never empty");
    match remote::check(&host) {
        Ok(found) => {
            if !found.is_empty() && found != host.ift {
                host.ift = found.clone();
                p.ift = Some(found);
            }
        }
        Err(e) => {
            eprintln!("ift: {e}");
            return ExitCode::from(1);
        }
    }
    if p.check_only {
        println!("{}: ready (ift is installed)", p.host);
        return ExitCode::SUCCESS;
    }

    let override_dir = std::env::var_os("INFINITERM_DATA_DIR").map(PathBuf::from);
    let data = joins_root(override_dir.as_deref(), &paths::home_dir()).join(folder_name(&p.host));
    if !socket_fits(&data) {
        eprintln!(
            "ift: the folder for {} is too long for a unix socket: {}",
            p.host,
            data.display()
        );
        return ExitCode::from(1);
    }
    let config = data.join("config");
    if let Err(e) = std::fs::create_dir_all(&data) {
        eprintln!("ift: {}: {e}", data.display());
        return ExitCode::from(1);
    }
    if std::os::unix::net::UnixStream::connect(data.join("infiniterm.sock")).is_ok() {
        println!("{}: already joined (its window is open)", p.host);
        return ExitCode::SUCCESS;
    }
    if let Some(c) = p
        .color
        .as_deref()
        .and_then(infiniterm_core::remote_identity::parse_hex)
    {
        if let Err(e) = infiniterm_core::remote_identity::save_color(&data, Some(c)) {
            eprintln!("ift: could not save the colour: {e}");
            return ExitCode::from(1);
        }
    }
    if let Err(e) = ensure_config(&config) {
        eprintln!("ift: could not make {}: {e}", config.display());
        return ExitCode::from(1);
    }
    let Some(app) = app_bundle() else {
        eprintln!("ift: join needs the app bundle (run the ift inside infiniterm.app)");
        return ExitCode::from(1);
    };
    let mut open = Command::new("open");
    open.arg("-n").arg(&app);
    for (k, v) in launch_env(&p, &data, &config) {
        open.arg("--env").arg(format!("{k}={v}"));
    }
    match open.status() {
        Ok(s) if s.success() => {
            println!("{}: opening a viewer window", p.host);
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
    fn viewers_live_beside_the_remote_instances_not_in_them() {
        let home = Path::new("/Users/me");
        assert_eq!(
            joins_root(None, home),
            Path::new("/Users/me/.infiniterm/joins")
        );
        assert_eq!(
            joins_root(Some(Path::new("/scratch")), home),
            Path::new("/scratch/joins")
        );
    }

    #[test]
    fn the_viewer_environment_names_the_host_without_making_it_a_remote_instance() {
        let p = parse(&a(&["mini", "--name", "the mini", "--color", "3B82F6"])).unwrap();
        let env = launch_env(&p, Path::new("/d"), Path::new("/d/config"));
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("INFINITERM_JOIN"), Some("mini"));
        assert_eq!(get("INFINITERM_REMOTE_NAME"), Some("the mini"));
        assert_eq!(get("INFINITERM_REMOTE_COLOR"), Some("3b82f6"));
        assert_eq!(get("INFINITERM_DATA_DIR"), Some("/d"));
        assert_eq!(get("INFINITERM_CONFIG_DIR"), Some("/d/config"));
        // The one thing that would start shells on the host.
        assert_eq!(get("INFINITERM_REMOTE"), None);
    }

    #[test]
    fn join_installs_nothing_and_says_where_that_lives() {
        // parse accepts --install for connect; join refuses it before any ssh.
        let code = run(&a(&["mini", "--install"]));
        assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::from(2)));
    }
}
