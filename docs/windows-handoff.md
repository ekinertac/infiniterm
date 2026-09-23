# Windows port: handoff

For a Claude session on Ekin's Windows desktop (the "gpu box", RTX 4070 Ti), starting from nothing but this repo. Written 2026-09-23 on the Mac by the session that built most of the tree. Read `CLAUDE.md` first: it holds the rules and the traps, and most of them still apply. This file is only what is different about Windows.

## The goal, and what it is not

Claude Code running in terminal cards on the infinite canvas, on Windows, with the editor, diff and transcript cards, the palette, workspaces, groups and agent colours from hooks. A lesser app than the Mac one, on purpose:

- **No daemon.** On macOS every card's shell lives in its own `iftd` process so it outlives the app. On Windows the terminal backend is `pty`: the card owns its shell directly through `portable-pty`, which uses ConPTY. Quitting, restarting or updating the app ends every shell; a card comes back as a fresh shell in its saved directory, and Claude sessions come back with `claude --resume`.
- Why not port `iftd`: ConPTY is itself a terminal emulator that re-renders the program's output before we see it. `iftd` exists so that nothing stands between Claude and our own parser and a reattach can replay the exact bytes; ConPTY is precisely the second emulator that design routes around. It needs a design of its own, later, if ever.
- No `ift attach`, no `ift sessions`, no scrollback on disk across a power cut.
- Browser cards come last (CEF on Windows is a phase of its own), and signing, packaging and the updater after that.

## Ground rules for this session

- **macOS must keep building and working.** Every Windows change goes behind `#[cfg(windows)]` / `#[cfg(unix)]` (or `target_os = "macos"` where the thing is Mac-specific rather than Unix-specific). Never delete a Mac path to make Windows compile.
- **Work on the `windows` branch**, push it, never force-push master, never merge into master yourself. The Mac session merges after `make check` passes on macOS, because only the Mac can check the Mac. Rebase `windows` onto master often; two other sessions (the Mac port session and `ift-browser`) commit to master daily.
- Commit messages say why, no attribution trailers of any kind, stage files by name. Every new file starts with a header block (responsibility, where it fits, callers, constraints). Tests always: logic is a pure function in its own file with tests, the gpui side is wiring.
- `infiniterm-browser/`, `browser_body.rs`, `browsers.rs` and `omnibox.rs` belong to the `ift-browser` session. For phases 1 to 4 you compile the browser OUT (see phase 1), so you should not need to touch them; phase 5 is coordinated with that session through Ekin.
- There is no GUI driver on Windows. Verifying on screen means Ekin at the desktop. Say what to look at, and wait.

## Setting up the box

The remote shell over `ssh win-gpu-box` is PowerShell 7; use pwsh syntax and quote Windows paths. For GUI work the session must run on the desktop itself (or over Remote Desktop), not through ssh.

1. Rust via rustup with the MSVC toolchain, and the Visual Studio Build Tools "Desktop development with C++" workload (gpui and tree-sitter compile C and C++).
2. `gh auth login`, then `gh repo clone ekinertac/infiniterm` (the repo is private).
3. `.cargo/config.toml` sets `CEF_PATH` to a macOS path with `force = false`, so an environment variable wins. You need CEF only from phase 5; until then the browser is compiled out.
4. The Makefile is macOS-only (`make bundle`, signing, notarization, the driver). On Windows use cargo directly: `cargo test -p infiniterm-core -p infiniterm-term -p infiniterm-editor`, `cargo build -p infiniterm-ui`, `cargo clippy -- -D warnings` on what you touch. Never `cargo fmt --all` (it rewrites the cli and hook crates; see CLAUDE.md's traps); format only the crates you change.

## What is Mac or Unix specific, file by file

Measured 2026-09-23 on master. Each one needs a Windows branch or a cfg gate.

**Unix sockets.** Rust's standard library has no Unix sockets on Windows. Used for the hook reports and `ift` requests, one socket at `/tmp/infiniterm.sock` (or `<INFINITERM_DATA_DIR>/infiniterm.sock`, `paths::socket_path`):
- `infiniterm-core/src/hooks.rs` (the listener, and the "is another instance running" probe that makes the socket the single-instance lock)
- `infiniterm-core/src/app.rs` (tests connecting to it)
- `infiniterm-ui/src/runtime.rs:~739` (an "is it up" probe)
- `infiniterm-hook/src/main.rs` (the hook binary; zero dependencies on purpose, keep it that way)
- `infiniterm-cli/src/socket.rs` (`ift`)

Recommended: a small transport module in core with one API (listen, connect, the probe) and two implementations, a Unix socket on unix and a named pipe (`\\.\pipe\infiniterm`, or `\\.\pipe\infiniterm-<hash of the data dir>` so `INFINITERM_DATA_DIR` still isolates a scratch instance) on Windows. Named pipes are std-reachable through `std::fs::OpenOptions` for the client; the server side needs `windows-sys` (CreateNamedPipeW / ConnectNamedPipe). The hook binary's zero-dependency rule can bend to `windows-sys` on Windows only, behind cfg; say so in its header.

**The daemon.** `infiniterm-session/` (`iftd`) and `infiniterm-core/src/backend/daemon.rs` are Unix-only (libc, fork, Unix sockets). Gate the whole backend `#[cfg(unix)]`, and on Windows make `terminal.backend` default to `pty` (`config.rs` sets `TerminalBackend::Daemon` as the default, around line 208) and refuse `daemon` and `tmux` with a notice, the way the pty backend already refuses `app.restart`'s promise. `infiniterm-session` should not build on Windows at all (a `[target.'cfg(unix)'.dependencies]` shape or a cfg on the whole binary).

**`ift attach` / `ift sessions`.** `infiniterm-cli/src/attach.rs` uses libc for raw terminal mode. Gate it `#[cfg(unix)]`; on Windows `ift attach` prints that it needs the daemon, which Windows does not have, and exits 2. Keep `SUBCOMMANDS` and `completion.rs` consistent (a test holds them together); the zsh completion is irrelevant on Windows.

**The pty backend itself.** `infiniterm-core/src/backend/local_pty.rs` mostly works through `portable-pty` already. Windows-specific:
- `default_shell()` reads `$SHELL` and falls back to `/bin/zsh`. On Windows: `pwsh.exe` if on PATH, else `powershell.exe`, else `%COMSPEC%`.
- A card's command runs as `<shell> -lc <cmd>` (local_pty.rs ~205, also `iftd` ~282): pwsh takes `-NoLogo -Command <cmd>`, cmd takes `/C <cmd>`. Make it a pure function of the shell's name, with tests.
- Tests spawn in `/tmp` (~427-452); use `std::env::temp_dir()`.
- The environment scrubbing of the parent terminal's identity and `TERM_PROGRAM=infiniterm` still apply.
- Process labels and cwd come from `ps` and `lsof` (`infiniterm-core/src/inspect.rs`). On Windows: the child process tree from Toolhelp32 (`windows-sys`), and the cwd is not readable from outside a process on Windows without debug APIs; accept "the directory the card started in" there and say so.

**Keys.** `infiniterm-ui/src/keycode.rs` keeps an `NSEvent` monitor for the physical key code, the real Shift state and dead keys, because gpui's `Keystroke` loses them on non-US layouts (Ekin types on Turkish Q; see CLAUDE.md's traps about Cmd+= arriving as Cmd+Shift+0). On Windows the equivalent is the virtual key and scan code from the window message. Check first whether gpui's Windows keystroke already carries enough; if it does, `keycode.rs` just returns `None` on Windows and `keymap.rs` falls back to the keystroke. Also decide the modifier: every app binding is on Cmd (`keymap.rs::is_allowed_chord`), and Cmd does not exist on Windows. Map Cmd to Ctrl+Alt? To the Windows key? That is Ekin's call, and it shapes the whole keymap; ask before building it. Terminals must keep Ctrl.

**Cocoa.** `objc` and `msg_send!` in `infiniterm-ui/src/main.rs` (the app menu, reduced motion, `open`), `input.rs`, `overlays.rs`, `keycode.rs`, and CEF's `infiniterm-browser/src/app_protocol.rs`. Gate them `target_os = "macos"`; gpui's own menu and window APIs cover what Windows needs.

**Shelling out to macOS tools.** `open` (cli `main.rs:158`, `links_fs.rs:58`, ui `main.rs:281`) becomes `cmd /C start "" <path>` or `explorer.exe`. `ditto`, `codesign`, `spctl`, `PlistBuddy`, `shasum` are the updater's and the build's; the updater is macOS-only for now (`updater.rs` / `update.rs::applies_to` already refuse anything outside an Applications folder; gate the start `target_os = "macos"`).

**Paths.** `paths.rs` puts data under `~/Library/Application Support/dev.ekinertac.infiniterm`; on Windows use `%APPDATA%\infiniterm`. Config is `~/.config/infiniterm/` on Mac; `%APPDATA%\infiniterm\config` on Windows is fine, `INFINITERM_CONFIG_DIR` still overrides. `ift install` symlinks into `~/.local/bin`; on Windows, print the folder to add to PATH instead (symlinks need Developer Mode).

**Defaults that name Mac things.** `terminal.fontFamily` defaults to `ui-monospace, Menlo, monospace`; add `Cascadia Mono, Consolas` for Windows. `terminal.decoyCommand` defaults to `command log stream --style compact` (a Mac command); on Windows something like `Get-Process | Sort CPU -Desc | Select -First 30` looped, or leave the mask off.

## Phases, each with its done line

1. **The core crates build and pass their tests on Windows.** `infiniterm-core`, `infiniterm-term`, `infiniterm-editor`: the daemon and tmux backends gated unix, the socket behind the transport module, `/tmp` in tests replaced. A `browser` cargo feature on `infiniterm-ui` (default on, so macOS is unchanged) that compiles `infiniterm-browser` and CEF out and draws a stub card ("browser cards are not in this build") in their place; this is also the build-time Lite/Pro split planned in ekinertac/notes#50, so build it the way that plan says (the stub card, no runtime key) and tell Ekin, because the `ift-browser` session owns that crate. Done: `cargo test` green for the three crates on Windows, and `make check` still green on the Mac (the Mac session runs it when merging).
2. **A window opens and a terminal card runs pwsh.** `cargo build -p infiniterm-ui --no-default-features` (browser off), then run it. Keys typed reach the shell, Enter runs a command, output draws, a resize reflows. Done: Ekin types `dir` in a card and sees the listing.
3. **The canvas and the app keys.** The modifier decision from "Keys" above, then pan, zoom, new card, focus movement, the palette, workspaces. Done: Ekin can do a morning's work on the canvas without reaching for a Mac-only chord.
4. **Claude in a card with hooks.** The named-pipe transport, the hook binary on Windows, `ift install-claude-hooks` writing the Windows path of `infiniterm-hook.exe` into `%USERPROFILE%\.claude\settings.json`. Check that Claude Code's Shift+Enter still makes a line break through ConPTY (the kitty keyboard protocol; recent ConPTY passes it through, older ones swallow it). Done: a Claude card turns working, waiting and done colours as it runs.
5. **Browser cards.** CEF for Windows through cef-rs (its README has the PowerShell steps), with the `ift-browser` session. Done: a browser card loads a page.
6. **Shipping.** A zip or MSI, Authenticode signing (a paid certificate; SmartScreen warns until it has reputation), and the updater's Windows half (the manifest and "newer is a higher build" logic in `update.rs` carry over; the signature check becomes WinVerifyTrust, and the swap has to wait for the exe to unlock). Ekin decides when; not before phases 1 to 4 are real.

## Things that will bite

- ConPTY rewrites what the program writes, redraws on resize, and older builds of it swallow escape sequences newer programs rely on. When something renders wrong, check whether Windows Terminal shows the same program correctly before blaming our parser.
- Line endings: set `core.autocrlf false` on the clone, or `.gitattributes` fights begin; the Mac side commits LF.
- A path in a card label, a drop, or `ift <path>` has backslashes and a drive letter; `shell_quote` (`drop.rs`) is POSIX quoting and must get a PowerShell twin before a dropped file reaches a pwsh prompt, because that function is the whole security story for a file named `; rm -rf ~`.
- The single-instance lock is the socket today; on Windows it is the named pipe, and a crashed instance leaves no stale file to clean (unlike the Unix socket), which is simpler.
- Report back in plain text what you changed, what you checked and what you could not, and ask Ekin to relay anything that needs the Mac session.
