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
//! WINDOWS CHECKS THE HASH, AND A SIGNATURE ONCE A KEY IS SET. Authenticode
//! needs a paid certificate and there is none, so the manifest is the
//! authority there: a download that matches the SHA-256 it names is
//! installed. That makes whoever can write the releases repo able to run
//! code on a Windows machine, where the Mac keeps the signature as a second
//! factor. Ekin decided that trade knowingly, 2026-09-25. The second factor
//! that costs nothing is an Ed25519 signature on the manifest
//! (`update_sign`), required once `update::UPDATE_PUBLIC_KEY` holds a key
//! and ignored while it is empty. `stage` says it again where it happens.
//!
//! Started by `runtime::startup` only for an installed app
//! (`update::applies_to`) and never for a scratch instance. The ui drains
//! `events` in `drain_backend`; `app.update.check` wakes the thread for a
//! check now; a staged update is applied by the restart waiter
//! (`update::swap_script`), which `app.restart` and `app.update.install`
//! both reach.
//!
//! LINUX (#464, #840, docs/agent/linux-port.md "Packaging"): `start_linux`
//! polls `latest-linux.json`. An install.sh install stages the newer build
//! the way install.sh would (`infiniterm_core::linux_update::stage`: the
//! sha256, the signature once a key is set, unpacked beside the running
//! version, `current` untouched) and reports it ready, so a restart or
//! `app.update.install` moves `current` and starts the new version, as on
//! the Mac. A Homebrew install only says a newer build is out, with
//! `brew upgrade`: brew owns its files. The downloader below is the Mac's
//! and Windows's and is not reached on Linux, hence the dead-code allowance
//! there rather than a cfg on every item.
#![cfg_attr(all(unix, not(target_os = "macos")), allow(dead_code))]
use infiniterm_core::update::{is_newer, parse_manifest, Manifest, CHECK_EVERY_MS, LATEST_URL};
// The signature requirement is the macOS half of the check; Windows has
// only the hash (see this file's header).
#[cfg(not(windows))]
use infiniterm_core::update::TEAM_REQUIREMENT;
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
    /// Linux: a newer build is out; `hint` is the command that installs it.
    Available {
        version: String,
        build: u64,
        hint: &'static str,
    },
    /// Only for a check somebody asked for; the timer's are silent.
    UpToDate,
    Failed(String),
}

/// The answer to a check somebody asked for when it downloaded nothing new:
/// "up to date", unless a newer build is already staged (the timer's check
/// found and staged it earlier, and said so once). Saying "up to date" then
/// hid a download that only needed a restart (#619).
fn answer_asked(staged: Option<&Staged>) -> UpdateEvent {
    match staged {
        Some(s) => UpdateEvent::Ready(s.clone()),
        None => UpdateEvent::UpToDate,
    }
}

pub struct Updater {
    pub events: Receiver<UpdateEvent>,
    wake: Sender<()>,
    pub staged: Option<Staged>,
    /// Linux: the newer build `start_notifier` found, and its command.
    pub available: Option<(u64, &'static str)>,
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

/// The build number this binary was made at.
///
/// Windows and Linux have no bundle to read it back out of, so
/// `tools/dist.ps1` and the Linux `desktop` job (`linux.yml`) stamp it at
/// COMPILE time and a dev build simply has none. The status bar already
/// draws nothing when there is none; that line exists so a friend's bug
/// report says which build they are on, and a dev build is not one anybody
/// reports.
#[cfg(not(target_os = "macos"))]
pub fn build_of(_bundle: &Path) -> Option<u64> {
    option_env!("INFINITERM_BUILD").and_then(|b| b.trim().parse().ok())
}

/// The build number of a bundle, from its Info.plist.
#[cfg(target_os = "macos")]
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
        let mut last_staged: Option<Staged> = None;
        let mut asked = false;
        loop {
            match check(running, &thread_dir, staged_build) {
                Ok(Some(s)) => {
                    staged_build = Some(s.build);
                    last_staged = Some(s.clone());
                    let _ = tx.send(UpdateEvent::Ready(s));
                }
                Ok(None) if asked => {
                    let _ = tx.send(answer_asked(last_staged.as_ref()));
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
        available: None,
        bundle,
        dir,
    }
}

/// Linux: the same hourly poll, of `latest-linux.json`. `root` is the
/// install.sh folder (`~/.local/share/infiniterm`) for an install.sh install
/// and `None` under Homebrew. With a root, a newer build is staged into it
/// and reported `Ready` once (`Updater.dir` keeps the root for the install);
/// a stage that fails says so to the one who asked, and the timer's falls
/// back to the notice with install.sh's command, so a broken download never
/// hides that a release is out. Without one, the notice only (#464). A check
/// somebody asked for also says when there is nothing new.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn start_linux(
    running: u64,
    install: infiniterm_core::update::LinuxInstall,
    root: Option<PathBuf>,
) -> Updater {
    use infiniterm_core::update::{linux_update_hint, parse_linux_manifest};
    let (tx, events) = channel();
    let (wake, woken) = channel::<()>();
    // A manifest elsewhere, for tools/drive/linux/update-test.sh (curl takes
    // a file:// URL). The tarball it names must still be under the release
    // downloads (`parse_linux_manifest`) and match its sha256, so a
    // redirected manifest cannot install anything the releases do not hold.
    let url = std::env::var("INFINITERM_LATEST_URL").unwrap_or_else(|_| LATEST_URL.to_string());
    let thread_root = root.clone();
    std::thread::spawn(move || {
        let mut told: Option<u64> = None;
        let mut staged: Option<Staged> = None;
        let mut asked = false;
        loop {
            let found = run("curl", &["-fsSL", "--max-time", "30", &url])
                .and_then(|text| parse_linux_manifest(&text, std::env::consts::ARCH));
            let available = |r: &infiniterm_core::update::LinuxRelease| UpdateEvent::Available {
                version: r.version.clone(),
                build: r.build,
                hint: linux_update_hint(install),
            };
            match found {
                Ok(r) if r.build <= running => {
                    if asked {
                        let _ = tx.send(UpdateEvent::UpToDate);
                    }
                }
                // Staged already: say it again only to the one who asks.
                Ok(r) if staged.as_ref().is_some_and(|s| s.build == r.build) => {
                    if asked {
                        let _ = tx.send(answer_asked(staged.as_ref()));
                    }
                }
                Ok(r) => match &thread_root {
                    Some(root) => {
                        let fetch = |from: &str, to: &Path| {
                            run(
                                "curl",
                                &[
                                    "-fsSL",
                                    "--max-time",
                                    "900",
                                    "-o",
                                    &to.to_string_lossy(),
                                    from,
                                ],
                            )
                            .map(|_| ())
                        };
                        match infiniterm_core::linux_update::stage(
                            &r,
                            root,
                            infiniterm_core::update::UPDATE_PUBLIC_KEY,
                            fetch,
                        ) {
                            Ok(dir) => {
                                let s = Staged {
                                    version: r.version.clone(),
                                    build: r.build,
                                    app: dir,
                                };
                                staged = Some(s.clone());
                                let _ = tx.send(UpdateEvent::Ready(s));
                            }
                            Err(e) if asked => {
                                let _ = tx.send(UpdateEvent::Failed(e));
                            }
                            Err(e) => {
                                eprintln!("[infiniterm] update stage: {e}");
                                if told != Some(r.build) {
                                    told = Some(r.build);
                                    let _ = tx.send(available(&r));
                                }
                            }
                        }
                    }
                    // Homebrew: once per build, and again to the one who
                    // asks (#619), never "up to date".
                    None if asked || told != Some(r.build) => {
                        told = Some(r.build);
                        let _ = tx.send(available(&r));
                    }
                    None => {}
                },
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
        available: None,
        bundle: PathBuf::new(),
        dir: root.unwrap_or_default(),
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

/// Download, check the hash, unpack, check the build. Any failure leaves
/// nothing staged.
///
/// THE HASH IS THE WHOLE CHECK on Windows UNTIL A KEY IS SET, which is a real
/// difference from the Mac and worth saying plainly: there a download must
/// also be signed by Ekin's team and notarized, so a zip somebody else built
/// is refused even if the manifest names its hash. Authenticode needs a paid
/// certificate and there is none, so here the manifest is the authority and
/// whoever can write the releases repo decides what runs. With
/// `UPDATE_PUBLIC_KEY` set, the repo's write access is no longer enough: the
/// manifest must also carry Ekin's signature, checked first. The manifest
/// itself comes over TLS from that repo and `parse_manifest` refuses a url
/// pointing anywhere else, which is the floor the hash rests on.
///
/// `curl.exe` and `tar.exe` ship with Windows 10 1803 and later, and
/// `certutil` with every Windows, so no crate is added for any of it — the
/// same reasoning the macOS side uses for curl, ditto and shasum.
#[cfg(windows)]
fn stage(m: &Manifest, dir: &Path) -> Result<Staged, String> {
    // The second factor, before a byte is downloaded: with a public key
    // committed (`update::UPDATE_PUBLIC_KEY`) a manifest must carry a valid
    // Ed25519 signature over its tag, build and hash. With none committed
    // this passes and the hash below is the whole check, as before.
    infiniterm_core::update::check_manifest_signature(
        infiniterm_core::update::UPDATE_PUBLIC_KEY,
        m,
    )?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let zip = dir.join(format!("{}.zip", m.tag));
    let out = dir.join(&m.tag);
    let _ = std::fs::remove_dir_all(&out);
    let zip_s = zip.to_string_lossy().into_owned();
    run(
        "curl",
        &["-fsSL", "--max-time", "900", "-o", &zip_s, &m.url],
    )?;

    if hash_of(&zip_s)? != m.sha256.to_ascii_lowercase() {
        let _ = std::fs::remove_file(&zip);
        return Err(format!("{} does not match its checksum", m.tag));
    }

    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    run("tar", &["-xf", &zip_s, "-C", &out.to_string_lossy()])?;
    let _ = std::fs::remove_file(&zip);

    // The zip holds one folder, named for the build; find it rather than
    // spelling the name twice.
    let folder = std::fs::read_dir(&out)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.join("infiniterm.exe").is_file())
        .ok_or_else(|| format!("{} holds no infiniterm.exe", m.tag))?;

    Ok(Staged {
        version: m.version.clone(),
        build: m.build,
        app: folder,
    })
}

/// A file's SHA-256, lowercase, from `certutil`.
///
/// Its output is a header line, the digest, and a success line; older
/// Windows put spaces inside the digest, so everything that is not a hex
/// digit comes out before comparing.
#[cfg(windows)]
fn hash_of(path: &str) -> Result<String, String> {
    let out = run("certutil", &["-hashfile", path, "SHA256"])?;
    let digest: String = out
        .lines()
        .find(|l| {
            let bare: String = l.chars().filter(|c| !c.is_whitespace()).collect();
            bare.len() == 64 && bare.chars().all(|c| c.is_ascii_hexdigit())
        })
        .ok_or_else(|| format!("certutil gave no digest for {path}"))?
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    Ok(digest.to_ascii_lowercase())
}

/// Download, check, unpack, check again. Any failure leaves nothing staged.
#[cfg(not(windows))]
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
    fn a_check_somebody_asked_for_reports_a_staged_update_not_up_to_date() {
        assert!(matches!(answer_asked(None), UpdateEvent::UpToDate));
        let s = Staged {
            version: "0.6.7".into(),
            build: 1337,
            app: PathBuf::from("/tmp/x.app"),
        };
        match answer_asked(Some(&s)) {
            UpdateEvent::Ready(got) => {
                assert_eq!((got.version.as_str(), got.build), ("0.6.7", 1337))
            }
            _ => panic!("a staged update must be reported as ready"),
        }
    }

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
