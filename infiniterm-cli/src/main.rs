//! `ift` — the terminal as a way into the canvas.
//!
//! Every card has a terminal in it, and a terminal is the best command interface
//! anyone has built. Typing a path should be enough to open it; that is what this
//! is for. It is NOT agent integration — hooks already solved reporting, one-way,
//! which is the right shape for state.
//!
//! Every verb here that performs an action is also a command in the app, so it
//! shows up in the palette. The reverse is deliberately not true: zoom, fit and
//! maximise stay palette-only, because nobody zooms a canvas by typing into a
//! terminal, and a CLI that accepted them would double the surface that has to
//! keep working in exchange for nothing.
//!
//! That is what keeps the verb set FIXED rather than a fuzzy-matched view of the
//! command registry: no ranking here, no ambiguity to report, and no matcher that
//! could drift from the palette's.
//!
//! Exit codes are API once anything scripts against them: 0 success, 1 infiniterm
//! is not running, 2 bad usage; `ift licence` adds 3 (rejected) and 4 (Lemon
//! Squeezy unreachable) for itself only.

mod attach;
mod claude_hooks;
mod cursor_hooks;
mod completion;
mod connect;
mod install;
mod licence_cmd;
mod proxy;
mod socket;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
ift — drive infiniterm from a shell

  ift                        launch infiniterm, or focus it if it is running
  ift <path>                 open a directory as a terminal card, a file as an
                             editor card; file:42 or file:42:7 opens at a line.
                             Run inside a card, a file opens IN PLACE over it
                             and ift waits until you close it, like vim, so
                             EDITOR=ift works for git commit. A file that
                             does not exist opens empty and the first save
                             creates it (any word that is not a command
                             below is taken as one); its directory must
                             exist
  ift -n <path>              a file in a card of its own, returning at once
  ift diff [path]            changes against git HEAD, as a card: a tree of the
                             changed files under path (the current directory
                             without one) and each file's diff
  ift ls                     cards: id, group, directory, state, remote, number
                             (the #7 on the card's label); a table on a
                             terminal, tab separated without a header into a pipe
  ift ls --agents            the cards with an agent: number, agent, session
                             id, state, directory, tab separated; the session
                             id is what `claude --resume` takes
  ift sessions [--full]      session daemons still running, app or no app:
                             card number and label (or directory) on a
                             terminal, every column with --full (id, pid,
                             cwd, command, started, card, label); into a
                             pipe always every column, tab separated, no
                             header
  ift attach <id|#N>         connect a session's shell to this terminal, by
                             its id or by the card's number (#7 on its
                             label, the same after a reboot; the id is not);
                             Ctrl-\\ (0x1c) detaches, leaving it running
  ift --version              the version of this ift
  ift connect <[user@]host> [--name N] [--color RRGGBB] [--check] [--install]
                             open a second infiniterm for a host: its own
                             window, Dock icon and canvas, with terminal cards
                             that run on that host over ssh (key login; ift
                             and iftd installed there). Connecting again
                             brings the same cards back. --check only tests
                             the host. --install first puts ift and iftd on
                             a Linux host (this version's package, sent over
                             ssh; --from <file> uses a file, --platform
                             linux-x86_64|linux-aarch64 skips asking the
                             host). Settings start with the defaults.
  ift terminal <dir>...      a terminal card whose shell starts in each folder,
                             on the workspace on screen, the last one
                             focused; starts infiniterm first when it is not
                             running (what Finder's Services > Open in
                             infiniterm does)
  ift send <card> [text] [--enter] [--key NAME]...
                             type into a card's shell, without moving the
                             focus; the card is its number (#7) or its id;
                             keys: enter esc tab backspace ctrl-c ctrl-d
                             ctrl-l ctrl-z up down left right
  ift read <card> [--lines N] [--all]
                             print what a card's terminal shows (the last N
                             lines of it; --all reaches into the history)
  ift close <card>           close a card like Cmd+W; a locked card refuses
  ift run <command-id>       run a registered command as the palette would,
                             on the card in focus (ift commands lists them)
  ift usage [days]           the commands and mouse gestures you used in the
                             last 30 days (or [days]), then the ones you
                             never did; from a log kept only on this Mac
  ift commands               every command the app registers: id, label,
                             key; the palette's list as text (the default
                             keybindings file has only the bound ones)
  ift omni <term>            what the omnibox (Cmd+L) would show for a term,
                             ranked against the real history and open cards
  ift name <text>            name the card this is run from
  ift group <name>           put this card in a group, creating it if needed
  ift licence [<email> <key>]
                             register this Mac with its commercial licence
                             key (optional; asks Lemon Squeezy once, then
                             the About window says who it is licensed to);
                             bare, says whether it is registered
  ift install                put ift on $PATH (a symlink in ~/.local/bin) and
                             its zsh completion on fpath
  ift completion zsh         print the zsh completion function
  ift install-claude-hooks   wire infiniterm into ~/.claude/settings.json
  ift install-codex-hooks    wire infiniterm into ~/.codex/hooks.json (or
                             $CODEX_HOME); approve them once with /hooks
  ift install-opencode-hooks install the OpenCode plugin into
                             ~/.config/opencode/plugins (or $XDG_CONFIG_HOME)
  ift install-pi-hooks [DIR] install the Pi extension into ~/.pi/agent (or
                             $PI_CODING_AGENT_DIR, or DIR: a wrapper that
                             runs Pi against its own agent dir needs its own)
  ift install-cursor-hooks   wire infiniterm into ~/.cursor/hooks.json
                             (Cursor Agent CLI and IDE agent)
  ift install-extension <path|id|store-url>
                             add an extension the browser cards load: an
                             unpacked directory, or an id / Chrome Web
                             Store url looked up in Chrome's own profile
                             (CEF cannot pull a .crx from the store
                             directly, so it must be installed there
                             first); takes effect on the app's next launch

Exit codes: 0 ok, 1 infiniterm not running, 2 bad usage; for ift licence
only, 3 key or email rejected, 4 Lemon Squeezy unreachable.
";

/// Every subcommand `main` dispatches, for the completion's test: the two
/// lists must agree, and this one is the source.
pub const SUBCOMMANDS: [&str; 24] = [
    "diff",
    "ls",
    "sessions",
    "attach",
    "connect",
    "terminal",
    "send",
    "read",
    "close",
    "run",
    "omni",
    "commands",
    "usage",
    "name",
    "group",
    "install",
    "licence",
    "install-claude-hooks",
    "install-codex-hooks",
    "install-opencode-hooks",
    "install-pi-hooks",
    "install-cursor-hooks",
    "install-extension",
    "completion",
];

/// The bundle id, which is how LaunchServices finds the app wherever it was
/// put — no path to guess, and a moved .app still launches.
const BUNDLE_ID: &str = "dev.ekinertac.infiniterm";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = || args[1..].to_vec();

    match args.first().map(String::as_str) {
        Some("install-claude-hooks") => install_hooks(args.contains(&"--dry-run".to_string())),
        Some("install-codex-hooks") => install_codex(args.contains(&"--dry-run".to_string())),
        Some("install-opencode-hooks") => {
            install_opencode(args.contains(&"--dry-run".to_string()))
        }
        Some("install-pi-hooks") => install_pi(
            args.iter().skip(1).find(|a| !a.starts_with("--")).map(String::as_str),
            args.contains(&"--dry-run".to_string()),
        ),
        Some("install-cursor-hooks") => {
            install_cursor(args.contains(&"--dry-run".to_string()))
        }
        Some("install-extension") => install_extension(args.get(1).map(String::as_str)),
        Some("install") => install_self(),
        Some("licence") => licence_cmd::run(&args[1..]),
        Some("connect") => connect::run(&args[1..]),
        // Not `ift version`: any word that is not a verb is a file to open.
        Some("--version") | Some("-V") => {
            println!("ift {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        // Plumbing for the remote instances, called over ssh by the app, so it
        // is in neither USAGE nor the completion (like dev-run).
        Some("proxy") => proxy::run(&args[1..]),
        Some("completion") => match args.get(1).map(String::as_str) {
            Some("zsh") => {
                print!("{}", completion::ZSH);
                ExitCode::SUCCESS
            }
            _ => {
                eprintln!("ift: completion takes one shell: zsh");
                ExitCode::from(2)
            }
        },
        Some("ls") if args.iter().any(|a| a == "--agents") => send("ls", rest()),
        Some("ls") => send_table("ls", &LS_HEADER),
        Some("terminal") => terminal_cmd(&args[1..]),
        Some("send") => send("send", rest()),
        Some("read") => send("read", rest()),
        Some("close") => send("close", rest()),
        Some("run") => send("run", rest()),
        Some("commands") => send_table("commands", &["id", "label", "key"]),
        Some("usage") => send("usage", rest()),
        Some("sessions") => attach::sessions_cmd(args.contains(&"--full".to_string())),
        Some("attach") => match args.get(1) {
            Some(id) => attach::attach(id),
            None => attach::no_id(),
        },
        Some("omni") => send("omni", rest()),
        Some("diff") => diff_path(args.get(1).map(String::as_str).unwrap_or(".")),
        Some("name") => send("name", rest()),
        Some("group") => send("group", rest()),
        // Not in USAGE on purpose. Runs a palette command by id, and the app
        // only answers it in a development build: it exists so the stress
        // harness can be driven from a script instead of by typing into the
        // palette, and UI actions are otherwise deliberately NOT ift's.
        Some("dev-run") => send("dev-run", rest()),
        Some("-n") => match new_card_path(&args) {
            Some(p) => open_path(p, false),
            None => {
                eprintln!("ift: -n takes a path");
                ExitCode::from(2)
            }
        },
        Some("-h") | Some("--help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        None => launch(),
        // A PATH beats a verb. A directory called `ls` in front of you is what you
        // meant; the verbs are checked first only so the common ones stay short.
        // Anything else is a path, existing or not: a word that is not a
        // command above is a new file, vim's way (Ekin, 2026-09-26: a rule
        // like "only if it has a dot" would be one more hidden thing). A
        // typo'd verb opens an empty editor; nothing is written unless saved.
        Some(arg) => open_path(arg, true),
    }
}

/// Bare `ift`: the app, the way typing `code` opens the editor.
///
/// Through `open -b`, so it works from anywhere the bundle has been registered
/// (dropped into /Applications, or simply launched once). A running instance
/// is focused rather than duplicated: the app is single-instance, and `open`
/// on a running bundle activates it.
fn launch() -> ExitCode {
    let status = std::process::Command::new("open")
        .args(["-b", BUNDLE_ID])
        .status();
    match status {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        _ => {
            eprintln!("ift: infiniterm is not installed (no app with bundle id {BUNDLE_ID})");
            eprintln!("ift: build it with `npm run tauri build` and put the .app in /Applications");
            ExitCode::from(1)
        }
    }
}

/// `ift install`: a symlink to this very binary in ~/.local/bin.
///
/// A symlink, not a copy, so the bundle's copy stays the one that runs and an
/// updated app updates the command. ~/.local/bin because it needs no sudo and
/// is on most people's PATH already; it says so when it is not.
fn install_self() -> ExitCode {
    let Ok(exe) = std::env::current_exe().and_then(std::fs::canonicalize) else {
        eprintln!("ift: cannot find my own path");
        return ExitCode::from(2);
    };
    let bin = home().join(".local").join("bin");
    if let Err(e) = std::fs::create_dir_all(&bin) {
        eprintln!("ift: cannot create {}: {e}", bin.display());
        return ExitCode::from(2);
    }
    let link = bin.join("ift");
    // The symlink, then the completion, every time: a second `ift install`
    // after an update is how the completion gets its new subcommands, so
    // "already installed" must not stop short of it.
    let linked = matches!(std::fs::read_link(&link), Ok(target) if target == exe);
    if linked {
        println!("ift: already installed at {}", link.display());
    } else {
        if link.exists() && std::fs::read_link(&link).is_err() {
            eprintln!("ift: {} exists and is not a symlink; move it first", link.display());
            return ExitCode::from(2);
        }
        let _ = std::fs::remove_file(&link);
        if let Err(e) = std::os::unix::fs::symlink(&exe, &link) {
            eprintln!("ift: cannot link {}: {e}", link.display());
            return ExitCode::from(2);
        }
        println!("ift: {} -> {}", link.display(), exe.display());
    }
    match completion::install(&home(), std::env::var("FPATH").ok().as_deref()) {
        Ok(lines) => lines.iter().for_each(|l| println!("{l}")),
        Err(e) => eprintln!("ift: {e}"),
    }
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d == bin))
        .unwrap_or(false);
    if !on_path {
        println!("ift: {} is not on your PATH; add it to your shell profile", bin.display());
    }
    ExitCode::SUCCESS
}

/// Opens a path, resolved to something absolute the app can spawn a shell in.
///
/// The KIND is decided here rather than in the app: `ift` is the process standing
/// in the directory the path is relative to, and the app is not.
fn open_path(arg: &str, in_place: bool) -> ExitCode {
    let (path, line) = split_line(arg);
    let full = match resolve(path) {
        Ok(full) => full,
        Err(e) => {
            eprintln!("ift: {e}");
            return ExitCode::from(2);
        }
    };
    let kind = if full.is_dir() { "directory" } else { "file" };
    // A file, from inside a card: in place, waiting (the app's `edit`).
    if in_place && kind == "file" && socket::in_a_card() {
        let mut args = vec![full.to_string_lossy().into_owned()];
        if let Some(line) = line {
            args.push(line.to_string());
        }
        return send_waiting("edit", args);
    }
    let mut args = vec![full.to_string_lossy().into_owned(), kind.to_string()];
    if let Some(line) = line {
        args.push(line.to_string());
    }
    send("open", args)
}

/// `path` made absolute. One that exists is canonicalised; one that does
/// not is a new file, allowed only where its directory exists, since vim
/// finds that out at the first write and that is the worst moment to.
fn resolve(path: &str) -> Result<PathBuf, String> {
    if let Ok(full) = std::fs::canonicalize(path) {
        return Ok(full);
    }
    let given = Path::new(path);
    let name = given
        .file_name()
        .ok_or_else(|| format!("cannot resolve {path}"))?;
    let dir = match given.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let dir = std::fs::canonicalize(&dir)
        .map_err(|_| format!("no such directory: {}", dir.display()))?;
    Ok(dir.join(name))
}

/// How long `ift terminal` waits for a launched app to open its socket:
/// CEF and the canvas load take a few seconds on a cold start.
const LAUNCH_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// `ift terminal <dir>...`: resolves each path here, since the app's working
/// directory is not the caller's, and starts the app first when its socket
/// is not there.
fn terminal_cmd(paths: &[String]) -> ExitCode {
    if paths.is_empty() {
        eprintln!("ift: terminal takes a directory");
        return ExitCode::from(2);
    }
    let mut dirs = Vec::new();
    for p in paths {
        match std::fs::canonicalize(p) {
            Ok(full) if full.is_dir() => dirs.push(full.to_string_lossy().into_owned()),
            Ok(_) => {
                eprintln!("ift: not a directory: {p}");
                return ExitCode::from(2);
            }
            Err(_) => {
                eprintln!("ift: cannot resolve {p}");
                return ExitCode::from(2);
            }
        }
    }
    if !socket::running() {
        if launch() != ExitCode::SUCCESS || !socket::wait_for_socket(LAUNCH_WAIT) {
            eprintln!("ift: infiniterm did not start");
            return ExitCode::from(1);
        }
    }
    send("terminal", dirs)
}

/// `ift diff [path]`: the changes under a directory, or of one file.
fn diff_path(arg: &str) -> ExitCode {
    let Ok(full) = std::fs::canonicalize(arg) else {
        eprintln!("ift: cannot resolve {arg}");
        return ExitCode::from(2);
    };
    let kind = if full.is_dir() { "directory" } else { "file" };
    send("diff", vec![full.to_string_lossy().into_owned(), kind.to_string()])
}

/// `file:42` and `file:42:7`, the way compilers print a location. Only when
/// the suffix parses as a number and the bare path is not itself a file, so a
/// file that really is called `a:1` still opens.
fn split_line(arg: &str) -> (&str, Option<u32>) {
    if Path::new(arg).exists() {
        return (arg, None);
    }
    let mut parts = arg.rsplitn(3, ':');
    let last = parts.next().unwrap_or("");
    let mid = parts.next();
    let rest = parts.next();
    // path:line:col -> the line is in the middle; path:line -> the line is last.
    if let (Some(mid), Some(rest)) = (mid, rest) {
        if let (Ok(line), Ok(_col)) = (mid.parse::<u32>(), last.parse::<u32>()) {
            return (rest, Some(line));
        }
    }
    if let (Some(path), Ok(line)) = (arg.rsplit_once(':').map(|(p, _)| p), last.parse::<u32>()) {
        return (path, Some(line));
    }
    (arg, None)
}

/// `ift ls`'s columns, the order the app answers in (`ift.rs`).
const LS_HEADER: [&str; 6] = ["id", "group", "directory", "state", "remote", "card"];

/// A listing the app answers tab-separated: a table with a header on a
/// terminal, the rows as they came into a pipe (`cut -f1` is the list of
/// ids, and stays so).
fn send_table(cmd: &str, header: &[&str]) -> ExitCode {
    use std::io::IsTerminal;
    match socket::request(cmd, vec![]) {
        Ok(reply) if reply.ok => {
            if reply.text.is_empty() {
                return ExitCode::SUCCESS;
            }
            if std::io::stdout().is_terminal() {
                let rows: Vec<Vec<String>> = reply
                    .text
                    .lines()
                    .map(|l| l.split('\t').map(str::to_string).collect())
                    .collect();
                print!("{}", attach::table(header, &rows));
            } else {
                println!("{}", reply.text);
            }
            ExitCode::SUCCESS
        }
        Ok(reply) => {
            eprintln!("ift: {}", reply.text);
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("ift: {e}");
            ExitCode::from(1)
        }
    }
}

/// `send` for a request answered when you finish, not within seconds: no
/// read timeout, and nothing printed on success, since this runs as
/// `$EDITOR` under git, where stdout is somebody else's.
fn send_waiting(cmd: &str, args: Vec<String>) -> ExitCode {
    match socket::request_waiting(cmd, args) {
        Ok(reply) if reply.ok => ExitCode::SUCCESS,
        Ok(reply) => {
            eprintln!("ift: {}", reply.text);
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("ift: {e}");
            ExitCode::from(1)
        }
    }
}

/// Sends one request and prints the answer.
fn send(cmd: &str, args: Vec<String>) -> ExitCode {
    match socket::request(cmd, args) {
        Ok(reply) if reply.ok => {
            if !reply.text.is_empty() {
                println!("{}", reply.text);
            }
            ExitCode::SUCCESS
        }
        Ok(reply) => {
            eprintln!("ift: {}", reply.text);
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("ift: {e}");
            ExitCode::from(1)
        }
    }
}

/// The path after `-n`. `args` already left the program name out, so it
/// is the second argument; it read the third, and `ift -n <file>` said
/// "-n takes a path" (#48).
fn new_card_path(args: &[String]) -> Option<&str> {
    args.get(1).map(String::as_str).filter(|p| !p.is_empty())
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

/// What every installer says when `hook_binary` finds nothing: the bundle
/// ships it beside `ift`, and a checkout builds it into the workspace's
/// `target/` beside `ift` too (the old `crates/infiniterm-hook` path was
/// the Tauri app's, #31).
fn hook_missing() -> ExitCode {
    eprintln!("ift: cannot find the infiniterm-hook binary");
    eprintln!("ift: in a checkout, build it beside ift:");
    eprintln!("  cargo build --release -p infiniterm-hook");
    eprintln!("ift: or put it on $PATH next to ift");
    ExitCode::from(2)
}

/// Where `infiniterm-hook` is, given where `ift` is.
///
/// `None` when it cannot be found, which the caller treats very differently from
/// a guess — see `install_hooks`.
///
/// Two places, in order: next to `ift`, which is where an install puts them, and
/// the sibling crate's release output, which is where they are in a checkout.
/// Without the second, running this from the repo finds nothing and the wiring is
/// written by bare name.
fn hook_binary() -> Option<String> {
    // Canonical, so a symlinked ift (`ift install`) looks beside the real
    // binary in the bundle, not beside the link in ~/.local/bin.
    let exe = std::env::current_exe().and_then(std::fs::canonicalize).ok()?;
    let dir = exe.parent()?;
    let candidates = [
        dir.join("infiniterm-hook"),
        // crates/infiniterm-cli/target/release/ift -> crates/infiniterm-hook/...
        dir.join("../../../infiniterm-hook/target/release/infiniterm-hook"),
    ];
    candidates
        .iter()
        .find(|p| p.exists())
        .and_then(|p| std::fs::canonicalize(p).ok())
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| which_on_path("infiniterm-hook"))
}

/// The first `infiniterm-hook` on `$PATH`, if there is one.
fn which_on_path(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

fn install_hooks(dry_run: bool) -> ExitCode {
    let path = home().join(".claude").join("settings.json");
    wire_hook_file(&path, &claude_hooks::EVENTS, None, dry_run)
}

const CURSOR_ADAPTER: &str = include_str!("../adapters/cursor-hook.sh");

fn cursor_adapter_source(hook: &str) -> String {
    CURSOR_ADAPTER.replace("__INFINITERM_HOOK__", hook)
}

fn cursor_hooks_dir() -> std::path::PathBuf {
    home().join(".cursor")
}

fn install_cursor(dry_run: bool) -> ExitCode {
    let Some(binary) = hook_binary() else {
        return hook_missing();
    };
    let dir = cursor_hooks_dir();
    let script = dir.join("hooks").join("infiniterm-cursor-hook.sh");
    let hooks_json = dir.join("hooks.json");
    let body = cursor_adapter_source(&binary);

    if std::fs::read_to_string(&script).ok().as_deref() != Some(body.as_str()) {
        if dry_run {
            println!("would write {}", script.display());
        } else if let Err(code) = write_cursor_script(&script, &body) {
            return code;
        }
    } else {
        println!("already installed: {}", script.display());
    }

    wire_cursor_hooks_file(&hooks_json, dry_run)
}

fn write_cursor_script(path: &std::path::Path, body: &str) -> Result<(), ExitCode> {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("ift: could not create {}: {e}", parent.display());
            return Err(ExitCode::from(2));
        }
    }
    if let Err(e) = infiniterm_core::files::write_atomically(path, body) {
        eprintln!("ift: could not write {}: {e}", path.display());
        return Err(ExitCode::from(2));
    }
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o755);
        let _ = std::fs::set_permissions(path, perms);
    }
    println!("installed {}", path.display());
    Ok(())
}

fn wire_cursor_hooks_file(path: &std::path::Path, dry_run: bool) -> ExitCode {
    let existing = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".into());
    let mut settings: serde_json::Value = match serde_json::from_str(&existing) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ift: {} does not parse as JSON ({e})", path.display());
            eprintln!("ift: refusing to rewrite it; fix the file and run again");
            return ExitCode::from(2);
        }
    };

    let changed = cursor_hooks::install(&mut settings);
    if changed.is_empty() {
        println!("already wired: {}", path.display());
        return ExitCode::SUCCESS;
    }

    if dry_run {
        println!("would update {} for: {}", path.display(), changed.join(", "));
        return ExitCode::SUCCESS;
    }

    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("ift: could not create {}: {e}", dir.display());
            return ExitCode::from(2);
        }
    }

    let body = serde_json::to_string_pretty(&settings).unwrap_or_default() + "\n";
    if let Err(e) = infiniterm_core::files::write_atomically(path, &body) {
        eprintln!("ift: could not write {}: {e}", path.display());
        return ExitCode::from(2);
    }

    println!("updated {}", path.display());
    println!("  events: {}", changed.join(", "));
    println!("  takes effect in the next cursor agent session");
    ExitCode::SUCCESS
}

/// Codex reads hooks from `$CODEX_HOME/hooks.json`, `~/.codex` by default,
/// in Claude's shape. It asks once before running hooks it did not write
/// itself, so the install says where to approve them.
fn install_codex(dry_run: bool) -> ExitCode {
    let dir = std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home().join(".codex"));
    let code = wire_hook_file(
        &dir.join("hooks.json"),
        &claude_hooks::CODEX_EVENTS,
        Some("codex"),
        dry_run,
    );
    if code == ExitCode::SUCCESS && !dry_run {
        println!("  Codex runs them once you approve them: type /hooks in codex");
    }
    code
}

/// Merges infiniterm's hooks into a JSON hook file of Claude's shape
/// (`claude_hooks::install_events`) and writes it back, refusing a file
/// that does not parse.
fn wire_hook_file(
    path: &std::path::Path,
    events: &[&str],
    agent: Option<&str>,
    dry_run: bool,
) -> ExitCode {
    let existing = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".into());

    let mut settings: serde_json::Value = match serde_json::from_str(&existing) {
        Ok(v) => v,
        Err(e) => {
            // Refused rather than replaced. A file that does not parse is one
            // somebody is mid-edit on, or has a typo in they need to see — and it
            // is the wrong moment to overwrite their permissions and env.
            eprintln!("ift: {} does not parse as JSON ({e})", path.display());
            eprintln!("ift: refusing to rewrite it; fix the file and run again");
            return ExitCode::from(2);
        }
    };

    let Some(binary) = hook_binary() else {
        // Refusing rather than guessing. An earlier version fell back to the bare
        // name here, which REWROTE working absolute paths into `infiniterm-hook`
        // and silently broke a setup that was already correct — the one outcome
        // this command exists to prevent.
        return hook_missing();
    };
    let changed = claude_hooks::install_events(&mut settings, &binary, events, agent);

    if changed.is_empty() {
        println!("already wired: {}", path.display());
        return ExitCode::SUCCESS;
    }

    if dry_run {
        println!("would update {} for: {}", path.display(), changed.join(", "));
        return ExitCode::SUCCESS;
    }

    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("ift: could not create {}: {e}", dir.display());
            return ExitCode::from(2);
        }
    }

    // Write-then-rename, so an interrupted write cannot leave a truncated
    // settings.json — which would take the user's permissions and env with it.
    // Through a symlink, not over it: ~/.claude/settings.json is often a link
    // into a dotfiles repo (#219).
    let body = serde_json::to_string_pretty(&settings).unwrap_or_default() + "\n";
    if let Err(e) = infiniterm_core::files::write_atomically(path, &body) {
        eprintln!("ift: could not write {}: {e}", path.display());
        return ExitCode::from(2);
    }

    println!("updated {}", path.display());
    println!("  hook: {binary}");
    println!("  events: {}", changed.join(", "));
    ExitCode::SUCCESS
}

/// The OpenCode plugin, with the hook path filled in; see
/// adapters/opencode.js for what it does.
const OPENCODE_ADAPTER: &str = include_str!("../adapters/opencode.js");

fn opencode_adapter_source(hook: &str) -> String {
    OPENCODE_ADAPTER.replace("__INFINITERM_HOOK__", hook)
}

/// OpenCode loads plugins from `$XDG_CONFIG_HOME/opencode/plugins`, else
/// `~/.config/opencode/plugins` (it globs `plugin` and `plugins` alike).
fn opencode_plugin_path() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("opencode")
        .join("plugins")
        .join("infiniterm.js")
}

fn install_opencode(dry_run: bool) -> ExitCode {
    let Some(binary) = hook_binary() else {
        return hook_missing();
    };
    let path = opencode_plugin_path();
    write_adapter(&path, &opencode_adapter_source(&binary), &binary, "opencode", dry_run)
}

/// Writes an adapter file (write-then-rename), saying so; unchanged files
/// are left alone.
fn write_adapter(
    path: &std::path::Path,
    body: &str,
    binary: &str,
    agent: &str,
    dry_run: bool,
) -> ExitCode {
    if std::fs::read_to_string(path).ok().as_deref() == Some(body) {
        println!("already installed: {}", path.display());
        return ExitCode::SUCCESS;
    }
    if dry_run {
        println!("would write {}", path.display());
        return ExitCode::SUCCESS;
    }
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("ift: could not create {}: {e}", parent.display());
            return ExitCode::from(2);
        }
    }
    // Through a symlink, not over it (#219).
    if let Err(e) = infiniterm_core::files::write_atomically(path, body) {
        eprintln!("ift: could not write {}: {e}", path.display());
        return ExitCode::from(2);
    }
    println!("installed {}", path.display());
    println!("  hook: {binary}");
    println!("  takes effect in the next {agent} session");
    ExitCode::SUCCESS
}

/// The Pi adapter, with the hook path filled in. Baked into the binary so
/// `ift` is the one thing to install; see adapters/pi.ts for what it does.
const PI_ADAPTER: &str = include_str!("../adapters/pi.ts");

fn pi_adapter_source(hook: &str) -> String {
    PI_ADAPTER.replace("__INFINITERM_HOOK__", hook)
}

/// Where Pi loads extensions from: `$PI_CODING_AGENT_DIR`, else ~/.pi/agent.
fn pi_agent_dir(explicit: Option<&str>) -> std::path::PathBuf {
    if let Some(dir) = explicit {
        return expand_home(dir);
    }
    if let Some(dir) = std::env::var_os("PI_CODING_AGENT_DIR") {
        return std::path::PathBuf::from(dir);
    }
    home().join(".pi").join("agent")
}

fn expand_home(path: &str) -> std::path::PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if path == "~" => home(),
        None => std::path::PathBuf::from(path),
    }
}

fn install_pi(dir: Option<&str>, dry_run: bool) -> ExitCode {
    let Some(binary) = hook_binary() else {
        return hook_missing();
    };
    let agent_dir = pi_agent_dir(dir);
    let path = agent_dir.join("extensions").join("infiniterm.ts");
    let body = pi_adapter_source(&binary);
    if std::fs::read_to_string(&path).ok().as_deref() == Some(body.as_str()) {
        println!("already installed: {}", path.display());
        return ExitCode::SUCCESS;
    }
    if dry_run {
        println!("would write {}", path.display());
        return ExitCode::SUCCESS;
    }
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("ift: could not create {}: {e}", parent.display());
            return ExitCode::from(2);
        }
    }
    // Through a symlink, not over it (#219).
    if let Err(e) = infiniterm_core::files::write_atomically(&path, &body) {
        eprintln!("ift: could not write {}: {e}", path.display());
        return ExitCode::from(2);
    }
    println!("installed {}", path.display());
    println!("  hook: {binary}");
    println!("  takes effect in the next pi session");
    ExitCode::SUCCESS
}

/// `ift install-extension <path|id|store-url>`: filesystem only, no socket,
/// the same shape as `install-pi-hooks` — the browser picks a new one up
/// on its own next launch, not this app's, so there is nothing to tell the
/// running app about.
fn install_extension(source: Option<&str>) -> ExitCode {
    let Some(source) = source else {
        eprintln!("ift: install-extension needs a path, an id, or a Chrome Web Store url");
        return ExitCode::from(2);
    };
    match infiniterm_core::extensions::install(&infiniterm_core::paths::browser_dir(), source) {
        Ok(dest) => {
            println!("installed {}", dest.display());
            println!("  takes effect on infiniterm's next launch");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("ift: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `ift -n <file>`: the path is the argument right after `-n`.
    #[test]
    fn dash_n_takes_the_next_argument_as_its_path() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(new_card_path(&a(&["-n", "CLAUDE.md"])), Some("CLAUDE.md"));
        assert_eq!(new_card_path(&a(&["-n"])), None);
    }

    // The OpenCode plugin is the template with the hook path in it, one
    // plugin export (OpenCode calls every export as a plugin), and it names
    // itself to the hook so the card knows whose session it is.
    #[test]
    fn the_opencode_adapter_gets_the_hook_path_and_names_its_agent() {
        let src = opencode_adapter_source("/Applications/x.app/Contents/MacOS/infiniterm-hook");
        assert!(src.contains("const HOOK = \"/Applications/x.app/Contents/MacOS/infiniterm-hook\";"));
        assert!(!src.contains("__INFINITERM_HOOK__"));
        assert!(src.contains("[event, \"opencode\"]"));
        assert_eq!(src.matches("export ").count(), 1);
    }

    #[test]
    fn the_pi_adapter_gets_the_hook_path_and_nothing_else_changes() {
        let src = pi_adapter_source("/Applications/x.app/Contents/MacOS/infiniterm-hook");
        assert!(src.contains("const HOOK = \"/Applications/x.app/Contents/MacOS/infiniterm-hook\";"));
        assert!(!src.contains("__INFINITERM_HOOK__"));
        assert!(src.contains("pi.on(\"agent_settled\""));
    }

    #[test]
    fn the_cursor_adapter_gets_the_hook_path_and_documents_cursor_events() {
        let src = cursor_adapter_source("/Applications/x.app/Contents/MacOS/infiniterm-hook");
        assert!(src.contains("HOOK=\"/Applications/x.app/Contents/MacOS/infiniterm-hook\""));
        assert!(!src.contains("__INFINITERM_HOOK__"));
        assert!(src.contains("beforeSubmitPrompt"));
        assert!(src.contains("postToolUseFailure"));
    }

    #[test]
    fn the_pi_agent_dir_prefers_an_explicit_one_then_the_env_then_the_default() {
        assert_eq!(pi_agent_dir(Some("/tmp/x")), std::path::PathBuf::from("/tmp/x"));
        assert_eq!(pi_agent_dir(Some("~/.agents-pig")), home().join(".agents-pig"));
        // The env var and default depend on the machine; only their shape is checked.
        let d = pi_agent_dir(None);
        assert!(d.is_absolute());
    }

    #[test]
    fn a_line_suffix_is_split_off() {
        assert_eq!(split_line("/tmp/nope.rs:42"), ("/tmp/nope.rs", Some(42)));
        assert_eq!(split_line("/tmp/nope.rs:42:7"), ("/tmp/nope.rs", Some(42)));
        assert_eq!(split_line("/tmp/nope.rs"), ("/tmp/nope.rs", None));
        assert_eq!(split_line("/tmp/nope:x"), ("/tmp/nope:x", None));
    }

    // A new file resolves where its directory exists and is refused where
    // it does not, before anything opens.
    #[test]
    fn a_new_file_needs_its_directory() {
        let dir = std::env::temp_dir().join(format!("ift-resolve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        let new = dir.join("notes.txt");
        assert_eq!(resolve(new.to_str().unwrap()).unwrap(), new);
        let missing = dir.join("nope/notes.txt");
        let err = resolve(missing.to_str().unwrap()).unwrap_err();
        assert!(err.starts_with("no such directory"), "{err}");
        std::fs::write(&new, "x").unwrap();
        assert_eq!(resolve(new.to_str().unwrap()).unwrap(), new, "existing");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
