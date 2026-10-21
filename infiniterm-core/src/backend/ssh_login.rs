//! The login step of `ift join` (#450): the one place ssh may ask a person a
//! question, so that every connection after it can run in `BatchMode`.
//!
//! The viewer opens about one ssh per card and none of them can ask anything.
//! Before the first one, `ift join` probes the host without asking
//! (`probe`). When the probe fails for a reason that a question fixes (the
//! host was never connected to, no key is set up), and there is a terminal,
//! `login_step` runs ONE ordinary `ssh <host> true` with the terminal's own
//! input and output: ssh asks its own questions (the fingerprint, a password,
//! a key's passphrase) and records the answer in `known_hosts`. Then the probe
//! runs again.
//!
//! A host key that CHANGED is never a question: it is an error naming the host
//! and the new fingerprint, and nothing here (or anywhere) passes
//! `StrictHostKeyChecking=no`. A password is never read, kept or logged here;
//! ssh reads it from the terminal itself.
//!
//! Called by `infiniterm-cli/src/join.rs` before `remote::check`; the
//! classification is also used by `remote::classify_join_failure`, so a viewer
//! that meets a changed key later says the same thing. Related: remote.rs
//! (`RemoteHost`, the ssh arguments), #442 (the other failure states).
//!
//! Not here yet (#450's later steps): a password host's shared connection, and
//! the app's own dialogs when there is no terminal.

use std::process::{Command, Stdio};

use super::remote::RemoteHost;

/// What ssh's own words say went wrong with a login attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshProblem {
    /// The host was never connected to: its key is not in `known_hosts`.
    UnknownHost,
    /// The host's key is not the one `known_hosts` has. Carries the new
    /// fingerprint when ssh printed it.
    ChangedKey(Option<String>),
    /// The host refused every key and password-less way in.
    NeedsLogin,
    /// Anything else (an unreachable host, a timeout): not a question's work.
    Other,
}

/// Reads ssh's stderr. Matched in lower case; the changed-key warning wins
/// over the unknown-host line that follows it ("Host key verification
/// failed." closes both).
pub fn classify(stderr: &str) -> SshProblem {
    let lower = stderr.to_lowercase();
    if lower.contains("remote host identification has changed")
        || lower.contains("host key for") && lower.contains("has changed")
    {
        return SshProblem::ChangedKey(fingerprint_in(stderr));
    }
    if lower.contains("host key verification failed")
        || lower.contains("host key is known") && lower.contains("strict checking")
    {
        return SshProblem::UnknownHost;
    }
    if lower.contains("permission denied") {
        return SshProblem::NeedsLogin;
    }
    SshProblem::Other
}

/// The new key's fingerprint from ssh's warning: the first `SHA256:` or `MD5:`
/// token. ssh prints "The fingerprint for the ED25519 key sent by the remote
/// host is" and then the value, on that line or the next.
fn fingerprint_in(stderr: &str) -> Option<String> {
    let mut lines = stderr.lines();
    while let Some(line) = lines.next() {
        if !line.to_lowercase().contains("fingerprint") {
            continue;
        }
        for text in std::iter::once(line).chain(lines.clone().take(1)) {
            if let Some(token) = text
                .split_whitespace()
                .find(|w| w.starts_with("SHA256:") || w.starts_with("MD5:"))
            {
                return Some(token.trim_end_matches(['.', ',']).to_string());
            }
        }
    }
    None
}

/// What a person is told for a changed key. Never softened into a question.
pub fn changed_key_sentence(target: &str, fingerprint: Option<&str>) -> String {
    let new = fingerprint
        .map(|f| format!(" (it now shows {f})"))
        .unwrap_or_default();
    format!(
        "the host key of {target} has changed{new}. That is either a reinstalled host or someone in between: check it with the host's owner, then run `ssh-keygen -R {target}` and join again. infiniterm will not accept a changed key for you"
    )
}

/// One ssh that must not ask: the same arguments the viewer's connections
/// use, running `true`. The stderr comes back for `classify`.
pub fn probe(host: &RemoteHost) -> std::io::Result<Result<(), String>> {
    let out = Command::new(&host.ssh)
        .args(super::remote::ssh_args_for(host, "true".into()))
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    Ok(if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    })
}

/// How long an idle shared connection lives on its own. The backstop for a
/// crash: a normal quit stops it (`stop_master`).
const MASTER_PERSIST: &str = "10m";

/// Where a host's shared connection (OpenSSH's control socket) lives: a short
/// path, because a unix socket path holds 104 bytes and ssh adds 17 of its own
/// while it makes the socket. `/tmp/iftc-<uid>/<hash of the target>`, the
/// folder owner-only, so nobody else can ride the connection.
pub fn control_path(target: &str, uid: u32) -> std::path::PathBuf {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in target.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    std::path::PathBuf::from(format!("/tmp/iftc-{uid}/{:08x}", h as u32))
}

/// The folder for `control_path`, made owner-only.
fn make_control_dir(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = path.parent().expect("a control path has a folder");
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// One ordinary ssh with the caller's terminal that also becomes the shared
/// connection: ssh asks what it needs (the fingerprint, the password, once),
/// runs `true`, and stays on as the master for later connections.
fn ask_master(
    host: &RemoteHost,
    control: &std::path::Path,
    askpass: Option<&std::path::Path>,
) -> std::io::Result<bool> {
    make_control_dir(control)?;
    let mut args = vec![
        "-o".to_string(),
        "ConnectTimeout=15".to_string(),
        "-M".to_string(),
        "-o".to_string(),
        format!("ControlPath={}", control.display()),
        "-o".to_string(),
        format!("ControlPersist={MASTER_PERSIST}"),
    ];
    args.extend(host.ssh_args.iter().cloned());
    args.push(host.target.clone());
    args.push("true".into());
    let mut command = Command::new(&host.ssh);
    command.args(args);
    match askpass {
        // No terminal: ssh runs the helper for each question (the
        // fingerprint, the password) and reads the answer from its output.
        Some(helper) => {
            command
                .env("SSH_ASKPASS", helper)
                .env("SSH_ASKPASS_REQUIRE", "force")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit());
        }
        None => {
            command
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
        }
    }
    Ok(command.status()?.success())
}

/// What an ssh question wants back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// "Are you sure you want to continue connecting (yes/no/[fingerprint])?"
    Confirm,
    /// A password or a key's passphrase: typed, never shown.
    Secret,
}

/// ssh passes the question as the helper's one argument.
pub fn prompt_kind(prompt: &str) -> PromptKind {
    if prompt.contains("(yes/no") {
        PromptKind::Confirm
    } else {
        PromptKind::Secret
    }
}

/// The `osascript` arguments that show `prompt`. The prompt is the LAST
/// argument, read inside the script as `item 1 of argv`, never spliced into
/// the script's text: a host name or a banner cannot become script.
pub fn dialog_args(kind: PromptKind) -> Vec<String> {
    let body = match kind {
        PromptKind::Secret => {
            "set r to display dialog (item 1 of argv) default answer \"\" with hidden answer with title \"infiniterm: ssh\" buttons {\"Cancel\", \"OK\"} default button \"OK\"\nreturn text returned of r"
        }
        PromptKind::Confirm => {
            "set r to display dialog (item 1 of argv) with title \"infiniterm: ssh\" buttons {\"No\", \"Yes\"} default button \"No\"\nreturn button returned of r"
        }
    };
    let mut args = vec!["-e".to_string(), "on run argv".to_string()];
    for line in body.lines() {
        args.push("-e".into());
        args.push(line.to_string());
    }
    args.push("-e".into());
    args.push("end run".into());
    args.push("--".into());
    args
}

/// The helper `ift askpass <question>`: shows the question in a dialog and
/// returns the answer for ssh. A password goes from the dialog to ssh and
/// nowhere else: not to a file, not to the log. `Err` when the dialog was
/// cancelled, which ssh reads as a failed login.
pub fn run_askpass(prompt: &str) -> Result<String, String> {
    let kind = prompt_kind(prompt);
    let mut args = dialog_args(kind);
    args.push(prompt.to_string());
    let out = Command::new("osascript")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("could not show the dialog: {e}"))?;
    if !out.status.success() {
        return Err("cancelled".into());
    }
    let said = String::from_utf8_lossy(&out.stdout)
        .trim_end_matches('\n')
        .to_string();
    Ok(match kind {
        PromptKind::Confirm => said.to_lowercase(),
        PromptKind::Secret => said,
    })
}

/// A script ssh can run as `SSH_ASKPASS` (it is executed with the question as
/// its only argument): `ift askpass "$@"`. It holds the path of `ift`, no
/// secret. Owner-only, beside the control socket.
pub fn write_askpass_helper(
    ift: &std::path::Path,
    control_dir_of: &std::path::Path,
) -> std::io::Result<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    make_control_dir(control_dir_of)?;
    let path = control_dir_of
        .parent()
        .expect("a control path has a folder")
        .join("askpass.sh");
    let quoted = crate::drop::posix_quote(&ift.display().to_string());
    std::fs::write(&path, format!("#!/bin/sh\nexec {quoted} askpass \"$@\"\n"))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

/// What the login step left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Login {
    /// The host accepts a login with no question: one ssh per card, as ever.
    Keys,
    /// The host needs a password. One shared connection holds it, and every
    /// ssh of the viewer must name this control path (`-o ControlPath=...`,
    /// already added to `host.ssh_args`); `stop_master` ends it.
    Shared(std::path::PathBuf),
}

/// Ends a host's shared connection, if there is one: what a normal quit does,
/// so nothing is left running (a crash leaves it at most `MASTER_PERSIST`).
/// Quiet about everything: there may be none.
pub fn stop_master(host: &RemoteHost, control: &std::path::Path) {
    if !control.exists() {
        return;
    }
    let mut args = vec![
        "-o".to_string(),
        format!("ControlPath={}", control.display()),
        "-O".to_string(),
        "exit".to_string(),
    ];
    args.extend(host.ssh_args.iter().cloned());
    args.push(host.target.clone());
    let _ = Command::new(&host.ssh)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Makes the host reachable without a question, asking once if it must.
/// `terminal` says whether a person can answer; without one the way out is
/// said in a sentence. `Ok` means the viewer's own connections will work.
///
/// A host that needs a password cannot be asked 37 times, once per card, so
/// the same question that asks for it also leaves the shared connection
/// (`Login::Shared`). A host with keys never gets one: a shared connection
/// stalls every card behind a flooding one (measured, 2026-10-18).
pub fn login_step(
    host: &mut RemoteHost,
    terminal: bool,
    uid: u32,
    askpass: Option<&std::path::Path>,
) -> Result<Login, String> {
    let target = host.target.clone();
    let target = &target;
    let first = probe(host).map_err(|e| format!("could not run {}: {e}", host.ssh))?;
    let Err(said) = first else {
        return Ok(Login::Keys);
    };
    match classify(&said) {
        SshProblem::ChangedKey(fingerprint) => {
            Err(changed_key_sentence(target, fingerprint.as_deref()))
        }
        SshProblem::Other => Err(format!(
            "{target}: {}",
            said.lines().last().unwrap_or("ssh failed")
        )),
        need @ (SshProblem::UnknownHost | SshProblem::NeedsLogin) => {
            // No terminal and no helper: the way out is a sentence.
            if !terminal && askpass.is_none() {
                return Err(match need {
                    SshProblem::UnknownHost => format!(
                        "{target} has not been connected to from this Mac. Run `ssh {target}` once in a terminal, answer its question, then join again"
                    ),
                    _ => format!(
                        "{target} did not accept a login without a prompt ({}). Set up key login (`ssh-copy-id {target}`), or run `ift join` from a terminal",
                        said.lines().last().unwrap_or("ssh refused")
                    ),
                });
            }
            eprintln!("ift: {target} needs a login first; ssh will ask.");
            let control = control_path(target, uid);
            // With a terminal ssh asks there; without one it asks through
            // the helper's dialogs.
            let done = ask_master(host, &control, if terminal { None } else { askpass })
                .map_err(|e| format!("could not run {}: {e}", host.ssh))?;
            if !done {
                return Err(format!("{target}: the login did not finish"));
            }
            // The answer is in known_hosts (or the key unlocked). Does the
            // viewer's own connection work with no question now? Then the
            // shared connection is not wanted: end it.
            let plain = probe(host).map_err(|e| format!("could not run {}: {e}", host.ssh))?;
            match plain {
                Ok(()) => {
                    stop_master(host, &control);
                    Ok(Login::Keys)
                }
                Err(again) => match classify(&again) {
                    SshProblem::ChangedKey(f) => {
                        stop_master(host, &control);
                        Err(changed_key_sentence(target, f.as_deref()))
                    }
                    SshProblem::NeedsLogin => {
                        // A password host: every ssh rides the shared one.
                        host.ssh_args.push("-o".into());
                        host.ssh_args
                            .push(format!("ControlPath={}", control.display()));
                        match probe(host).map_err(|e| format!("could not run {}: {e}", host.ssh))? {
                            Ok(()) => Ok(Login::Shared(control)),
                            Err(last) => {
                                stop_master(host, &control);
                                Err(format!(
                                    "{target}: the shared connection did not work: {}",
                                    last.lines().last().unwrap_or("ssh failed")
                                ))
                            }
                        }
                    }
                    _ => {
                        stop_master(host, &control);
                        Err(format!(
                            "{target}: {}",
                            again.lines().last().unwrap_or("ssh failed")
                        ))
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const CHANGED: &str = "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\nIT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!\nThe fingerprint for the ED25519 key sent by the remote host is\nSHA256:abc123DEF456.\nPlease contact your system administrator.\nHost key verification failed.";

    #[test]
    fn ssh_words_are_sorted_into_what_a_question_can_fix() {
        assert_eq!(
            classify("Host key verification failed."),
            SshProblem::UnknownHost
        );
        assert_eq!(
            classify("No ED25519 host key is known for h and you have requested strict checking.\nHost key verification failed."),
            SshProblem::UnknownHost
        );
        assert_eq!(
            classify("test@h: Permission denied (publickey,password)."),
            SshProblem::NeedsLogin
        );
        assert_eq!(
            classify("ssh: connect to host h port 22: Operation timed out"),
            SshProblem::Other
        );
    }

    #[test]
    fn a_changed_key_is_recognised_with_its_fingerprint_and_wins_over_the_unknown_line() {
        assert_eq!(
            classify(CHANGED),
            SshProblem::ChangedKey(Some("SHA256:abc123DEF456".into()))
        );
        assert_eq!(
            classify("WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!"),
            SshProblem::ChangedKey(None)
        );
        let said = changed_key_sentence("mini", Some("SHA256:abc"));
        assert!(
            said.contains("mini")
                && said.contains("SHA256:abc")
                && said.contains("ssh-keygen -R mini")
        );
    }

    /// Waits until a script just written can be executed. A parallel test's
    /// `fork` can hold a copy of this file's write descriptor for a moment, and
    /// exec of a file open for writing fails with "Text file busy" (ETXTBSY):
    /// the rare CI failure of `an_unreachable_host_is_not_a_question` (#457).
    /// The warm-up run's own result does not matter.
    fn settle(path: &std::path::Path) {
        const ETXTBSY: i32 = 26;
        for _ in 0..200 {
            match std::process::Command::new(path)
                .arg("--settle")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                Err(e) if e.raw_os_error() == Some(ETXTBSY) => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                _ => return,
            }
        }
    }

    /// A uid no real user has, so the control folder of a test is its own.
    const TEST_UID: u32 = 4_000_000_001;

    /// A stand-in ssh. The interactive run (`-M`) records that it ran and
    /// makes the control socket file. `-O exit` records the stop. A probe
    /// (`BatchMode=yes`) with a ControlPath passes once the interactive run
    /// happened; without one it fails as `batch_fail` says, unless the host
    /// takes keys (`plain_after_ask`) and the interactive run happened.
    fn stand_in(
        name: &str,
        batch_fail: &str,
        interactive_ok: bool,
        plain_after_ask: bool,
    ) -> (RemoteHost, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("sl-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("asked");
        let script = dir.join("ssh");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
case "$*" in
*"-O exit"*) touch '{d}/exited'; exit 0;;
*" -M "*)
  cp=$(echo "$*" | sed -n 's/.*ControlPath=\([^ ]*\).*/\1/p')
  touch '{m}'; touch "$cp"; exit {code};;
*BatchMode=yes*ControlPath=*) [ -e '{m}' ] && exit 0;;
*BatchMode=yes*) [ -e '{m}' ] && [ {plain} = 1 ] && exit 0;;
esac
cat <<'EOF' >&2
{fail}
EOF
exit 255
"#,
                d = dir.display(),
                m = marker.display(),
                fail = batch_fail,
                code = if interactive_ok { 0 } else { 1 },
                plain = plain_after_ask as u8
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        settle(&script);
        let mut host = RemoteHost::new(&format!("test@mini-{name}"));
        host.ssh = script.display().to_string();
        let _ = std::fs::remove_file(control_path(&host.target, TEST_UID));
        (host, marker)
    }

    #[test]
    fn a_host_that_works_without_a_question_is_never_asked() {
        let (mut host, marker) = stand_in("quiet", "", true, true);
        std::fs::write(&marker, "").unwrap(); // the probe will pass
        assert_eq!(login_step(&mut host, true, TEST_UID, None), Ok(Login::Keys));
    }

    #[test]
    fn a_host_never_seen_is_asked_once_in_the_terminal_and_then_works_with_keys() {
        let (mut host, marker) = stand_in("unknown", "Host key verification failed.", true, true);
        assert_eq!(login_step(&mut host, true, TEST_UID, None), Ok(Login::Keys));
        assert!(marker.exists(), "the interactive ssh ran");
        // A host with keys gets no shared connection: it was stopped.
        assert!(marker.parent().unwrap().join("exited").exists());
        assert!(!host.ssh_args.iter().any(|a| a.starts_with("ControlPath=")));
    }

    #[test]
    fn a_password_host_is_asked_once_and_every_ssh_rides_the_shared_connection() {
        let (mut host, marker) = stand_in(
            "password",
            "test@mini: Permission denied (password).",
            true,
            false,
        );
        let got = login_step(&mut host, true, TEST_UID, None).unwrap();
        let control = control_path(&host.target, TEST_UID);
        assert_eq!(got, Login::Shared(control.clone()));
        assert!(marker.exists());
        assert!(
            host.ssh_args
                .contains(&format!("ControlPath={}", control.display())),
            "the viewer's ssh arguments name the shared connection: {:?}",
            host.ssh_args
        );
        // Kept for the viewer: not stopped; a quit stops it.
        assert!(!marker.parent().unwrap().join("exited").exists());
        stop_master(&host, &control);
        assert!(marker.parent().unwrap().join("exited").exists());
        let _ = std::fs::remove_file(control);
    }

    #[test]
    fn the_control_path_is_short_stable_per_host_and_in_an_owner_only_folder() {
        let a = control_path("mini", 501);
        assert_eq!(a, control_path("mini", 501));
        assert_ne!(a, control_path("other", 501));
        // ssh adds 17 characters while it makes the socket; 104 is the limit.
        assert!(a.as_os_str().len() + 17 < 104, "{}", a.display());
        make_control_dir(&control_path("mini", 4_000_000_002)).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata("/tmp/iftc-4000000002")
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        let _ = std::fs::remove_dir("/tmp/iftc-4000000002");
    }

    #[test]
    fn a_question_is_a_confirm_or_a_secret_and_never_becomes_script_text() {
        assert_eq!(
            prompt_kind("The authenticity of host 'h' can't be established.\nAre you sure you want to continue connecting (yes/no/[fingerprint])?"),
            PromptKind::Confirm
        );
        assert_eq!(prompt_kind("test@mini's password: "), PromptKind::Secret);
        assert_eq!(
            prompt_kind("Enter passphrase for key '/Users/me/.ssh/id_ed25519': "),
            PromptKind::Secret
        );
        for kind in [PromptKind::Confirm, PromptKind::Secret] {
            let args = dialog_args(kind);
            assert_eq!(args.last().map(String::as_str), Some("--"));
            assert!(args.iter().any(|a| a.contains("item 1 of argv")));
            // The question is appended after "--" by the caller, so even a
            // question holding quotes and `end run` is only a string.
        }
        assert!(dialog_args(PromptKind::Secret)
            .iter()
            .any(|a| a.contains("hidden answer")));
    }

    #[test]
    fn the_askpass_helper_runs_ift_askpass_and_holds_no_secret() {
        use std::os::unix::fs::PermissionsExt;
        let control = control_path("helper-test", 4_000_000_003);
        let ift = std::path::Path::new("/Applications/my apps/infiniterm.app/ift");
        let path = write_askpass_helper(ift, &control).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("askpass \"$@\""), "{text}");
        assert!(
            text.contains("'/Applications/my apps/infiniterm.app/ift'"),
            "{text}"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let _ = std::fs::remove_dir_all("/tmp/iftc-4000000003");
    }

    #[test]
    fn without_a_terminal_the_helpers_dialogs_answer_ssh_and_a_password_host_is_shared() {
        let dir = std::env::temp_dir().join(format!("sl-{}-dialog", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The helper "types" the password; the stand-in ssh accepts only it.
        let helper = dir.join("askpass");
        std::fs::write(&helper, "#!/bin/sh\necho hunter2\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let script = dir.join("ssh");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
case "$*" in
*" -M "*)
  [ "$SSH_ASKPASS_REQUIRE" = force ] || exit 1
  ans=$("$SSH_ASKPASS" "test@mini's password: ")
  [ "$ans" = hunter2 ] || exit 1
  cp=$(echo "$*" | sed -n 's/.*ControlPath=\([^ ]*\).*/\1/p')
  touch '{d}/asked'; touch "$cp"; exit 0;;
*BatchMode=yes*ControlPath=*) [ -e '{d}/asked' ] && exit 0;;
esac
echo 'test@mini: Permission denied (password).' >&2
exit 255
"#,
                d = dir.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        settle(&helper);
        settle(&script);
        let mut host = RemoteHost::new("test@mini-dialog");
        host.ssh = script.display().to_string();
        let control = control_path(&host.target, TEST_UID);
        let _ = std::fs::remove_file(&control);
        let got = login_step(&mut host, false, TEST_UID, Some(&helper)).unwrap();
        assert_eq!(got, Login::Shared(control.clone()));
        assert!(dir.join("asked").exists());
        let _ = std::fs::remove_file(control);
    }

    #[test]
    fn stopping_a_connection_that_is_not_there_does_nothing() {
        let (host, marker) = stand_in("nomaster", "", true, true);
        stop_master(&host, &control_path(&host.target, TEST_UID));
        assert!(!marker.parent().unwrap().join("exited").exists());
    }

    #[test]
    fn without_a_terminal_a_host_never_seen_says_what_to_do_and_asks_nothing() {
        let (mut host, marker) = stand_in("noterm", "Host key verification failed.", true, true);
        let err = login_step(&mut host, false, TEST_UID, None).unwrap_err();
        assert!(
            err.contains("ssh test@mini") && err.contains("join again"),
            "{err}"
        );
        assert!(!marker.exists());
    }

    #[test]
    fn a_changed_key_is_an_error_and_nothing_is_asked() {
        let (mut host, marker) = stand_in("changed", CHANGED, true, true);
        let err = login_step(&mut host, true, TEST_UID, None).unwrap_err();
        assert!(
            err.contains("has changed") && err.contains("SHA256:abc123DEF456"),
            "{err}"
        );
        assert!(!marker.exists(), "no question about a changed key");
    }

    #[test]
    fn a_login_that_does_not_finish_is_said() {
        let (mut host, _) = stand_in(
            "refused",
            "test@mini: Permission denied (publickey).",
            false,
            false,
        );
        let err = login_step(&mut host, true, TEST_UID, None).unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
    }

    #[test]
    fn an_unreachable_host_is_not_a_question() {
        let (mut host, marker) = stand_in(
            "down",
            "ssh: connect to host mini port 22: Operation timed out",
            true,
            true,
        );
        let err = login_step(&mut host, true, TEST_UID, None).unwrap_err();
        assert!(err.contains("timed out"), "{err}");
        assert!(!marker.exists());
    }
}
