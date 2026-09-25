# Windows port: handoff

For a Claude session on Ekin's Windows desktop (the "gpu box", RTX 4070 Ti), starting from nothing but this repo. Written 2026-09-23 on the Mac by the session that built most of the tree. Read `CLAUDE.md` first: it holds the rules and the traps, and most of them still apply. This file is only what is different about Windows.

## Where this stands

**Every phase is done, 2026-09-23 to -25.** Every crate builds and passes
its tests on Windows with the browser feature ON (787 core, 72 ui, 44 term,
34 editor, 28 cli, 8 browser; clippy clean), the window opens, a card runs
PowerShell, Claude Code runs in a card with its hooks reaching the app, the
app chords work (cards, workspaces, the omnibox, focus, fit-all), a card
names itself from the process table, and `tools/dist.ps1` makes an unpacked
release folder, a zip and a manifest that the app updates itself from.

Phase 5, browser cards, landed on 2026-09-25 and was checked on screen
rather than only compiled: a CEF card renders a real page with its images
and mixed scripts, the omnibox navigates it, a click lands where the
screenshot says and focuses the field under it, typing and the named keys
reach the page, the wheel scrolls it, and the right-click menu opens. What
that cost is under "What phase 5 changed" below.

**The modifier question under "Keys" below is answered.** Windows gets its
Cmd from the SIDE of Ctrl: left is the app's, right is the terminal's, so a
keyboard remapped into Mac order (Ekin's is; the key where Cmd sits emits
left Ctrl and Caps Lock emits right Ctrl) puts all 79 bindings under the
finger that already reaches for them, and Caps Lock plus C is still a real
`^C`. The keymap needs no Windows edition at all. It is a setting,
`keyboard.commandModifier`, with `win` as the alternative for an un-remapped
machine. Rejected: the Win key alone (the OS takes Win+L, D, E, R, S, T, the
digits, the arrows and =, and Win+L locks the box), Ctrl+Alt (that is AltGr,
which is how Turkish Q types `{ [ ] } \ @`), bare Ctrl (the carve-out list
differs per program in a card), and Ctrl+Shift (collides with itself on the
eighteen bindings that already carry Shift).

There IS a small GUI driver now, `tools/drive/win/`, and it is how every
claim above was checked. `run.ps1` launches on a scratch data dir,
`shot.ps1` screenshots the window, `post.ps1` types into it and `click.ps1`
clicks or scrolls in it without taking the foreground, `type.ps1` and
`chord.ps1` send real OS input when the question is about the keyboard
itself, `kill.ps1` stops it, `awake.ps1` keeps the box from blanking, and
`window.ps1` is the one handle lookup they share.

Three things about it are worth knowing before trusting a result:

- **`window.ps1` exists because `Get-Process infiniterm` returns seven
  processes** once a browser card is open. Windows re-executes the same exe
  for every Chromium subprocess, and `.MainWindowHandle` on that array is
  whichever one PowerShell listed first.
- **`post.ps1` posts WM_CHAR, and must keep doing so.** Posting key messages
  looks right and is not: the app's pump calls TranslateMessage, which turns
  a posted WM_KEYDOWN into a character anyway, and whether it lands twice
  depends on where the frame loop is. A quarter of the characters doubled,
  so short strings usually came out right and the driver looked honest while
  a URL arrived as nonsense. `chord.ps1` is the cross-check: real hardware
  input, and it typed the same string cleanly.
- **`click.ps1` takes shot.ps1's coordinates**, not the client area's, so a
  session can read a pixel off a screenshot and click it.

What phase 1 actually changed, beyond the plan below:

- `infiniterm-core/src/transport.rs` is the one IPC channel, a unix socket on
  unix and a named pipe on Windows. `paths::socket_path` answers a pipe name
  there, with the data dir hashed into it so `INFINITERM_DATA_DIR` still
  isolates a scratch instance. `ift` dropped its own copy of both; the hook
  binary keeps one, and keeps its zero dependencies (the CLIENT half of a
  named pipe is std).
- `infiniterm-core/src/shell_cmd.rs` holds which shell a card gets and how to
  hand it one command, decided from the shell's file NAME, not the OS. Git
  Bash on Windows is still POSIX and takes `-lc`.
- `infiniterm-ui` grew a `browser` feature, on by default. Off, it compiles
  CEF out and `browser_stub.rs` IS the browsers module (`#[path]`), so every
  call site stays as it is. This is also the Lite/Pro split's shape
  (ekinertac/notes#50); the `ift-browser` session has not been told yet.
- `infiniterm-session` builds on Windows to a binary that refuses, because a
  workspace member cannot be cfg'd out and the root's `cargo test` builds
  every one. Its body moved to `daemon.rs`.
- Two real bugs, not Windows-only in principle: fifteen places took a file's
  name by splitting on `/` alone, and three derived a parent directory the
  same way (`C:\a` gave `C:`, which names a drive's cursor rather than its
  root). Both are `paths::base_name` and `paths::parent_dir` now.
- `tree-sitter-scss` cannot build on MSVC at all (see the traps below), so
  `.scss` opens without highlighting on Windows.

What phase 2 changed:

- Windows gets a real caption bar. `appears_transparent` means "we draw the
  whole title bar" and gpui on Windows takes it literally, which left the
  window with no close, minimise or maximise button: the traffic lights we
  leave room for are macOS's and we never drew any others.
- The 84-pixel inset that keeps the workspace tabs clear of those traffic
  lights is 8 on Windows, where it was that much empty space.

What phase 5 changed:

- **Windows has no helper binary, and needs none.** macOS runs each Chromium
  subprocess from an `infiniterm Helper.app`; Windows re-executes the main
  exe, which `process::early` already handles by calling `execute_process`
  before gpui starts. So `bin/helper.rs`, the framework loader and
  `app_protocol` (CEF's two methods on gpui's NSApplication) are all gated
  to macOS. `early` now tells a subprocess apart by Chromium's own `--type=`
  switch and skips the profile seeding, or half a dozen children would each
  copy the same directories over each other at launch.
- `no_sandbox` is set on Windows, because we hand `initialize` a null
  `sandbox_info`. On macOS the helper bundles carry the sandbox instead.
- `native_key_code` is now two tables. It is the physical key in the
  platform's own codeset: Carbon's on macOS, where Cocoa resolves Backspace
  and the arrows from it, and the SCAN CODE on Windows, where Chromium turns
  it into the `code` a page's key handler reads. Left at the macOS value, a
  page heard `code: "Comma"` for every Backspace — the key still worked,
  which is why it could have sat there for a long time. The arrows and the
  navigation block carry the E0 prefix that tells them from the keypad keys
  they share a code with.
- `objc` moved to a macOS-only dependency of `infiniterm-browser`; as an
  unconditional one it put `objc.lib` on the Windows link line.
- CEF is set up with `CEF_PATH` pointing at a Windows distribution (see
  "Setting up the box"); the `cef` crate downloads it and its build script
  copies libcef.dll and the rest into `target/debug` beside the exe.

NOTE the ownership rule in CLAUDE.md: `infiniterm-browser/`,
`browser_body.rs`, `browsers.rs` and `omnibox.rs` are the `ift-browser`
session's files. That rule is about ONE shared checkout on the Mac and this
is a separate clone, so there is no shared index to sweep, but these changes
still have to merge. `surface.rs` and `process.rs` are the two that carry
real edits.

What is still open:

- **The Mac has not seen any of this.** `make check` on macOS is the gate
  before a merge, and only the Mac session can run it. Everything here is
  `cfg`-gated, but a `#[cfg(not(windows))]` arm is only as good as the
  compiler that saw it.
- **A killed CEF app locks its own binary.** Force-killing leaves one thread
  of the main process alive in the kernel: the process stays enumerable with
  `HasExited` true and goes on holding `infiniterm.exe` and the Chromium
  dlls, so the next build fails on "Access is denied". It did not clear in a
  minute of watching. `kill.ps1` asks with WM_CLOSE first for that reason,
  and `-FreeLocks` moves a stuck exe aside. It deliberately leaves the dlls
  alone: moving libcef.dll aside does not get a new one, because cargo will
  not re-run cef-dll-sys's build script when its inputs have not changed,
  and the app then starts and dies instantly with no Chromium beside it.
  The way out of that is `cargo clean -p cef-dll-sys` and a minute.
- **A cold CEF start takes over thirty seconds.** CEF is initialised inside
  gpui's run closure, before the window opens, so a first launch on a fresh
  profile directory shows nothing at all for that long. `run.ps1` waits a
  minute now. It is a second or two on a warm profile.
- The omnibox's `file:///C:/...` reading. It navigates correctly, but the
  suggestion row shows the address as the path `/C:/Users/...`. Cosmetic,
  and the omnibox is the `ift-browser` session's file.

  **The hash is the only check on Windows, deliberately.** Ekin decided
  against Authenticode on 2026-09-25 knowing what it costs: the Mac requires
  a download to be signed by his team and notarized, so a zip somebody else
  built is refused even when the manifest names its hash; here the manifest
  is the authority, and whoever can write the releases repo can run code on
  a Windows machine. `parse_manifest` refusing a url outside that repo, over
  TLS, is the floor it rests on. Do not quietly widen this: a manifest field
  that could point the download elsewhere would remove the last of it.

- Transcript cards are the one card kind never opened on Windows. Ekin is
  testing that (2026-09-25). The Claude card this session ran had transcript
  saving off because it inherited `CLAUDE_CODE_CHILD_SESSION` from the
  agent that launched the app, which is not a Windows fault: `local_pty`
  scrubs the parent TERMINAL's identity variables and not an agent's, so
  launching the app from inside a Claude card does the same on a Mac.
- A card's DIRECTORY never changes on Windows. A process's cwd cannot be
  read from outside it without debug privileges, so `inspect::cwds` answers
  nothing there and a card keeps the directory it opened in. The fix is not
  a Windows one: `PaneEvent::CwdChanged` exists and NOTHING emits or
  consumes it on either platform, while oh-my-posh already writes OSC 7. A
  Mac change, so the Mac session's to make, and it would answer the
  directory on both at once.
- A card can say `ssh` but never which host: Toolhelp gives an exe's file
  name, not a command line. `ssh_destination` is unreachable on Windows.
- End-to-end backpressure (`HIGH_WATER`) has no Windows test, for the ConPTY
  reason below. The credit machinery itself is tested directly.

## The goal, and what it is not

Claude Code running in terminal cards on the infinite canvas, on Windows, with the editor, diff and transcript cards, the palette, workspaces, groups and agent colours from hooks. A lesser app than the Mac one, on purpose:

- **No daemon.** On macOS every card's shell lives in its own `iftd` process so it outlives the app. On Windows the terminal backend is `pty`: the card owns its shell directly through `portable-pty`, which uses ConPTY. Quitting, restarting or updating the app ends every shell; a card comes back as a fresh shell in its saved directory, and Claude sessions come back with `claude --resume`.
- Why not port `iftd`: ConPTY is itself a terminal emulator that re-renders the program's output before we see it. `iftd` exists so that nothing stands between Claude and our own parser and a reattach can replay the exact bytes; ConPTY is precisely the second emulator that design routes around. It needs a design of its own, later, if ever.
- No `ift attach`, no `ift sessions`, no scrollback on disk across a power cut.
- Browser cards came last (CEF on Windows was a phase of its own), then packaging and the updater. All three are done; Authenticode is the one piece deliberately skipped, see the hash note above.

## Ground rules for this session

- **macOS must keep building and working.** Every Windows change goes behind `#[cfg(windows)]` / `#[cfg(unix)]` (or `target_os = "macos"` where the thing is Mac-specific rather than Unix-specific). Never delete a Mac path to make Windows compile.
- **Work on the `windows` branch**, push it, never force-push master, never merge into master yourself. The Mac session merges after `make check` passes on macOS, because only the Mac can check the Mac. Rebase `windows` onto master often; two other sessions (the Mac port session and `ift-browser`) commit to master daily.
- Commit messages say why, no attribution trailers of any kind, stage files by name. Every new file starts with a header block (responsibility, where it fits, callers, constraints). Tests always: logic is a pure function in its own file with tests, the gpui side is wiring.
- `infiniterm-browser/`, `browser_body.rs`, `browsers.rs` and `omnibox.rs` belong to the `ift-browser` session. Phase 5 touched `surface.rs` and `process.rs` there, at Ekin's word; anything further is coordinated with that session through him.
- **Verify on screen with `tools/drive/win/`, not by compiling.** Every bug phase 5 found was invisible to the compiler and the tests: a page hearing the wrong key, the driver itself doubling characters. A screenshot is the evidence.

## Setting up the box

The remote shell over `ssh win-gpu-box` is PowerShell 7; use pwsh syntax and quote Windows paths. For GUI work the session must run on the desktop itself (or over Remote Desktop), not through ssh.

1. Rust via rustup with the MSVC toolchain, and the Visual Studio Build Tools "Desktop development with C++" workload (gpui and tree-sitter compile C and C++).
2. `gh auth login`, then `gh repo clone ekinertac/infiniterm` (the repo is private).
3. `.cargo/config.toml` sets `CEF_PATH` to a macOS path with `force = false`, so an environment variable wins. On the box set `$env:CEF_PATH = "$env:LOCALAPPDATA\cef"` and build once: the `cef` crate downloads the Windows distribution under it and its build script copies libcef.dll, the paks and the rest into `target/debug`. Without that variable the build picks up the macOS path and fails; with `--no-default-features` no CEF is needed at all.
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

**All six are done as of 2026-09-25.** This is the plan as it was written on
the Mac, kept because the "Done:" lines say what each phase was actually
being judged on. What happened instead of the plan is under "Where this
stands" above; where the two disagree, that section is the record.

1. **The core crates build and pass their tests on Windows.** `infiniterm-core`, `infiniterm-term`, `infiniterm-editor`: the daemon and tmux backends gated unix, the socket behind the transport module, `/tmp` in tests replaced. A `browser` cargo feature on `infiniterm-ui` (default on, so macOS is unchanged) that compiles `infiniterm-browser` and CEF out and draws a stub card ("browser cards are not in this build") in their place; this is also the build-time Lite/Pro split planned in ekinertac/notes#50, so build it the way that plan says (the stub card, no runtime key) and tell Ekin, because the `ift-browser` session owns that crate. Done: `cargo test` green for the three crates on Windows, and `make check` still green on the Mac (the Mac session runs it when merging).
2. **A window opens and a terminal card runs pwsh.** `cargo build -p infiniterm-ui --no-default-features` (browser off), then run it. Keys typed reach the shell, Enter runs a command, output draws, a resize reflows. Done: Ekin types `dir` in a card and sees the listing.
3. **The canvas and the app keys.** The modifier decision from "Keys" above, then pan, zoom, new card, focus movement, the palette, workspaces. Done: Ekin can do a morning's work on the canvas without reaching for a Mac-only chord.
4. **Claude in a card with hooks.** The named-pipe transport, the hook binary on Windows, `ift install-claude-hooks` writing the Windows path of `infiniterm-hook.exe` into `%USERPROFILE%\.claude\settings.json`. Check that Claude Code's Shift+Enter still makes a line break through ConPTY (the kitty keyboard protocol; recent ConPTY passes it through, older ones swallow it). Done: a Claude card turns working, waiting and done colours as it runs.
5. **Browser cards.** CEF for Windows through cef-rs (its README has the PowerShell steps), with the `ift-browser` session. Done: a browser card loads a page.
6. **Shipping.** A zip or MSI, Authenticode signing (a paid certificate; SmartScreen warns until it has reputation), and the updater's Windows half (the manifest and "newer is a higher build" logic in `update.rs` carry over; the signature check becomes WinVerifyTrust, and the swap has to wait for the exe to unlock). Ekin decides when; not before phases 1 to 4 are real. **Shipped as a zip, and Authenticode is out**: Ekin decided on 2026-09-25 that the SHA-256 in the manifest is the only check, so there is no WinVerifyTrust. See the hash note above for what that trades away.

## Things that will bite

- ConPTY rewrites what the program writes, redraws on resize, and older builds of it swallow escape sequences newer programs rely on. When something renders wrong, check whether Windows Terminal shows the same program correctly before blaming our parser.
- **ConPTY writes NOTHING until the terminal answers its cursor probe.** It
  opens with `ESC [ ? 9001 h`, `ESC [ ? 1004 h` and then `ESC [ 6 n`, and
  blocks there. The app is fine (alacritty answers, and
  `Panes::we_are_the_terminal` is true for this backend), but any test that
  only reads sees the probe and then silence forever. `local_pty.rs`'s
  `answer_conpty_probe` is what that cost.
- **ConPTY caps a pane's output rate at roughly its own render rate.**
  Measured 2026-09-23: about 9 KiB a second for a program writing flat out,
  against a pty that does megabytes. It renders the pane and emits what
  changed, so a `yes`-shaped flood mostly never reaches us. Nothing is wrong
  when a flood looks slow; the 256 KiB backpressure mark is half a minute
  away at that rate, which is why the flood test stayed unix.
- **PSReadLine redraws a typed line a character at a time**, with cursor
  moves between, so a string typed at an interactive prompt never comes back
  as one run of bytes. A test that matches output from an interactive shell
  has to ask a one-shot command instead.
- **A shell is not listening the moment its pty exists.** PowerShell runs the
  profile first, which took the best part of twenty seconds under a loaded
  test suite here. A line typed before then is gone, not queued.
- **`ChildKiller::kill` does not kill a ConPTY tree.** It ends the process
  portable-pty spawned and nothing else: the shell sits beside an
  OpenConsole of its own and its own children are untouched. Lifting a
  card's mask left the decoy running after the app had quit, and a
  force-killed app left pwsh and OpenConsole behind. Each pane holds a job
  object with KILL_ON_JOB_CLOSE now, which covers the kill and the crash.
- **A card's command runs with the user's PROFILE loaded**, deliberately,
  and a profile may redefine anything. `Sort` on this box is a function that
  runs Git's sort.exe, so the decoy default showed "Input file specified two
  times" instead of a process list. Anything the app puts in a card's
  command line wants full cmdlet names, never aliases.
- **Claude Code runs a hook through `/usr/bin/bash` on Windows**, where a
  backslash is an escape character, so an absolute path written into
  settings.json arrives as `C:UsersPCCode...` and the hook is never found.
  `ift install-claude-hooks` writes forward slashes there. The only sign is
  a non-blocking notice inside Claude's own startup output, so it is easy to
  install a setup that looks fine and never lights a border.
- **`std::fs::canonicalize` returns the VERBATIM form**, `\?\C:\...`, which
  turns off path parsing: right for an API call, wrong for a card's cwd, a
  shell command line and anything a person reads. `paths::canonical` is the
  one that strips it; three call sites had leaked it before that existed.
- **A force-killed instance's endpoint answers for a couple of seconds**
  while Windows tears its handles down, so a relaunch inside that window is
  told another infiniterm holds it and exits. Asking the pipe which process
  serves it (`GetNamedPipeServerProcessId`) and whether that process lives
  looks like the fix and is not: while a dying instance's handle lingers
  beside a live one's, a client lands on either, and landing on the dead one
  lets a SECOND instance start on the same save file. It was tried, it did
  exactly that, and it was withdrawn. `tools/drive/win/run.ps1` waits the
  window out instead.
- **gpui names a Windows key through the current layout** and hands back the
  SHIFTED character with shift cleared for punctuation and digits, which is
  the macOS trap word for word. `keycode.rs`'s Windows half takes the SCAN
  CODE from a thread-local `WH_KEYBOARD` hook instead. Thread-local and not
  `WH_KEYBOARD_LL` on purpose: the low-level hook is global and would have
  this process watching every key typed in every other application.
- **tree-sitter-scss 1.0.0 cannot build on MSVC**: its build script hands cc
  an unconditional `-Wno-unused-parameter` and cl refuses it (D8021). It is
  the crate's only release. It is `cfg(not(windows))` now.
- Line endings: set `core.autocrlf false` on the clone, or `.gitattributes` fights begin; the Mac side commits LF.
- A path in a card label, a drop, or `ift <path>` has backslashes and a drive letter; `shell_quote` (`drop.rs`) is POSIX quoting and must get a PowerShell twin before a dropped file reaches a pwsh prompt, because that function is the whole security story for a file named `; rm -rf ~`.
- The single-instance lock is the socket today; on Windows it is the named pipe, and a crashed instance leaves no stale file to clean (unlike the Unix socket), which is simpler.
- Report back in plain text what you changed, what you checked and what you could not, and ask Ekin to relay anything that needs the Mac session.
