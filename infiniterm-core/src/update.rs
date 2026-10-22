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
/// at the newest release's asset of that name. The main repo since #52:
/// releases lived on `ekinertac/infiniterm-releases` while the source was
/// private, and builds up to 0.4.0 still poll that one (a bridge release
/// there carries them over, `tools/publish.sh BRIDGE=1`).
///
/// A SEPARATE file per platform rather than one manifest with a key each.
/// An already-shipped Mac app parses this with `parse_manifest` and a shape
/// it does not expect is a refusal on somebody else's machine, so the
/// Windows release gets its own name (`tools/dist.ps1` writes it).
pub const LATEST_URL: &str = if cfg!(windows) {
    "https://github.com/ekinertac/infiniterm/releases/latest/download/latest-windows.json"
} else if cfg!(target_os = "linux") {
    "https://github.com/ekinertac/infiniterm/releases/latest/download/latest-linux.json"
} else {
    "https://github.com/ekinertac/infiniterm/releases/latest/download/latest.json"
};

/// What `latest-linux.json` says about the newest Linux release, for the
/// notice. Linux never swaps itself (docs/agent/linux-port.md,
/// "Packaging"): install.sh or `brew upgrade` is the update, so only the
/// version and build matter here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinuxRelease {
    pub version: String,
    pub build: u64,
    pub tag: String,
}

/// `latest-linux.json`, written by `tools/publish-linux.sh`: flat keys so
/// install.sh can read it with `sed`, one URL and sha256 pair per
/// architecture (`x86_64_url`, `x86_64_sha256`, `aarch64_...`). A release
/// with no build for `arch` is an error, so a machine is never told about an
/// update it cannot install; a URL outside the release downloads is refused
/// as for the other manifests.
pub fn parse_linux_manifest(text: &str, arch: &str) -> Result<LinuxRelease, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let s = |k: &str| -> Result<String, String> {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(String::from)
            .ok_or_else(|| format!("latest-linux.json has no {k}"))
    };
    let build = v
        .get("build")
        .and_then(|b| b.as_u64())
        .ok_or("latest-linux.json has no build number")?;
    let url = s(&format!("{arch}_url"))?;
    if !DOWNLOAD_PREFIXES.iter().any(|p| url.starts_with(p)) {
        return Err(format!(
            "latest-linux.json points outside the release downloads: {url}"
        ));
    }
    s(&format!("{arch}_sha256"))?;
    Ok(LinuxRelease {
        version: s("version")?,
        build,
        tag: s("tag")?,
    })
}

/// How a Linux build got where it runs from: by install.sh (under
/// `~/.local/share/infiniterm/<version>/`, `paths::LINUX_INSTALL_ROOT`) or
/// by Homebrew (a `Cellar` folder). Anything else (a `cargo build`, an
/// unpacked tarball somewhere) gets no update notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinuxInstall {
    Script,
    Brew,
}

pub fn linux_install(exe_dir: &Path, home: &Path) -> Option<LinuxInstall> {
    if exe_dir.starts_with(home.join(crate::paths::LINUX_INSTALL_ROOT)) {
        return Some(LinuxInstall::Script);
    }
    exe_dir
        .components()
        .any(|c| c.as_os_str() == "Cellar")
        .then_some(LinuxInstall::Brew)
}

/// The one line that updates that kind of install.
pub fn linux_update_hint(install: LinuxInstall) -> &'static str {
    match install {
        LinuxInstall::Script => "curl -fsSL https://infiniterm.app/install.sh | sh",
        LinuxInstall::Brew => "brew upgrade infiniterm-linux",
    }
}

/// Where an update's zip may come from, whatever a manifest says: the
/// main repo, and the old releases repo, which the bridge release serves.
pub const DOWNLOAD_PREFIXES: [&str; 2] = [
    "https://github.com/ekinertac/infiniterm/releases/download/",
    "https://github.com/ekinertac/infiniterm-releases/releases/download/",
];

/// How often a running app looks again after the check at launch. One hour
/// (Ekin, 2026-10-15: six hours let a release go unseen for most of a
/// working day). `latest.json` is a small file on GitHub's CDN, so GitHub
/// sees up to 24 requests a day per Mac. `tools/usage-stats.py` divides by
/// this number of fetches, so it changes together with this.
pub const CHECK_EVERY_MS: f64 = 60. * 60. * 1000.;

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
    /// Hex of an Ed25519 signature over the tag, build and hash
    /// (`update_sign`). Written by `ift update-sign`; the Windows updater
    /// requires it once `UPDATE_PUBLIC_KEY` is set. The Mac manifest has none.
    pub signature: Option<String>,
}

/// The public half of the key that signs Windows release manifests, as hex
/// of its 32 bytes (`ift update-key new` prints it).
///
/// EMPTY until Ekin generates a key and commits the public half here. While
/// it is empty a Windows update is checked by its SHA-256 alone, as it was
/// before signatures existed, so merging the code changes nothing for anyone
/// until he turns it on. Once it is set, a manifest with no valid signature
/// is refused before anything is downloaded. Windows only: the Mac's check is
/// its Developer ID signature and notarization.
pub const UPDATE_PUBLIC_KEY: &str = "";

/// The rule `UPDATE_PUBLIC_KEY` makes: with no key set, nothing is required;
/// with one, the manifest must carry a signature that verifies. Taking the
/// key as an argument keeps the rule testable with a key made on the spot.
pub fn check_manifest_signature(public_key_hex: &str, m: &Manifest) -> Result<(), String> {
    if public_key_hex.trim().is_empty() {
        return Ok(());
    }
    let signature = m
        .signature
        .as_deref()
        .ok_or("the manifest is not signed, and this build requires a signature")?;
    crate::update_sign::verify(
        public_key_hex,
        &crate::update_sign::signed_message(&m.tag, m.build, &m.sha256),
        signature,
    )
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
    // Only from our release downloads, whatever the manifest says: a
    // manifest is not trusted to send the app elsewhere even though the
    // code signature check would catch a foreign build anyway.
    if !DOWNLOAD_PREFIXES.iter().any(|p| url.starts_with(p)) {
        return Err(format!(
            "latest.json points outside the release downloads: {url}"
        ));
    }
    Ok(Manifest {
        version: s("version")?,
        build,
        tag: s("tag")?,
        url,
        sha256: s("sha256")?,
        signature: v
            .get("signature")
            .and_then(|x| x.as_str())
            .map(String::from),
    })
}

/// Newer is a higher build number, nothing else: a build that goes
/// backwards (a re-published older release) is not offered.
pub fn is_newer(running_build: u64, m: &Manifest) -> bool {
    m.build > running_build
}

/// Whether an updater may act on what it is running from.
///
/// macOS: an app installed into an Applications folder. A bundle under the
/// source tree's `target/` is a development build, and replacing it with a
/// release would be a surprise; a scratch instance (`INFINITERM_DATA_DIR`)
/// is the caller's to exclude.
///
/// Windows: the folder the exe sits in, wherever somebody unpacked it,
/// because there is no install location to name. The one refusal is a
/// folder under `target`, which is a `cargo build` and not a release, and
/// where an update would replace what the next build is about to write.
#[cfg(windows)]
pub fn applies_to(folder: &Path, _home: &Path) -> bool {
    !folder
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("target"))
}

#[cfg(not(windows))]
pub fn applies_to(bundle: &Path, home: &Path) -> bool {
    bundle.extension().is_some_and(|e| e == "app")
        && bundle
            .parent()
            .is_some_and(|p| p == Path::new("/Applications") || p == home.join("Applications"))
}

/// The Windows waiter, as one PowerShell command line.
///
/// The same shape as the unix one and for the same reasons: wait for this
/// process to be gone, move the old folder aside, move the staged one in,
/// put the old one back if that fails, then start the app again. A running
/// exe cannot be overwritten on Windows but its FOLDER can be renamed, so
/// moving is what makes this possible at all.
///
/// `Wait-Process` rather than a poll: it takes the pid and returns when the
/// process ends, and a pid that is already gone is not an error worth
/// stopping for.
#[cfg(windows)]
pub fn swap_script(pid: u32, folder: &str, staged: &str, aside: &str) -> String {
    let q = |s: &str| crate::drop::quote_for(s, crate::shell_cmd::ShellKind::PowerShell);
    let (f, s, a) = (q(folder), q(staged), q(aside));
    let exe = q(&format!(
        "{folder}{}infiniterm.exe",
        std::path::MAIN_SEPARATOR
    ));
    format!(
        "Wait-Process -Id {pid} -ErrorAction SilentlyContinue;          try {{ Move-Item -LiteralPath {f} -Destination {a} -Force -ErrorAction Stop;          try {{ Move-Item -LiteralPath {s} -Destination {f} -Force -ErrorAction Stop }}          catch {{ Move-Item -LiteralPath {a} -Destination {f} -Force }} }} catch {{}};          Start-Process -FilePath {exe}"
    )
}

/// `CFBundleVersion` from an Info.plist in XML (what `tools/bundle.sh`
/// writes): the build of the app on disk.
pub fn plist_build(text: &str) -> Option<u64> {
    let after = text.split("<key>CFBundleVersion</key>").nth(1)?;
    let value = after.split("<string>").nth(1)?.split("</string>").next()?;
    value.trim().parse().ok()
}

/// Whether a staged update should replace the app on disk NOW. An update
/// staged while an older build ran can be older than what is installed by
/// the time of the restart (a `make install`, a DMG); swapping it in then
/// is a downgrade. It happened on 2026-09-26: 0.2.0 (356), staged by 355,
/// replaced a freshly installed 359 on the next restart, and two fixes
/// seemed not to work. Unknown on-disk build: the swap goes ahead, as before.
pub fn staged_is_newer(staged: u64, on_disk: Option<u64>) -> bool {
    on_disk.is_none_or(|d| staged > d)
}

/// The waiter that replaces the bundle after this process has exited, and
/// then opens it: the old bundle is moved aside (deleted at the next
/// launch, `updater::clean`), the staged one moved into its place, and if
/// the second move fails the old one goes back, so a failed update is a
/// plain restart rather than no app at all.
#[cfg(not(windows))]
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

    // Windows has no install location to name, so the rule is the opposite
    // shape: anywhere a person unpacked it, except a build tree.
    #[cfg(windows)]
    #[test]
    fn a_windows_release_folder_may_update_unless_it_is_a_build_tree() {
        let home = Path::new(r"C:\Users\PC");
        assert!(applies_to(Path::new(r"C:\Users\PC\infiniterm"), home));
        assert!(applies_to(Path::new(r"C:\Program Files\infiniterm"), home));
        assert!(applies_to(
            Path::new(r"D:\tools\infiniterm-0.1.0-337-x86_64"),
            home
        ));
        // A cargo build is not a release, whatever the case of the folder.
        assert!(!applies_to(
            Path::new(r"C:\Users\PC\Code\infiniterm\target\release"),
            home
        ));
        assert!(!applies_to(Path::new(r"C:\x\Target\debug"), home));
    }

    // The waiter has to survive a path with a space in it, which on Windows
    // is where programs live.
    #[cfg(windows)]
    #[test]
    fn the_windows_waiter_quotes_every_path_and_names_the_exe() {
        let script = swap_script(
            4321,
            r"C:\Program Files\infiniterm",
            r"C:\staged\v1",
            r"C:\staged\aside",
        );
        assert!(script.contains("Wait-Process -Id 4321"), "{script}");
        // Quoted, so the space does not split the argument.
        assert!(
            script.contains(r"'C:\Program Files\infiniterm'"),
            "{script}"
        );
        // It starts the app again from the folder it just put back.
        assert!(
            script
                .contains(r"Start-Process -FilePath 'C:\Program Files\infiniterm\infiniterm.exe'"),
            "{script}"
        );
        // The rollback is there: if the second move fails the old one returns.
        assert!(script.contains("catch"), "{script}");
    }

    // One manifest per platform, so a shipped Mac app never sees a shape it
    // does not expect.
    #[test]
    fn the_manifest_url_names_this_platforms_file() {
        if cfg!(windows) {
            assert!(LATEST_URL.ends_with("/latest-windows.json"), "{LATEST_URL}");
        } else if cfg!(target_os = "linux") {
            assert!(LATEST_URL.ends_with("/latest-linux.json"), "{LATEST_URL}");
        } else {
            assert!(LATEST_URL.ends_with("/latest.json"), "{LATEST_URL}");
        }
        assert!(LATEST_URL.starts_with("https://github.com/ekinertac/infiniterm/releases/"));
    }

    const LINUX_MANIFEST: &str = r#"{
        "version": "0.6.3", "build": 1100, "tag": "v0.6.3",
        "x86_64_url": "https://github.com/ekinertac/infiniterm/releases/download/v0.6.3/infiniterm-linux-x86_64.tar.gz",
        "x86_64_sha256": "aa",
        "aarch64_url": "https://github.com/ekinertac/infiniterm/releases/download/v0.6.3/infiniterm-linux-aarch64.tar.gz",
        "aarch64_sha256": "bb"
    }"#;

    #[test]
    fn the_linux_manifest_names_a_build_per_architecture() {
        let r = parse_linux_manifest(LINUX_MANIFEST, "x86_64").unwrap();
        assert_eq!(
            (r.version.as_str(), r.build, r.tag.as_str()),
            ("0.6.3", 1100, "v0.6.3")
        );
        assert!(parse_linux_manifest(LINUX_MANIFEST, "aarch64").is_ok());
        // No build for this machine: no notice about it.
        assert!(parse_linux_manifest(LINUX_MANIFEST, "riscv64").is_err());
        let elsewhere =
            LINUX_MANIFEST.replace("https://github.com/ekinertac", "https://evil.example");
        assert!(parse_linux_manifest(&elsewhere, "x86_64").is_err());
        assert!(parse_linux_manifest("{}", "x86_64").is_err());
    }

    #[test]
    fn a_linux_install_is_told_its_own_update_command() {
        let home = Path::new("/home/ekin");
        let script = Path::new("/home/ekin/.local/share/infiniterm/0.6.3");
        assert_eq!(linux_install(script, home), Some(LinuxInstall::Script));
        let brew = Path::new("/home/linuxbrew/.linuxbrew/Cellar/infiniterm-linux/0.6.3/libexec");
        assert_eq!(linux_install(brew, home), Some(LinuxInstall::Brew));
        // A source build or a tarball unpacked anywhere else: no notice.
        assert_eq!(
            linux_install(Path::new("/home/ekin/Code/infiniterm/target/release"), home),
            None
        );
        assert_eq!(linux_install(Path::new("/opt/infiniterm"), home), None);
        assert!(linux_update_hint(LinuxInstall::Script).contains("install.sh"));
        assert_eq!(
            linux_update_hint(LinuxInstall::Brew),
            "brew upgrade infiniterm-linux"
        );
    }

    // The usage estimate (tools/usage-stats.py) assumes this many fetches a
    // day per Mac that stays up; change them together.
    #[test]
    fn the_check_interval_is_an_hour() {
        assert_eq!(CHECK_EVERY_MS, 3_600_000.);
        assert_eq!(24. * 3_600_000. / CHECK_EVERY_MS, 24.);
    }

    use super::*;

    const GOOD: &str = r#"{
      "version": "0.1.0", "build": 1301, "tag": "v0.1.0-1301",
      "pub_date": "2026-09-23T10:00:00Z", "commit": "abc1234",
      "url": "https://github.com/ekinertac/infiniterm-releases/releases/download/v0.1.0-1301/infiniterm-0.1.0-1301-arm64.zip",
      "sha256": "00ff", "dmg": "x"
    }"#;

    /// The fixture's manifest, signed by a key made for the test, returning
    /// the public key and the manifest text with its `signature` field.
    fn signed_manifest() -> (String, String) {
        let (private, public) = crate::update_sign::generate().unwrap();
        let m = parse_manifest(GOOD).unwrap();
        let sig = crate::update_sign::sign(
            &private,
            &crate::update_sign::signed_message(&m.tag, m.build, &m.sha256),
        )
        .unwrap();
        let text = GOOD.replacen('{', &format!("{{\"signature\": \"{sig}\","), 1);
        (public, text)
    }

    #[test]
    fn a_manifest_with_no_signature_parses_and_a_signed_one_keeps_it() {
        assert_eq!(parse_manifest(GOOD).unwrap().signature, None);
        let (_, text) = signed_manifest();
        assert_eq!(parse_manifest(&text).unwrap().signature.unwrap().len(), 128);
    }

    // The switch is the constant: empty means today's behaviour, so nothing
    // changes until a key is committed.
    #[test]
    fn with_no_key_set_nothing_is_required() {
        assert_eq!(UPDATE_PUBLIC_KEY, "", "the key is Ekin's to commit");
        let m = parse_manifest(GOOD).unwrap();
        assert!(check_manifest_signature("", &m).is_ok());
        assert!(check_manifest_signature("  ", &m).is_ok());
        let (_, signed) = signed_manifest();
        assert!(check_manifest_signature("", &parse_manifest(&signed).unwrap()).is_ok());
    }

    #[test]
    fn with_a_key_set_a_valid_signature_is_required() {
        let (public, text) = signed_manifest();
        let m = parse_manifest(&text).unwrap();
        assert!(check_manifest_signature(&public, &m).is_ok());
        // Not signed at all.
        let unsigned = parse_manifest(GOOD).unwrap();
        let err = check_manifest_signature(&public, &unsigned).unwrap_err();
        assert!(err.contains("not signed"), "{err}");
        // Signed, then changed: another build, another hash, another tag.
        for tampered in [
            Manifest {
                build: m.build + 1,
                ..m.clone()
            },
            Manifest {
                sha256: "ffff".into(),
                ..m.clone()
            },
            Manifest {
                tag: "v9.9.9-9999".into(),
                ..m.clone()
            },
        ] {
            assert!(
                check_manifest_signature(&public, &tampered).is_err(),
                "{tampered:?}"
            );
        }
        // Somebody else's key.
        let (other, _) = crate::update_sign::generate().unwrap();
        assert!(check_manifest_signature(&other, &m).is_err());
        // The same text with its signature cut short.
        let broken = Manifest {
            signature: Some("abcd".into()),
            ..m
        };
        assert!(check_manifest_signature(&public, &broken).is_err());
    }

    // A staged update never downgrades the app on disk.
    #[test]
    fn a_staged_update_older_than_the_installed_app_is_not_swapped_in() {
        let plist = "<?xml version=\"1.0\"?><plist><dict>\n\t<key>CFBundleVersion</key>\n\t<string>359</string>\n</dict></plist>";
        assert_eq!(plist_build(plist), Some(359));
        assert!(
            !staged_is_newer(356, plist_build(plist)),
            "356 over 359 is a downgrade"
        );
        assert!(!staged_is_newer(359, Some(359)));
        assert!(staged_is_newer(360, Some(359)));
        assert!(staged_is_newer(360, None), "unknown: as before");
        assert_eq!(plist_build("<dict></dict>"), None);
    }

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

    // The main repo's downloads are accepted as the old releases repo's
    // were; a lookalike path on github.com is not.
    #[test]
    fn downloads_come_from_either_release_repo_and_nowhere_else() {
        let main = GOOD.replace(
            "https://github.com/ekinertac/infiniterm-releases/",
            "https://github.com/ekinertac/infiniterm/",
        );
        assert!(parse_manifest(&main).is_ok());
        assert!(parse_manifest(GOOD).is_ok());
        let lookalike = GOOD.replace(
            "https://github.com/ekinertac/infiniterm-releases/",
            "https://github.com/ekinertac/infiniterm-evil/",
        );
        assert!(parse_manifest(&lookalike).is_err());
        assert!(LATEST_URL.starts_with("https://github.com/ekinertac/infiniterm/releases/"));
    }

    // An Applications folder is a macOS idea; Windows has its own rule and
    // its own test above.
    #[cfg(not(windows))]
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

    // The sh waiter. The PowerShell one has its own test above.
    #[cfg(not(windows))]
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
