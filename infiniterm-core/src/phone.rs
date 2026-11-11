//! The pure half of `ift phone` (#phone pairing): everything that is text in,
//! text out, so it is tested without ssh, a disk or a phone. The CLI half is
//! `infiniterm-cli/src/phone_cmd.rs`, which generates the key, writes
//! `~/.ssh/authorized_keys` and prints the QR code.
//!
//! What it holds:
//! - the `authorized_keys` line a paired phone gets, restricted to the forced
//!   command `ift phone-gate`, and the add / list / remove edits of that file
//!   (one line per phone, tagged by its comment `infiniterm-phone:<name>`);
//! - the pairing URL the QR code carries (`infiniterm://pair?...`);
//! - reading the 32-byte seed out of an OpenSSH private key file, so the key
//!   can be made by `ssh-keygen` and no crypto crate is needed;
//! - the gate: `$SSH_ORIGINAL_COMMAND` split into words the way the phone's
//!   `shellQuote` writes them, and the one table of what a phone may run.
//!
//! Constraints: nothing here runs a shell, and a word that is not plain or
//! single-quoted is refused, so no `$`, backtick, backslash or double quote
//! ever reaches a command. A phone's identity is its key only; the comment is
//! how `remove` finds the line, so a device name is [A-Za-z0-9_-] and nothing
//! else (a newline in it would add a second authorized_keys line).
//! Related: `push_notify.rs` (the push-devices file `push add` writes).

/// The comment tag of a paired phone's line.
pub const TAG_PREFIX: &str = "infiniterm-phone:";

/// A device name: 1 to 32 of letters, digits, dash and underscore.
pub fn valid_device_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

// ---------------------------------------------------------------- base64

const STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding (an ssh public key's second field).
pub fn base64_std(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(STD[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// URL-safe base64 without padding (the seed in the QR code).
pub fn base64url_nopad(bytes: &[u8]) -> String {
    base64_std(bytes)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

/// Base64, standard or URL-safe, padded or not. `None` on any other byte.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in text.trim().trim_end_matches('=').chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' | '-' => 62,
            '/' | '_' => 63,
            _ => return None,
        };
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits & 0xff) as u8);
        }
    }
    Some(out)
}

// ------------------------------------------------------ keys and the file

/// The wire form of an ed25519 public key: `string "ssh-ed25519"` then
/// `string <32 bytes>`. Its base64 is the middle field of a `.pub` line.
pub fn ed25519_blob(public: &[u8; 32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(51);
    b.extend_from_slice(&11u32.to_be_bytes());
    b.extend_from_slice(b"ssh-ed25519");
    b.extend_from_slice(&32u32.to_be_bytes());
    b.extend_from_slice(public);
    b
}

/// A big-endian length-prefixed field at `*at`; moves `at` past it.
fn field<'a>(data: &'a [u8], at: &mut usize) -> Result<&'a [u8], String> {
    let end = at
        .checked_add(4)
        .filter(|e| *e <= data.len())
        .ok_or("the key file is cut short")?;
    let len = u32::from_be_bytes(data[*at..end].try_into().unwrap()) as usize;
    let stop = end
        .checked_add(len)
        .filter(|e| *e <= data.len())
        .ok_or("the key file is cut short")?;
    *at = stop;
    Ok(&data[end..stop])
}

/// The (seed, public key) of an unencrypted OpenSSH ed25519 private key file
/// (`ssh-keygen -t ed25519 -N ""`).
pub fn parse_openssh_ed25519(text: &str) -> Result<([u8; 32], [u8; 32]), String> {
    let body: String = text
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    let data = base64_decode(&body).ok_or("the key file is not base64")?;
    let magic = b"openssh-key-v1\0";
    if !data.starts_with(magic) {
        return Err("not an OpenSSH private key".into());
    }
    let mut at = magic.len();
    let cipher = field(&data, &mut at)?;
    let kdf = field(&data, &mut at)?;
    let _kdf_options = field(&data, &mut at)?;
    if cipher != b"none" || kdf != b"none" {
        return Err("the key is encrypted".into());
    }
    let count = data
        .get(at..at + 4)
        .ok_or("the key file is cut short")?
        .try_into()
        .map(u32::from_be_bytes)
        .unwrap();
    at += 4;
    if count != 1 {
        return Err("the key file holds more than one key".into());
    }
    let _public = field(&data, &mut at)?;
    let private = field(&data, &mut at)?;
    let mut p = 8; // two check integers
    if private.len() < p {
        return Err("the key file is cut short".into());
    }
    if field(private, &mut p)? != b"ssh-ed25519" {
        return Err("not an ed25519 key".into());
    }
    let public: [u8; 32] = field(private, &mut p)?
        .try_into()
        .map_err(|_| "bad public key")?;
    let secret = field(private, &mut p)?;
    if secret.len() != 64 || secret[32..] != public {
        return Err("bad private key".into());
    }
    let seed: [u8; 32] = secret[..32].try_into().unwrap();
    Ok((seed, public))
}

/// The options every phone key carries. `restrict` takes away the pty, the
/// agent and forwarding; `port-forwarding` plus the three `permitopen`
/// entries give back the one thing the phone's browser card needs, a tunnel
/// to this Mac's own localhost (the app opens all three spellings).
const OPTIONS: &str = "restrict,port-forwarding,permitopen=\"127.0.0.1:*\",permitopen=\"localhost:*\",permitopen=\"[::1]:*\"";

/// One authorized_keys line for a phone: restricted to `ift phone-gate`.
/// `ift` is the absolute path of this binary; a `"` or a backslash in it
/// would end the `command="..."` option early, so it is refused.
pub fn authorized_line(ift: &str, public: &[u8; 32], name: &str) -> Result<String, String> {
    if !valid_device_name(name) {
        return Err("a device name is 1 to 32 letters, digits, - or _".into());
    }
    if ift.contains(['"', '\\', '\n', '\r']) || !ift.starts_with('/') {
        return Err(format!("cannot pin this ift path: {ift}"));
    }
    Ok(format!(
        "{OPTIONS},command=\"{ift} phone-gate\" ssh-ed25519 {} {TAG_PREFIX}{name}",
        base64_std(&ed25519_blob(public))
    ))
}

/// The name a line is tagged with, if it is one of ours.
fn tagged(line: &str) -> Option<&str> {
    let last = line.trim_end().rsplit(' ').next()?;
    last.strip_prefix(TAG_PREFIX)
}

/// `existing` with `line` appended (a missing final newline is added).
/// Refuses a name already paired: remove it first, or the old key would stay
/// valid beside the new one.
pub fn add_line(existing: &str, line: &str, name: &str) -> Result<String, String> {
    if existing.lines().any(|l| tagged(l) == Some(name)) {
        return Err(format!(
            "a phone called {name} is already paired: ift phone remove {name}"
        ));
    }
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    Ok(out)
}

/// `existing` without the line tagged `name`, and whether there was one. Every
/// other line is kept byte for byte.
pub fn remove_line(existing: &str, name: &str) -> (String, bool) {
    let mut found = false;
    let kept: String = existing
        .split_inclusive('\n')
        .filter(|l| {
            let hit = tagged(l.trim_end_matches('\n')) == Some(name);
            found |= hit;
            !hit
        })
        .collect();
    (kept, found)
}

/// The names of the paired phones, in file order.
pub fn paired_names(existing: &str) -> Vec<String> {
    existing
        .lines()
        .filter_map(tagged)
        .map(str::to_string)
        .collect()
}

// ------------------------------------------------------------ the QR code

/// Everything the phone needs, as the QR code's URL (`v=1`).
pub struct Pairing<'a> {
    /// The Mac's ComputerName, for labels.
    pub name: &'a str,
    pub host: &'a str,
    pub port: u16,
    pub user: &'a str,
    /// The sshd ed25519 host key's blob, so the phone does not trust on first use.
    pub host_key_blob: &'a [u8],
    /// The phone key's 32-byte seed.
    pub seed: &'a [u8; 32],
    pub device: &'a str,
    /// The absolute path of `ift` on this Mac.
    pub ift: &'a str,
    pub ift_version: &'a str,
}

/// Percent-encodes everything but unreserved characters.
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn pair_url(p: &Pairing) -> String {
    format!(
        "infiniterm://pair?v=1&n={}&h={}&p={}&u={}&hk={}&k={}&d={}&i={}&iv={}",
        encode(p.name),
        encode(p.host),
        p.port,
        encode(p.user),
        base64url_nopad(p.host_key_blob),
        base64url_nopad(p.seed),
        encode(p.device),
        encode(p.ift),
        encode(p.ift_version),
    )
}

/// The middle field of a `ssh_host_ed25519_key.pub` line, decoded.
pub fn host_key_blob(pub_file: &str) -> Option<Vec<u8>> {
    let mut f = pub_file.split_whitespace();
    (f.next()? == "ssh-ed25519")
        .then(|| f.next())
        .flatten()
        .and_then(base64_decode)
}

// ---------------------------------------------------------------- the gate

/// A word `shellQuote` leaves bare.
fn bare(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_-./@%+=:,~".contains(c)
}

/// Splits `$SSH_ORIGINAL_COMMAND` into words: plain words of safe characters,
/// and single-quoted words with `'\''` for a quote inside. Anything else
/// (double quotes, backslashes, `$`, `;`, a bare `*`) is refused, so the
/// result is what a shell would have produced and nothing ever needs one.
pub fn tokenize(command: &str) -> Result<Vec<String>, String> {
    let mut words = vec![];
    let mut cur = String::new();
    let mut open = false; // a word is being built
    let mut quoted = false;
    for c in command.chars() {
        if quoted {
            if c == '\'' {
                quoted = false;
            } else {
                cur.push(c);
            }
        } else if c == '\'' {
            quoted = true;
            open = true;
        } else if c == '\\' {
            // Only the phone's `'\''`: a backslash straight after a closing
            // quote and straight before an opening one is a literal quote.
            return Err("a backslash is not allowed".into());
        } else if c == ' ' {
            if open {
                words.push(std::mem::take(&mut cur));
                open = false;
            }
        } else if bare(c) {
            cur.push(c);
            open = true;
        } else {
            return Err(format!("{c:?} is not allowed"));
        }
    }
    if quoted {
        return Err("a quote is not closed".into());
    }
    if open {
        words.push(cur);
    }
    Ok(words)
}

/// A card as `ift` takes it: a number (`7`, `#7`) or an id, made of safe
/// characters only.
fn valid_card(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '#' || c == '-' || c == '_')
}

/// What `ift phone-gate` runs: `ift` with these arguments, or why not.
/// `words` are the tokens after the ift path (the path itself is checked by
/// the caller against the one this key was paired for).
pub fn gate_args(words: &[String]) -> Result<Vec<String>, String> {
    let w: Vec<&str> = words.iter().map(String::as_str).collect();
    let ok = match w.as_slice() {
        ["proxy", "join", "canvas"] => true,
        ["proxy", "join", "session", id] => valid_card(id),
        // The file service, read-only: it refuses write and put_drop. The
        // bare `proxy files` would let the phone write to the Mac.
        ["proxy", "files", "--read-only"] => true,
        ["--version"] | ["update", "status"] | ["workspaces"] => true,
        ["run", "app.update.check"] | ["run", "app.update.install"] => true,
        ["read", "--stream"] => true,
        ["settings", "show"] | ["snippets", "--nul"] => true,
        ["push", "remove", id] => crate::push_notify::valid_device_id(id),
        ["push", "add", id, "--server", url] => {
            crate::push_notify::valid_device_id(id) && url.starts_with("https://")
        }
        ["read", card, rest @ ..] => valid_card(card) && read_flags(rest),
        _ => false,
    };
    if ok {
        Ok(words.to_vec())
    } else {
        Err("this command is not allowed for a phone".into())
    }
}

/// `--all`, `--ansi` and `--lines N`, in any order, nothing else.
fn read_flags(rest: &[&str]) -> bool {
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match *a {
            "--all" | "--ansi" => {}
            "--lines" => {
                if !it.next().is_some_and(|n| n.parse::<u32>().is_ok()) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

// ------------------------------------------------------------- Tailscale SSH

/// The first tailnet port tried for the forward to this Mac's sshd.
pub const FORWARD_FIRST_PORT: u16 = 2222;

/// Tailscale SSH takes the tailnet address on port 22 and ignores
/// `authorized_keys`, so a paired phone never reaches this Mac's sshd there.
/// `tailscale debug prefs` says whether it is on.
pub fn tailscale_ssh_on(prefs_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(prefs_json)
        .ok()
        .and_then(|v| v["RunSSH"].as_bool())
        == Some(true)
}

/// Which tailnet port reaches this Mac's sshd.
#[derive(Debug, PartialEq, Eq)]
pub enum Forward {
    /// A `tailscale serve` TCP forward to localhost:22 already exists.
    Existing(u16),
    /// None does; this free port needs one.
    New(u16),
}

/// From `tailscale serve status --json`: the port of a TCP forward to this
/// Mac's own sshd if there is one, else the first free port from
/// `FORWARD_FIRST_PORT`. A port the serve config already uses (https on 443,
/// someone else's forward) is never taken.
pub fn plan_forward(status_json: &str) -> Forward {
    let v: serde_json::Value = serde_json::from_str(status_json).unwrap_or_default();
    let tcp = v["TCP"].as_object().cloned().unwrap_or_default();
    for (port, entry) in &tcp {
        let to_sshd = matches!(
            entry["TCPForward"].as_str(),
            Some("localhost:22" | "127.0.0.1:22" | "[::1]:22")
        );
        if let (true, Ok(p)) = (to_sshd, port.parse::<u16>()) {
            return Forward::Existing(p);
        }
    }
    let used: Vec<u16> = tcp.keys().filter_map(|k| k.parse().ok()).collect();
    let free = (FORWARD_FIRST_PORT..=u16::MAX)
        .find(|p| !used.contains(p))
        .unwrap_or(FORWARD_FIRST_PORT);
    Forward::New(free)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(a: &[&str]) -> Vec<String> {
        a.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn device_names_are_plain() {
        for ok in ["iphone", "Ekin-12", "a_b", &"x".repeat(32)] {
            assert!(valid_device_name(ok), "{ok}");
        }
        for bad in ["", "a b", "a\nb", "a:b", "é", &"x".repeat(33)] {
            assert!(!valid_device_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn base64_matches_the_known_vectors_and_decodes_both_alphabets() {
        assert_eq!(base64_std(b""), "");
        assert_eq!(base64_std(b"f"), "Zg==");
        assert_eq!(base64_std(b"fo"), "Zm8=");
        assert_eq!(base64_std(b"foo"), "Zm9v");
        assert_eq!(base64url_nopad(&[0xfb, 0xff, 0xfe]), "-__-");
        assert_eq!(base64_decode("Zm9vYg==").unwrap(), b"foob");
        assert_eq!(base64_decode("Zm9vYg").unwrap(), b"foob");
        assert_eq!(base64_decode("-__-").unwrap(), [0xfb, 0xff, 0xfe]);
        assert_eq!(base64_decode("+//+").unwrap(), [0xfb, 0xff, 0xfe]);
        assert!(base64_decode("a b").is_none());
    }

    /// An OpenSSH ed25519 private key file for a known seed, built by hand.
    fn openssh_file(seed: [u8; 32], public: [u8; 32]) -> String {
        let mut private = vec![1, 2, 3, 4, 1, 2, 3, 4]; // check integers
        let put = |v: &mut Vec<u8>, b: &[u8]| {
            v.extend_from_slice(&(b.len() as u32).to_be_bytes());
            v.extend_from_slice(b);
        };
        put(&mut private, b"ssh-ed25519");
        put(&mut private, &public);
        let mut both = seed.to_vec();
        both.extend_from_slice(&public);
        put(&mut private, &both);
        put(&mut private, b"comment");
        while private.len() % 8 != 0 {
            private.push(private.len() as u8 % 8 + 1);
        }
        let mut d = b"openssh-key-v1\0".to_vec();
        put(&mut d, b"none");
        put(&mut d, b"none");
        put(&mut d, b"");
        d.extend_from_slice(&1u32.to_be_bytes());
        put(&mut d, &ed25519_blob(&public));
        put(&mut d, &private);
        format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
            base64_std(&d)
        )
    }

    #[test]
    fn the_seed_comes_out_of_an_openssh_key_file() {
        let (seed, public) = ([7u8; 32], [9u8; 32]);
        let got = parse_openssh_ed25519(&openssh_file(seed, public)).unwrap();
        assert_eq!(got, (seed, public));
        assert!(parse_openssh_ed25519("hello").is_err());
        let cut = openssh_file(seed, public).replace("AAAA", "AA");
        assert!(parse_openssh_ed25519(&cut).is_err());
    }

    #[test]
    fn an_encrypted_key_is_refused() {
        let text = openssh_file([1; 32], [2; 32]);
        let data = base64_decode(
            &text
                .lines()
                .filter(|l| !l.starts_with("-----"))
                .collect::<String>(),
        )
        .unwrap();
        let mut d = data.clone();
        // "none" cipher name becomes "aes1": the key claims to be encrypted.
        let at = b"openssh-key-v1\0".len() + 4;
        d[at..at + 4].copy_from_slice(b"aes1");
        let enc = format!("-----BEGIN-----\n{}\n-----END-----\n", base64_std(&d));
        assert_eq!(
            parse_openssh_ed25519(&enc).unwrap_err(),
            "the key is encrypted"
        );
    }

    #[test]
    fn the_line_is_restricted_and_tagged() {
        let line = authorized_line(
            "/Applications/infiniterm.app/Contents/MacOS/ift",
            &[5; 32],
            "iphone",
        )
        .unwrap();
        assert!(line.starts_with("restrict,port-forwarding,permitopen=\"127.0.0.1:*\","));
        assert!(line.contains("permitopen=\"localhost:*\""));
        assert!(line.contains("permitopen=\"[::1]:*\""));
        assert!(
            line.contains("command=\"/Applications/infiniterm.app/Contents/MacOS/ift phone-gate\"")
        );
        assert!(line.contains(" ssh-ed25519 AAAAC3NzaC1lZDI1NTE5"));
        assert!(line.ends_with(" infiniterm-phone:iphone"));
        assert!(authorized_line("/a\"b/ift", &[5; 32], "x").is_err());
        assert!(
            authorized_line("ift", &[5; 32], "x").is_err(),
            "absolute only"
        );
        assert!(authorized_line("/a/ift", &[5; 32], "x y").is_err());
    }

    #[test]
    fn add_list_and_remove_touch_only_the_tagged_line() {
        let other = "ssh-rsa AAAA me@laptop\n# a comment\n";
        let a = authorized_line("/i/ift", &[1; 32], "one").unwrap();
        let b = authorized_line("/i/ift", &[2; 32], "two").unwrap();
        let file = add_line(other, &a, "one").unwrap();
        let file = add_line(&file, &b, "two").unwrap();
        assert_eq!(paired_names(&file), ["one", "two"]);
        assert!(
            add_line(&file, &a, "one").is_err(),
            "no second key for a name"
        );
        let (without, found) = remove_line(&file, "one");
        assert!(found);
        assert_eq!(paired_names(&without), ["two"]);
        assert!(
            without.starts_with(other),
            "the other lines stay as they were"
        );
        let (same, found) = remove_line(&without, "one");
        assert!(!found);
        assert_eq!(same, without);
        // No final newline in the file: the new line still starts on its own line.
        let file = add_line("ssh-rsa AAAA me", &a, "one").unwrap();
        assert_eq!(file.lines().count(), 2);
        // A name that is a prefix of another does not match it.
        let (kept, found) = remove_line(&file, "on");
        assert!(!found && kept == file);
    }

    #[test]
    fn the_pair_url_carries_every_field() {
        let url = pair_url(&Pairing {
            name: "Ekin's Mac mini",
            host: "mini.tail1234.ts.net",
            port: 22,
            user: "ekinertac",
            host_key_blob: &ed25519_blob(&[3; 32]),
            seed: &[0xfb; 32],
            device: "iphone",
            ift: "/Applications/infiniterm.app/Contents/MacOS/ift",
            ift_version: "0.6.12",
        });
        assert!(url.starts_with("infiniterm://pair?v=1&n=Ekin%27s%20Mac%20mini&h=mini.tail1234.ts.net&p=22&u=ekinertac&hk="));
        assert!(url.contains("&k=-_v7-_v7")); // 0xfb bytes, url-safe, no padding
        assert!(url.contains(
            "&d=iphone&i=%2FApplications%2Finfiniterm.app%2FContents%2FMacOS%2Fift&iv=0.6.12"
        ));
        assert!(!url.contains('=') || !url.ends_with('='));
    }

    #[test]
    fn a_host_key_file_gives_its_blob() {
        let blob = ed25519_blob(&[4; 32]);
        let file = format!("ssh-ed25519 {} root@mini\n", base64_std(&blob));
        assert_eq!(host_key_blob(&file).unwrap(), blob);
        assert!(host_key_blob("ssh-rsa AAAA x").is_none());
    }

    #[test]
    fn the_gate_reads_plain_and_single_quoted_words() {
        assert_eq!(
            tokenize("/opt/ift read --stream").unwrap(),
            s(&["/opt/ift", "read", "--stream"])
        );
        assert_eq!(
            tokenize("'/opt/my tools/ift' read --stream").unwrap(),
            s(&["/opt/my tools/ift", "read", "--stream"])
        );
        // The phone writes a quote inside a word as '\'' ; that is a backslash,
        // which this gate refuses: such a path is not paired.
        assert!(tokenize("'/a/it'\\''s/ift' read 1").is_err());
        assert_eq!(tokenize("  a   b ").unwrap(), s(&["a", "b"]));
        assert_eq!(tokenize("''").unwrap(), s(&[""]));
        for bad in [
            "a;b", "a$b", "a`b`", "a\"b\"", "a|b", "a&b", "a>b", "a\nb", "a*", "'open",
        ] {
            assert!(tokenize(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_gate_allows_the_phones_commands_and_nothing_else() {
        let ok = |a: &[&str]| gate_args(&s(a)).is_ok();
        assert!(ok(&["proxy", "join", "canvas"]));
        assert!(ok(&["proxy", "join", "session", "abc-123"]));
        assert!(ok(&["proxy", "files", "--read-only"]));
        assert!(!ok(&["proxy", "files"]), "writes would be allowed");
        assert!(!ok(&["proxy", "files", "--read-only", "x"]));
        assert!(ok(&["read", "7", "--all"]));
        assert!(ok(&["read", "#7", "--ansi"]));
        assert!(ok(&["read", "7", "--lines", "20"]));
        assert!(ok(&["read", "--stream"]));
        assert!(ok(&["settings", "show"]));
        assert!(ok(&["snippets", "--nul"]));
        assert!(ok(&["--version"]));
        assert!(ok(&["update", "status"]));
        assert!(ok(&["run", "app.update.check"]));
        assert!(ok(&["run", "app.update.install"]));
        assert!(ok(&["workspaces"]));
        assert!(ok(&["push", "remove", "abcDEF_-123"]));
        assert!(!ok(&["run", "app.quit"]), "only the two update ids");
        assert!(!ok(&["run"]));
        assert!(!ok(&["send", "7", "rm -rf ~"]));
        assert!(!ok(&["close", "7"]));
        assert!(!ok(&["terminal", "/"]));
        assert!(!ok(&["read", "7", "--evil"]));
        assert!(!ok(&["read", "7", "--lines", "x"]));
        assert!(!ok(&["read", "7;ls"]));
        assert!(!ok(&["proxy", "join", "session"]));
        assert!(!ok(&["push", "add", "short", "--server", "http://x"]));
        assert!(
            !ok(&["phone", "add", "x"]),
            "a phone cannot pair another phone"
        );
        assert!(!ok(&[]));
    }

    #[test]
    fn tailscale_ssh_is_read_from_the_prefs() {
        assert!(tailscale_ssh_on(r#"{"RunSSH": true, "Hostname": ""}"#));
        assert!(!tailscale_ssh_on(r#"{"RunSSH": false}"#));
        assert!(!tailscale_ssh_on(r#"{"Hostname": ""}"#));
        assert!(!tailscale_ssh_on("not json"));
        assert!(!tailscale_ssh_on(""));
    }

    #[test]
    fn the_forward_reuses_a_tcp_forward_to_sshd_and_never_takes_a_used_port() {
        // No serve config at all: the first port.
        assert_eq!(plan_forward("{}"), Forward::New(2222));
        assert_eq!(plan_forward(""), Forward::New(2222));
        // https on 443 only (this Mac's own state before pairing).
        let https = r#"{"TCP": {"443": {"HTTPS": true}}, "Web": {}}"#;
        assert_eq!(plan_forward(https), Forward::New(2222));
        // A forward to sshd already exists: reused, on whatever port.
        let ours = r#"{"TCP": {"2222": {"TCPForward": "localhost:22"}, "443": {"HTTPS": true}}}"#;
        assert_eq!(plan_forward(ours), Forward::Existing(2222));
        let other = r#"{"TCP": {"2300": {"TCPForward": "127.0.0.1:22"}}}"#;
        assert_eq!(plan_forward(other), Forward::Existing(2300));
        // 2222 is taken by something that is not sshd: the next one.
        let taken =
            r#"{"TCP": {"2222": {"TCPForward": "localhost:8080"}, "2223": {"HTTPS": true}}}"#;
        assert_eq!(plan_forward(taken), Forward::New(2224));
    }
}
