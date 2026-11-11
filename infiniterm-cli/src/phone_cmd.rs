//! `ift phone add | list | remove`, and `ift phone-gate`, `ift read --stream`,
//! `ift settings show`, `ift snippets --nul`: pairing the iPhone app with this
//! Mac over its own sshd. No listener, no new way in.
//!
//! `phone add <name>` makes an ed25519 key with `ssh-keygen`, appends its
//! public half to `~/.ssh/authorized_keys` restricted to the forced command
//! `ift phone-gate`, and prints a QR code with the private seed, the sshd host
//! key and this Mac's address. Nothing about the phone is kept here except
//! that one authorized_keys line, which `list` and `remove` find by its tag.
//!
//! `phone-gate` is what sshd runs for such a key: it reads
//! `$SSH_ORIGINAL_COMMAND`, accepts only the exact commands the app sends
//! (`infiniterm_core::phone::gate_args`), and runs this same binary with them,
//! never a shell. `read --stream`, `settings show` and `snippets --nul` exist
//! so those three phone needs are `ift` verbs, not shell strings: their output
//! bytes are the ones the phone's old shell commands printed.
//!
//! The logic (line editing, URL, key parsing, the gate's table) is
//! `infiniterm_core::phone`, tested there. This file is processes and files.
//! Related: push_cmd.rs (the other phone verb), main.rs (dispatch).

use std::io::{BufRead, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use infiniterm_core::phone;

const ADD_USAGE: &str = "ift: phone add <name> [--host <name>] [--port <n>]";
const USAGE: &str = "ift: phone takes add <name>, list, or remove <name>";
const SSHD_ADDR: &str = "127.0.0.1";
/// This Mac's own sshd: where the forward goes and what `phone add` checks.
const SSHD_PORT: u16 = 22;
const TAILSCALE_PATH_CMD: &str = "tailscale";
/// The sshd host key the phone pins.
const HOST_KEY: &str = "/etc/ssh/ssh_host_ed25519_key.pub";
const TAILSCALE_APP: &str = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";

pub fn run(args: &[String]) -> ExitCode {
    let result = match args.first().map(String::as_str) {
        Some("add") => add(&args[1..]),
        Some("list") if args.len() == 1 => list(),
        Some("remove") => match args {
            [_, name] => remove(name),
            _ => Err(USAGE.to_string()),
        },
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", if e.starts_with("ift:") { e } else { format!("ift: {e}") });
            ExitCode::from(2)
        }
    }
}

fn home() -> PathBuf {
    infiniterm_core::paths::home_dir()
}

fn authorized_keys_path() -> PathBuf {
    home().join(".ssh").join("authorized_keys")
}

/// This binary's real path: what the phone is told to call, and what the gate
/// compares the phone's first word with.
fn own_path() -> Result<String, String> {
    std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| format!("cannot find this ift: {e}"))
}

fn add(args: &[String]) -> Result<(), String> {
    let mut name = None;
    let mut host = None;
    let mut port: Option<u16> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--host" => host = Some(it.next().ok_or(ADD_USAGE)?.clone()),
            "--port" => {
                port = Some(
                    it.next()
                        .and_then(|p| p.parse().ok())
                        .ok_or("ift: --port takes a number")?,
                )
            }
            n if name.is_none() && !n.starts_with("--") => name = Some(n.to_string()),
            _ => return Err(ADD_USAGE.into()),
        }
    }
    let name = name.ok_or(ADD_USAGE)?;
    if !phone::valid_device_name(&name) {
        return Err("a device name is 1 to 32 letters, digits, - or _".into());
    }
    sshd_is_on(SSHD_PORT)?;
    let keys = authorized_keys_path();
    check_ssh_dir(&keys)?;
    let existing = std::fs::read_to_string(&keys).unwrap_or_default();
    if phone::paired_names(&existing).contains(&name) {
        return Err(format!("a phone called {name} is already paired: ift phone remove {name}"));
    }
    let host_key = std::fs::read_to_string(HOST_KEY)
        .ok()
        .and_then(|t| phone::host_key_blob(&t))
        .ok_or_else(|| format!("cannot read the sshd host key {HOST_KEY}"))?;
    let ift = own_path()?;
    // Before the key is made or written: a forward that cannot be set up
    // leaves nothing behind.
    let (port, note) = match port {
        Some(p) => (p, None),
        None => ssh_port_for_phone()?,
    };
    let (seed, public) = make_key()?;
    let line = phone::authorized_line(&ift, &public, &name)?;
    let text = phone::add_line(&existing, &line, &name)?;
    write_private(&keys, &text)?;
    let host = host.unwrap_or_else(default_host);
    let url = phone::pair_url(&phone::Pairing {
        name: &computer_name(),
        host: &host,
        port,
        user: &user(),
        host_key_blob: &host_key,
        seed: &seed,
        device: &name,
        ift: &ift,
        ift_version: env!("CARGO_PKG_VERSION"),
    });
    print_qr(&url)?;
    println!("\nScan this with the infiniterm app on the phone ({name}, to {host}).");
    println!("The code holds the phone's private key: anyone who sees it can use it.");
    println!("This terminal card's session buffer keeps it too. If anyone saw it, run:");
    println!("  ift phone remove {name}");
    if let Some(note) = note {
        println!("\n{note}");
    }
    Ok(())
}

// ------------------------------------------------------------ Tailscale SSH

/// The Tailscale command line that answers, if there is one: on `$PATH`, or
/// inside the Mac app.
fn tailscale() -> Option<&'static str> {
    [TAILSCALE_PATH_CMD, TAILSCALE_APP]
        .into_iter()
        .find(|c| run_text(c, &["version"]).is_some())
}

/// The ssh port the phone is told to use. 22, unless Tailscale SSH is on: it
/// takes the tailnet address on port 22, ignores `authorized_keys` (so the
/// phone's limited key never applies) and wants a login page the app cannot
/// show. Then a tailnet port is forwarded to this Mac's own sshd with
/// `tailscale serve`, the same one an earlier pairing made if there is one.
fn ssh_port_for_phone() -> Result<(u16, Option<String>), String> {
    let Some(ts) = tailscale() else {
        return Ok((SSHD_PORT, None));
    };
    let on = run_text(ts, &["debug", "prefs"]).is_some_and(|p| phone::tailscale_ssh_on(&p));
    if !on {
        return Ok((SSHD_PORT, None));
    }
    let status = run_text(ts, &["serve", "status", "--json"]).unwrap_or_default();
    let (port, made) = match phone::plan_forward(&status) {
        phone::Forward::Existing(p) => (p, false),
        phone::Forward::New(p) => {
            let target = format!("tcp://localhost:{SSHD_PORT}");
            let tcp = format!("--tcp={p}");
            let out = Command::new(ts)
                .args(["serve", "--bg", &tcp, &target])
                .output()
                .map_err(|e| format!("cannot run tailscale: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "Tailscale SSH is on, so the phone needs a tailnet port that reaches this Mac's ssh, and tailscale serve refused it: {}\nrun once: tailscale set --operator=$USER, or pass --port <n> to use a port you set up yourself",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            (p, true)
        }
    };
    if made {
        // The config folder may not exist yet on a fresh Mac; without the
        // marker `phone remove` could never close the port.
        let marker = forward_marker();
        if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&marker, port.to_string()) {
            eprintln!("ift: could not note the forward in {}: {e}", marker.display());
        }
    }
    Ok((
        port,
        Some(format!(
            "Tailscale SSH is on and takes port 22 on the tailnet, so the phone connects on port {port}, which forwards to this Mac's own ssh.\nThis Mac's ssh also allows password login, and from your tailnet that login is reachable on port {port} too. To close the port: tailscale serve --tcp={port} off\n(ift phone remove closes it when the last phone is removed)."
        )),
    ))
}

/// Where `phone add` notes a forward it made, so `phone remove` of the last
/// phone takes only that one away.
fn forward_marker() -> PathBuf {
    infiniterm_core::paths::config_dir().join("phone-forward")
}

/// After the last phone is removed: the forward `phone add` made goes too.
fn close_forward() {
    let marker = forward_marker();
    let Some(port) = std::fs::read_to_string(&marker)
        .ok()
        .and_then(|t| t.trim().parse::<u16>().ok())
    else {
        return;
    };
    if let Some(ts) = tailscale() {
        let off = Command::new(ts)
            .args(["serve", &format!("--tcp={port}"), "off"])
            .output();
        if off.is_ok_and(|o| o.status.success()) {
            println!("closed the tailnet port {port}");
        } else {
            eprintln!("ift: could not close the tailnet port {port}: tailscale serve --tcp={port} off");
            return;
        }
    }
    let _ = std::fs::remove_file(marker);
}

fn list() -> Result<(), String> {
    let text = std::fs::read_to_string(authorized_keys_path()).unwrap_or_default();
    for n in phone::paired_names(&text) {
        println!("{n}");
    }
    Ok(())
}

fn remove(name: &str) -> Result<(), String> {
    let keys = authorized_keys_path();
    let text = std::fs::read_to_string(&keys).map_err(|_| format!("no phone called {name}"))?;
    let (kept, found) = phone::remove_line(&text, name);
    if !found {
        return Err(format!("no phone called {name}"));
    }
    write_private(&keys, &kept)?;
    println!("removed {name}");
    if phone::paired_names(&kept).is_empty() {
        close_forward();
    }
    Ok(())
}

/// Remote Login is on when something answers on the sshd port.
fn sshd_is_on(port: u16) -> Result<(), String> {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    let addr: SocketAddr = format!("{SSHD_ADDR}:{port}").parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map(|_| ())
        .map_err(|_| {
            "nothing answers on the ssh port: turn on System Settings > General > Sharing > Remote Login".into()
        })
}

/// `~/.ssh` and `authorized_keys` must exist with modes sshd accepts. A
/// missing folder or file is made; a loose one is said, not changed.
fn check_ssh_dir(keys: &Path) -> Result<(), String> {
    let dir = keys.parent().unwrap();
    if !dir.exists() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    if !keys.exists() {
        write_private(keys, "")?;
    }
    for (path, bad) in [(dir, 0o077), (keys, 0o077)] {
        let mode = std::fs::metadata(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?
            .permissions()
            .mode();
        if mode & bad != 0 {
            return Err(format!(
                "{} is open to other users (mode {:o}): chmod {} {}",
                path.display(),
                mode & 0o777,
                if path == dir { "700" } else { "600" },
                path.display()
            ));
        }
    }
    Ok(())
}

/// Replaces `path` with `text`, mode 600: written to a file beside it first
/// and renamed over it, so a crash never leaves half an authorized_keys.
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension(format!("ift-tmp-{}", std::process::id()));
    let io = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    std::fs::write(&tmp, text).map_err(io)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

/// A new ed25519 key: `ssh-keygen` writes it to a private temp folder, the
/// seed and public key are read out, and the folder is deleted.
fn make_key() -> Result<([u8; 32], [u8; 32]), String> {
    let dir = std::env::temp_dir().join(format!(
        "ift-phone-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir(&dir).map_err(|e| format!("cannot make a temp folder: {e}"))?;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    let file = dir.join("k");
    let made = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "", "-f"])
        .arg(&file)
        .status();
    let key = match made {
        Ok(s) if s.success() => std::fs::read_to_string(&file)
            .map_err(|e| format!("cannot read the new key: {e}"))
            .and_then(|t| phone::parse_openssh_ed25519(&t)),
        Ok(s) => Err(format!("ssh-keygen failed: {s}")),
        Err(e) => Err(format!("cannot run ssh-keygen: {e}")),
    };
    let _ = std::fs::remove_dir_all(&dir);
    key
}

fn run_text(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// The Tailscale MagicDNS name when there is one, else the hostname.
fn default_host() -> String {
    for cmd in ["tailscale", TAILSCALE_APP] {
        let name = run_text(cmd, &["status", "--json"])
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v["Self"]["DNSName"].as_str().map(str::to_string))
            .map(|n| n.trim_end_matches('.').to_string())
            .filter(|n| !n.is_empty());
        if let Some(n) = name {
            return n;
        }
    }
    run_text("hostname", &[]).unwrap_or_else(|| "localhost".into())
}

fn computer_name() -> String {
    run_text("scutil", &["--get", "ComputerName"])
        .or_else(|| run_text("hostname", &[]))
        .unwrap_or_else(|| "Mac".into())
}

fn user() -> String {
    std::env::var("USER")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| run_text("id", &["-un"]))
        .unwrap_or_default()
}

/// Black modules on a white ground whatever the terminal's theme is: a QR
/// code drawn light on dark is read by fewer scanners.
fn print_qr(url: &str) -> Result<(), String> {
    use qrcode::render::unicode::Dense1x2;
    let code = qrcode::QrCode::new(url.as_bytes()).map_err(|e| format!("cannot make the QR code: {e}"))?;
    let art = code.render::<Dense1x2>().quiet_zone(true).build();
    for line in art.lines() {
        println!("\x1b[30;47m{line}\x1b[0m");
    }
    Ok(())
}

// ------------------------------------------------------------ the gate

/// `ift phone-gate`: sshd's forced command for a phone key.
pub fn gate() -> ExitCode {
    let refuse = |why: &str| {
        eprintln!("ift: {why}");
        ExitCode::from(2)
    };
    let Some(original) = std::env::var_os("SSH_ORIGINAL_COMMAND") else {
        return refuse("a phone key opens no shell");
    };
    let Some(original) = original.to_str() else {
        return refuse("the command is not text");
    };
    let words = match phone::tokenize(original) {
        Ok(w) => w,
        Err(e) => return refuse(&e),
    };
    let Some((first, rest)) = words.split_first() else {
        return refuse("an empty command");
    };
    let own = match own_path() {
        Ok(p) => p,
        Err(e) => return refuse(&e),
    };
    // The phone calls `ift` by the path it was paired with; any other
    // program, or this one by another name, is refused.
    if Path::new(first).canonicalize().map(|p| p.to_string_lossy().into_owned()).ok().as_deref() != Some(own.as_str()) {
        return refuse("not the paired ift");
    }
    let args = match phone::gate_args(rest) {
        Ok(a) => a,
        Err(e) => return refuse(&e),
    };
    match args.as_slice() {
        [a, b] if a == "read" && b == "--stream" => read_stream(),
        [a, b] if a == "settings" && b == "show" => settings_show(),
        [a, b] if a == "snippets" && b == "--nul" => snippets_nul(),
        _ => match Command::new(&own).args(&args).status() {
            Ok(s) => ExitCode::from(s.code().unwrap_or(1) as u8),
            Err(e) => refuse(&format!("cannot run ift: {e}")),
        },
    }
}

// ------------------------------------------------- the three phone verbs

/// `ift read --stream`: card numbers, one per line on stdin; for each, the
/// framing the phone's old shell loop printed: 0x1e, the number, 0x1f, the
/// colour screen (or the plain one when the app is older), 0x1d. Ends at EOF.
pub fn read_stream() -> ExitCode {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    stream_cards(stdin.lock(), &mut out, card_text);
    ExitCode::SUCCESS
}

/// `read_stream` with its input, output and the app's answer passed in, so
/// the bytes are tested without an app. `fetch(card, ansi)` is the card's
/// screen, or `None` when the app refuses (an older one has no `--ansi`).
fn stream_cards(
    input: impl BufRead,
    out: &mut impl Write,
    fetch: impl Fn(&str, bool) -> Option<String>,
) {
    for line in input.lines() {
        let Ok(n) = line else { break };
        let n = n.trim().to_string();
        if n.is_empty() {
            continue;
        }
        let _ = write!(out, "\x1e{n}\x1f");
        // Same card refs `ift read` takes; anything else prints an empty frame.
        let text = if n.chars().all(|c| c.is_ascii_alphanumeric() || "#-_".contains(c)) {
            fetch(&n, true).or_else(|| fetch(&n, false))
        } else {
            None
        };
        if let Some(t) = text.filter(|t| !t.is_empty()) {
            let _ = writeln!(out, "{t}");
        }
        let _ = write!(out, "\x1d");
        let _ = out.flush();
    }
}

fn card_text(card: &str, ansi: bool) -> Option<String> {
    let mut args = vec![card.to_string()];
    if ansi {
        args.push("--ansi".into());
    }
    crate::socket::request("read", args)
        .ok()
        .filter(|r| r.ok)
        .map(|r| r.text)
}

/// `ift settings show`: settings.json as it is on disk, comments and trailing
/// commas included; nothing when there is none.
pub fn settings_show() -> ExitCode {
    let dir = infiniterm_core::paths::config_dir();
    let _ = std::io::stdout().write_all(&settings_bytes(&dir));
    ExitCode::SUCCESS
}

fn settings_bytes(config_dir: &Path) -> Vec<u8> {
    let mut bytes = vec![];
    if let Ok(mut f) = std::fs::File::open(config_dir.join("settings.json")) {
        if f.read_to_end(&mut bytes).is_err() {
            bytes.clear();
        }
    }
    bytes
}

/// `ift snippets --nul`: for each regular, non-hidden file directly in the
/// snippets folder (links followed): `./name`, NUL, the bytes, NUL.
pub fn snippets_nul() -> ExitCode {
    let dir = infiniterm_core::paths::config_dir();
    let _ = std::io::stdout().write_all(&snippets_bytes(&dir));
    ExitCode::SUCCESS
}

fn snippets_bytes(config_dir: &Path) -> Vec<u8> {
    let mut out = vec![];
    let Ok(entries) = std::fs::read_dir(config_dir.join("snippets")) else {
        return out;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // `is_file` follows links, as `find -L` did.
        if name.starts_with('.') || !e.path().is_file() {
            continue;
        }
        let Ok(bytes) = std::fs::read(e.path()) else {
            continue;
        };
        out.extend_from_slice(format!("./{name}\0").as_bytes());
        out.extend_from_slice(&bytes);
        out.push(0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ift-phone-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn read_stream_frames_each_card_like_the_phones_shell_loop() {
        let mut out = vec![];
        let fetch = |card: &str, ansi: bool| match (card, ansi) {
            ("7", true) => Some("\x1b[0mred".to_string()),
            // An older app has no --ansi: the plain screen answers.
            ("8", true) => None,
            ("8", false) => Some("plain".to_string()),
            ("9", _) => Some(String::new()),
            _ => None,
        };
        stream_cards("7\n8\n9\nnope\n\n  #3  \n".as_bytes(), &mut out, fetch);
        // Card 9 answers with an empty screen, "nope" with a refusal and #3
        // with nothing: each is an empty frame, never an error.
        assert_eq!(
            out,
            b"\x1e7\x1f\x1b[0mred\n\x1d\x1e8\x1fplain\n\x1d\x1e9\x1f\x1d\x1enope\x1f\x1d\x1e#3\x1f\x1d"
        );
    }

    #[test]
    fn read_stream_prints_an_empty_frame_for_an_odd_card_ref_and_never_asks_the_app() {
        let mut out = vec![];
        stream_cards("7;ls\n".as_bytes(), &mut out, |_, _| panic!("asked"));
        assert_eq!(out, b"\x1e7;ls\x1f\x1d");
    }

    #[test]
    fn settings_show_is_the_file_unchanged_and_nothing_when_missing() {
        let d = scratch("settings");
        assert!(settings_bytes(&d).is_empty());
        let text = "{\n  // a comment\n  \"ui.cardRadius\": 8,\n}\n";
        std::fs::write(d.join("settings.json"), text).unwrap();
        assert_eq!(settings_bytes(&d), text.as_bytes());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn snippets_list_regular_visible_files_with_links_followed() {
        let d = scratch("snippets");
        assert!(snippets_bytes(&d).is_empty(), "no folder: nothing");
        let s = d.join("snippets");
        std::fs::create_dir_all(s.join("sub")).unwrap();
        std::fs::write(s.join("one.json"), "{}").unwrap();
        std::fs::write(s.join(".hidden"), "x").unwrap();
        std::fs::write(d.join("target.txt"), "linked").unwrap();
        std::os::unix::fs::symlink(d.join("target.txt"), s.join("link.txt")).unwrap();
        let mut got: Vec<Vec<u8>> = snippets_bytes(&d)
            .split(|b| *b == 0)
            .map(<[u8]>::to_vec)
            .collect();
        // Trailing NUL leaves one empty piece; the rest are name, bytes pairs.
        assert_eq!(got.pop().unwrap(), b"");
        let mut pairs: Vec<(String, String)> = got
            .chunks(2)
            .map(|c| {
                (
                    String::from_utf8(c[0].clone()).unwrap(),
                    String::from_utf8(c[1].clone()).unwrap(),
                )
            })
            .collect();
        pairs.sort();
        assert_eq!(
            pairs,
            [
                ("./link.txt".to_string(), "linked".to_string()),
                ("./one.json".to_string(), "{}".to_string())
            ]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    // The seed parser against a key the real ssh-keygen wrote: the public key
    // it reads must be the one `ssh-keygen -y` derives from the file.
    #[test]
    fn a_real_ssh_keygen_key_parses_to_its_own_public_key() {
        let Ok((seed, public)) = make_key() else {
            eprintln!("no ssh-keygen here: skipped");
            return;
        };
        assert_ne!(seed, [0u8; 32]);
        assert_ne!(seed, public);
        let d = scratch("keygen");
        let file = d.join("k");
        let made = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "", "-f"])
            .arg(&file)
            .status()
            .unwrap();
        assert!(made.success());
        let (_, parsed_public) =
            phone::parse_openssh_ed25519(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let derived = run_text("ssh-keygen", &["-y", "-f", file.to_str().unwrap()]).unwrap();
        let blob = phone::base64_decode(derived.split_whitespace().nth(1).unwrap()).unwrap();
        assert_eq!(blob, phone::ed25519_blob(&parsed_public));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_qr_code_draws_black_on_white_lines() {
        // Not asserting the pixels: a code for a real URL is built without error.
        let url = phone::pair_url(&phone::Pairing {
            name: "Mac",
            host: "mini.example.ts.net",
            port: 22,
            user: "me",
            host_key_blob: &phone::ed25519_blob(&[3; 32]),
            seed: &[9; 32],
            device: "iphone",
            ift: "/Applications/infiniterm.app/Contents/MacOS/ift",
            ift_version: "0.6.13",
        });
        assert!(url.len() < 400, "{}", url.len());
        assert!(print_qr(&url).is_ok());
    }

    #[test]
    fn writing_the_keys_file_replaces_it_whole_with_mode_600() {
        let d = scratch("write");
        let f = d.join("authorized_keys");
        write_private(&f, "one\n").unwrap();
        write_private(&f, "two\n").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "two\n");
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no temp file left");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_loose_ssh_folder_is_said_not_changed() {
        let d = scratch("loose");
        let ssh = d.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = check_ssh_dir(&ssh.join("authorized_keys")).unwrap_err();
        assert!(err.contains("chmod 700"), "{err}");
        assert_eq!(std::fs::metadata(&ssh).unwrap().permissions().mode() & 0o777, 0o755);
        let _ = std::fs::remove_dir_all(&d);
    }
}
