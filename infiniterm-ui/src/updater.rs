//! Updates, the io half: a thread that polls the releases repo, downloads
//! a newer build, checks it the way macOS checks a download, and stages it
//! for the next restart. `infiniterm_core::update` holds every rule this
//! follows (the manifest, what is newer, where an updater may act, the
//! swap script); read that first.
//!
//! No HTTP crate: `curl`, the way the omnibox's suggestions are fetched,
//! so no request can hold a frame and the tree stays as it is. No archive
//! crate: `ditto`, which keeps the code signature intact on the way out
//! of the zip. No crypto crate: `codesign` against the team requirement,
//! then `spctl` for the notarization, which is a stronger check than the
//! minisign signature Tauri's updater carries for platforms without one.
//!
//! Started by `runtime::startup` only for an installed app
//! (`update::applies_to`) and never for a scratch instance. The ui drains
//! `events` in `drain_backend`; `app.update.check` wakes the thread for a
//! check now; a staged update is applied by the restart waiter
//! (`update::swap_script`), which `app.restart` and `app.update.install`
//! both reach.
use infiniterm_core::update::{
    is_newer, parse_manifest, Manifest, CHECK_EVERY_MS, LATEST_URL, TEAM_REQUIREMENT,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

/// A download that passed every check and waits in the staging dir.
#[derive(Clone, Debug)]
pub struct Staged {
    pub version: String,
    pub build: u64,
    pub app: PathBuf,
}

pub enum UpdateEvent {
    Ready(Staged),
    /// Only for a check somebody asked for; the timer's are silent.
    UpToDate,
    Failed(String),
}

pub struct Updater {
    pub events: Receiver<UpdateEvent>,
    wake: Sender<()>,
    pub staged: Option<Staged>,
    /// The installed bundle the swap replaces.
    pub bundle: PathBuf,
    /// Where downloads unpack and the old bundle is moved aside.
    pub dir: PathBuf,
}

impl Updater {
    /// A check now, reported either way.
    pub fn check_now(&self) {
        let _ = self.wake.send(());
    }
}

/// The staging dir, emptied at every launch: what it holds is either a
/// bundle the last update moved aside or a download the last run never
/// installed, and a fresh check stages again.
pub fn clean(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

/// The build number of a bundle, from its Info.plist.
pub fn build_of(bundle: &Path) -> Option<u64> {
    let out = Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg("Print :CFBundleVersion")
        .arg(bundle.join("Contents/Info.plist"))
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

pub fn start(bundle: PathBuf, running: u64, dir: PathBuf) -> Updater {
    let (tx, events) = channel();
    let (wake, woken) = channel::<()>();
    let thread_dir = dir.clone();
    std::thread::spawn(move || {
        let mut staged_build: Option<u64> = None;
        let mut asked = false;
        loop {
            match check(running, &thread_dir, staged_build) {
                Ok(Some(s)) => {
                    staged_build = Some(s.build);
                    let _ = tx.send(UpdateEvent::Ready(s));
                }
                Ok(None) if asked => {
                    let _ = tx.send(UpdateEvent::UpToDate);
                }
                Ok(None) => {}
                Err(e) if asked => {
                    let _ = tx.send(UpdateEvent::Failed(e));
                }
                Err(e) => eprintln!("[infiniterm] update check: {e}"),
            }
            match woken.recv_timeout(Duration::from_millis(CHECK_EVERY_MS as u64)) {
                Ok(()) => asked = true,
                Err(RecvTimeoutError::Timeout) => asked = false,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    Updater {
        events,
        wake,
        staged: None,
        bundle,
        dir,
    }
}

/// One check: `Ok(None)` is up to date (or the newest is already staged).
fn check(running: u64, dir: &Path, staged: Option<u64>) -> Result<Option<Staged>, String> {
    let text = run("curl", &["-fsSL", "--max-time", "20", LATEST_URL])?;
    let m = parse_manifest(&text)?;
    if !is_newer(running, &m) || staged == Some(m.build) {
        return Ok(None);
    }
    stage(&m, dir).map(Some)
}

/// Download, check, unpack, check again. Any failure leaves nothing staged.
fn stage(m: &Manifest, dir: &Path) -> Result<Staged, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let zip = dir.join(format!("{}.zip", m.tag));
    let out = dir.join(&m.tag);
    let _ = std::fs::remove_dir_all(&out);
    let zip_s = zip.to_string_lossy().into_owned();
    run(
        "curl",
        &["-fsSL", "--max-time", "900", "-o", &zip_s, &m.url],
    )?;
    let sum = run("shasum", &["-a", "256", &zip_s])?;
    if sum.split_whitespace().next() != Some(m.sha256.as_str()) {
        let _ = std::fs::remove_file(&zip);
        return Err(format!("{} does not match its checksum", m.tag));
    }
    run("ditto", &["-x", "-k", &zip_s, &out.to_string_lossy()])?;
    let _ = std::fs::remove_file(&zip);
    let app = out.join("infiniterm.app");
    let app_s = app.to_string_lossy().into_owned();
    let refuse = |why: String| {
        let _ = std::fs::remove_dir_all(&out);
        Err(why)
    };
    // Signed by Ekin's team, every nested binary, nothing altered since.
    if let Err(e) = run(
        "codesign",
        &[
            "--verify",
            "--deep",
            "--strict",
            &format!("-R={TEAM_REQUIREMENT}"),
            &app_s,
        ],
    ) {
        return refuse(format!(
            "{} is not signed by infiniterm's developer: {e}",
            m.tag
        ));
    }
    // Notarized: what Gatekeeper would demand of a download.
    if let Err(e) = run("spctl", &["-a", "-t", "exec", &app_s]) {
        return refuse(format!("{} is not notarized: {e}", m.tag));
    }
    if build_of(&app) != Some(m.build) {
        return refuse(format!(
            "{} carries a different build than its manifest",
            m.tag
        ));
    }
    Ok(Staged {
        version: m.version.clone(),
        build: m.build,
        app,
    })
}

/// A command's stdout, or its stderr as the error.
fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{cmd} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

// macOS only: every one of these runs a macOS tool (PlistBuddy for the
// build number, codesign for the requirement) against a macOS bundle. The
// updater itself does not run on Windows either (`update::applies_to` wants
// an Applications folder); shipping there is phase 6 of the Windows handoff
// and brings WinVerifyTrust with it.
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn the_build_number_is_read_from_the_bundles_info_plist() {
        let dir = std::env::temp_dir().join(format!("updater-build-{}", std::process::id()));
        let app = dir.join("x.app");
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleVersion</key><string>1301</string></dict></plist>"#,
        )
        .unwrap();
        assert_eq!(build_of(&app), Some(1301));
        assert_eq!(build_of(&dir.join("missing.app")), None);
        clean(&dir);
        assert!(!dir.exists());
    }

    /// The requirement string is what every update is judged by, and a typo
    /// in it would refuse every one. Checked against a real bundle signed
    /// by the team (the installed app, when this Mac has it) and a real
    /// bundle signed by somebody else (Apple's own).
    #[test]
    fn the_team_requirement_accepts_ekins_signature_and_refuses_anyone_elses() {
        let req = format!("-R={TEAM_REQUIREMENT}");
        let ours = "/Applications/infiniterm.app";
        if Path::new(ours).exists() {
            assert!(
                run("codesign", &["--verify", &req, ours]).is_ok(),
                "the installed app must satisfy the requirement"
            );
        }
        assert!(
            run(
                "codesign",
                &["--verify", &req, "/System/Applications/Calculator.app"]
            )
            .is_err(),
            "Apple's own app is not ours"
        );
    }
}
