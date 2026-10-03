//! `ift licence`: register this Mac with its commercial licence key (#106).
//!
//! Optional and one-shot. `ift licence <email> <key>` asks Lemon Squeezy's
//! public validate endpoint once, judges the answer with
//! `infiniterm_core::licence::accept`, and on a yes writes `licence.json`
//! beside the save file; bare `ift licence` reads that file back without
//! touching the network. It works whether the app is running or not: the
//! About window reads the file when it opens.
//!
//! The request goes through `curl`, as the updater's does (no HTTP crate in
//! the tree), and `register` takes the fetch as an argument so the tests
//! never reach the network. The key travels on curl's stdin, not its
//! argv, so it does not show in `ps`.
//!
//! Exit codes, this command only: 3 the key or email was rejected, 4 Lemon
//! Squeezy could not be reached; 2 is bad usage as everywhere in `ift`.
use infiniterm_core::licence::{self, Licence};
use std::path::Path;
use std::process::ExitCode;

/// How long to wait for Lemon Squeezy, as curl's --max-time.
const TIMEOUT_SECS: &str = "20";

pub const REJECTED: u8 = 3;
pub const UNREACHABLE: u8 = 4;

pub fn run(args: &[String]) -> ExitCode {
    let path = licence::path();
    match args {
        [] => {
            println!("{}", status(&path));
            ExitCode::SUCCESS
        }
        [email, key] => {
            // A test-mode purchase may register a development build only.
            let allow_test = cfg!(debug_assertions);
            match register(&path, email, key, allow_test, fetch) {
                Ok(line) => {
                    println!("{line}");
                    ExitCode::SUCCESS
                }
                Err((code, msg)) => {
                    eprintln!("ift: {msg}");
                    ExitCode::from(code)
                }
            }
        }
        _ => {
            eprintln!("ift: licence takes an email and a key, or nothing");
            ExitCode::from(2)
        }
    }
}

/// What bare `ift licence` prints.
pub fn status(path: &Path) -> String {
    match licence::read(path) {
        Some(l) => licence::licensed_to(&l),
        None => format!(
            "Not registered. Personal use is free; paid work needs a licence: {}",
            licence::BUY_PAGE
        ),
    }
}

/// Checks the key and writes the file. `fetch` posts the key and returns the
/// response body, or an error when nothing usable came back.
pub fn register(
    path: &Path,
    email: &str,
    key: &str,
    allow_test: bool,
    fetch: impl Fn(&str) -> Result<String, String>,
) -> Result<String, (u8, String)> {
    let (email, key) = (email.trim(), key.trim());
    if !email.contains('@') || key.is_empty() {
        return Err((2, "usage: ift licence <email> <key>".into()));
    }
    let unreachable = |why: String| (UNREACHABLE, format!("could not reach Lemon Squeezy: {why}"));
    let body = fetch(key).map_err(unreachable)?;
    // A 5xx comes back as an HTML page, which is as good as no answer.
    let resp: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| unreachable("the answer was not JSON".into()))?;
    let name = licence::accept(&resp, email, allow_test)
        .map_err(|r| (REJECTED, format!("{}; nothing was saved", r.message())))?;
    let l = Licence {
        email: email.to_string(),
        key: key.to_string(),
        name,
        checked_at: licence::now(),
    };
    licence::write(path, &l).map_err(|e| (2, format!("cannot write {}: {e}", path.display())))?;
    Ok(licence::licensed_to(&l))
}

/// The one network request: curl, the body on stdout. Without `-f`, so a
/// 404 for an unknown key still hands back its JSON.
fn fetch(key: &str) -> Result<String, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("curl")
        .args(["-sS", "--max-time", TIMEOUT_SECS, "-H", "Accept: application/json"])
        .args(["--data-binary", "@-", licence::VALIDATE_URL])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run curl: {e}"))?;
    let form = format!("license_key={}", form_encode(key));
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(form.as_bytes()).map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Percent-encodes everything but letters, digits, `-` and `_`: a key is
/// hex and dashes, but whatever was pasted must not break the form.
fn form_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ift-licence-cmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("licence.json")
    }

    fn answer(test_mode: bool, email: &str) -> String {
        serde_json::json!({
            "valid": true,
            "license_key": {"status": "active", "test_mode": test_mode},
            "meta": {
                "store_id": licence::STORE_ID,
                "product_name": licence::PRODUCT_NAME,
                "customer_name": "Ada Lovelace",
                "customer_email": email
            }
        })
        .to_string()
    }

    // One test, run in order, since the steps share the scratch file.
    #[test]
    fn register_writes_only_on_a_yes_and_exits_by_the_issue() {
        let p = scratch();
        assert!(status(&p).starts_with("Not registered."));

        // Unreachable: 4, nothing written.
        let r = register(&p, "ada@example.com", "K", false, |_| Err("timed out".into()));
        assert_eq!(r.unwrap_err().0, UNREACHABLE);
        let r = register(&p, "ada@example.com", "K", false, |_| Ok("<html>502</html>".into()));
        assert_eq!(r.unwrap_err().0, UNREACHABLE);
        assert!(!p.exists());

        // Rejected: 3, nothing written.
        let r = register(&p, "bob@example.com", "K", false, |_| Ok(answer(false, "ada@example.com")));
        assert_eq!(r.unwrap_err().0, REJECTED);
        let r = register(&p, "ada@example.com", "K", false, |_| Ok(answer(true, "ada@example.com")));
        assert_eq!(r.unwrap_err().0, REJECTED);
        assert!(!p.exists());

        // Bad usage never asks the network.
        let r = register(&p, "not-an-email", "K", false, |_| panic!("no request"));
        assert_eq!(r.unwrap_err().0, 2);

        // A yes: the key it was asked about, the file, the line.
        let r = register(&p, "ada@example.com", " K-1 ", false, |k| {
            assert_eq!(k, "K-1");
            Ok(answer(false, "ada@example.com"))
        });
        assert_eq!(r, Ok("Licensed to Ada Lovelace (ada@example.com)".into()));
        assert_eq!(status(&p), "Licensed to Ada Lovelace (ada@example.com)");
        assert_eq!(licence::read(&p).unwrap().key, "K-1");
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_key_is_form_encoded() {
        assert_eq!(form_encode("ABC123-DEF0_9"), "ABC123-DEF0_9");
        assert_eq!(form_encode("a b&c=d"), "a%20b%26c%3Dd");
    }
}
