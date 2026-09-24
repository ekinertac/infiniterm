//! Updates, the pure half: the manifest the app polls, the rule for what
//! counts as newer, where an updater may act at all, and the shell script
//! that swaps the bundle once the app has quit.
//!
//! Tauri's updater's shape without Tauri: a static `latest.json` on the
//! public releases repo (`tools/dist.sh` writes it, `tools/publish.sh`
//! uploads it), a zip of the notarized, stapled app, and a swap and
//! relaunch. Sparkle was declined (a framework to embed and sign); Tauri's
//! minisign check is replaced by macOS's own, stronger one: the unpacked
//! app must pass `codesign` against a requirement pinned to Ekin's team
//! (`TEAM_REQUIREMENT`) and Gatekeeper's notarization check, so a zip not
//! signed by him is refused with no key of ours to keep or lose.
//!
//! The io half is `infiniterm-ui/src/updater.rs` (the polling thread, the
//! download, the checks, the staging); the swap happens in the waiter that
//! `app.restart` already spawns (runtime.rs `relaunch_after_exit`), after
//! the process is gone, so a running bundle is never renamed under itself.
//! Under the daemon backend a restart keeps every session, which is what
//! makes an update cost nothing: the cards come back to their shells.
//!
//! The one standing constraint this adds: `iftd` daemons started by the
//! previous build keep running after an update, so a new app must still
//! speak their protocol (`session_protocol.rs`). CLAUDE.md carries it.
use std::path::Path;

/// Where the running app looks. GitHub points `releases/latest/download`
/// at the newest release's asset of that name, so this never changes.
pub const LATEST_URL: &str =
    "https://github.com/ekinertac/infiniterm-releases/releases/latest/download/latest.json";

/// How often a running app looks again after the check at launch. Six
/// hours: a friend who leaves it open all week still hears about a fix the
/// same day, and GitHub sees four requests a day per Mac.
pub const CHECK_EVERY_MS: f64 = 6. * 60. * 60. * 1000.;

/// What a downloaded app must satisfy before it replaces this one: signed
/// by Apple's chain, leaf certificate of Ekin's team. The same team id
/// `tools/sign.sh` signs with.
pub const TEAM_REQUIREMENT: &str =
    "anchor apple generic and certificate leaf[subject.OU] = \"QKN7RYV5PD\"";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub version: String,
    /// The commit count the build was made at (`CFBundleVersion`), which is
    /// what is compared: versions stay 0.1.0 for many builds.
    pub build: u64,
    pub tag: String,
    /// The zip of the notarized, stapled app.
    pub url: String,
    pub sha256: String,
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let s = |k: &str| -> Result<String, String> {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(String::from)
            .ok_or_else(|| format!("latest.json has no {k}"))
    };
    let build = v
        .get("build")
        .and_then(|b| b.as_u64())
        .ok_or("latest.json has no build number")?;
    let url = s("url")?;
    // Only from the releases repo, whatever the manifest says: a manifest
    // is not trusted to send the app elsewhere even though the code
    // signature check would catch a foreign build anyway.
    if !url.starts_with("https://github.com/ekinertac/infiniterm-releases/") {
        return Err(format!(
            "latest.json points outside the releases repo: {url}"
        ));
    }
    Ok(Manifest {
        version: s("version")?,
        build,
        tag: s("tag")?,
        url,
        sha256: s("sha256")?,
    })
}

/// Newer is a higher build number, nothing else: a build that goes
/// backwards (a re-published older release) is not offered.
pub fn is_newer(running_build: u64, m: &Manifest) -> bool {
    m.build > running_build
}

/// Whether an updater may act on this bundle: an app installed into an
/// Applications folder. A bundle under the source tree's `target/` is a
/// development build, and replacing it with a release would be a surprise;
/// a scratch instance (`INFINITERM_DATA_DIR`) is the caller's to exclude.
pub fn applies_to(bundle: &Path, home: &Path) -> bool {
    bundle.extension().is_some_and(|e| e == "app")
        && bundle
            .parent()
            .is_some_and(|p| p == Path::new("/Applications") || p == home.join("Applications"))
}

/// The waiter that replaces the bundle after this process has exited, and
/// then opens it: the old bundle is moved aside (deleted at the next
/// launch, `updater::clean`), the staged one moved into its place, and if
/// the second move fails the old one goes back, so a failed update is a
/// plain restart rather than no app at all.
pub fn swap_script(pid: u32, bundle: &str, staged: &str, aside: &str) -> String {
    let q = crate::drop::posix_quote;
    let (b, s, a) = (q(bundle), q(staged), q(aside));
    format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.1; done; \
         if mv {b} {a}; then mv {s} {b} || mv {a} {b}; fi; \
         exec open {b}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{
      "version": "0.1.0", "build": 1301, "tag": "v0.1.0-1301",
      "pub_date": "2026-09-23T10:00:00Z", "commit": "abc1234",
      "url": "https://github.com/ekinertac/infiniterm-releases/releases/download/v0.1.0-1301/infiniterm-0.1.0-1301-arm64.zip",
      "sha256": "00ff", "dmg": "x"
    }"#;

    #[test]
    fn the_manifest_parses_and_newer_is_a_higher_build() {
        let m = parse_manifest(GOOD).unwrap();
        assert_eq!(m.build, 1301);
        assert_eq!(m.tag, "v0.1.0-1301");
        assert!(is_newer(1300, &m));
        assert!(!is_newer(1301, &m), "the same build is not an update");
        assert!(!is_newer(1400, &m), "an older release is never offered");
    }

    #[test]
    fn a_manifest_pointing_elsewhere_or_missing_fields_is_refused() {
        let elsewhere = GOOD.replace(
            "https://github.com/ekinertac/infiniterm-releases/",
            "https://evil.example/",
        );
        assert!(parse_manifest(&elsewhere).is_err());
        assert!(parse_manifest(r#"{"version": "0.1.0"}"#).is_err());
        assert!(parse_manifest("not json").is_err());
    }

    #[test]
    fn only_an_installed_app_updates_itself() {
        let home = Path::new("/Users/me");
        assert!(applies_to(Path::new("/Applications/infiniterm.app"), home));
        assert!(applies_to(
            Path::new("/Users/me/Applications/infiniterm.app"),
            home
        ));
        assert!(!applies_to(
            Path::new("/Users/me/Code/infiniterm/target/bundle/infiniterm.app"),
            home
        ));
        assert!(!applies_to(
            Path::new("/Applications/Other/infiniterm.app"),
            home
        ));
    }

    #[test]
    fn the_swap_waits_moves_aside_puts_back_on_failure_and_opens() {
        let s = swap_script(
            42,
            "/Applications/infiniterm.app",
            "/tmp/u/infiniterm.app",
            "/tmp/u/old.app",
        );
        assert!(s.starts_with("while kill -0 42"));
        let aside = s
            .find("mv /Applications/infiniterm.app /tmp/u/old.app")
            .unwrap();
        let swap = s
            .find("mv /tmp/u/infiniterm.app /Applications/infiniterm.app")
            .unwrap();
        let back = s
            .find("|| mv /tmp/u/old.app /Applications/infiniterm.app")
            .unwrap();
        let open = s.find("exec open /Applications/infiniterm.app").unwrap();
        assert!(aside < swap && swap < back && back < open);
        // A path with a space is quoted, not split.
        let spaced = swap_script(1, "/Users/me/Applications/my term.app", "/a", "/b");
        assert!(
            spaced.contains("'/Users/me/Applications/my term.app'"),
            "{spaced}"
        );
    }
}
