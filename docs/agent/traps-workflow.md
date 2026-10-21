# Traps in the workflow

What this covers: mistakes already made while working in this repo, kept so they are not made again: running and testing the app, driving it with the GUI driver, building and signing the bundle, cargo and git, shell quoting, and the macOS privacy and quarantine rules.

Read it before running the app, writing a driver scenario, building a bundle, committing, merging, or measuring performance.

Related: `traps-code.md` (traps in the code itself), `repo-map.md` (where the scripts are), CLAUDE.md "Working here" (the rules these traps taught).

Moved here from CLAUDE.md's "Traps already hit" on 2026-10-19 with the text unchanged; the headings are new.

## Running and testing the app

- A binary started from a shell is not activated and gets no key events, and has no framework beside it. Bundle and `open`.
- A test must never bind `/tmp/infiniterm.sock`: the installed app holds it. `hooks::listen` takes the path; tests use a temp one.
- Two apps named infiniterm with the same bundle id: a driver that finds windows or processes by name typed into Ekin's real canvas once.
- Never `pkill -f <pattern>` to stop a scratch instance: the real app has the same binary name, and `pkill -f "MacOS/infiniterm$" -n` ended it (options go before the pattern on macOS, so `-n` was ignored; 2026-10-04). Find the pid from the instance's socket (`lsof -t`) and `kill` that. Same on a remote host: `iftd` by pid from `ps`.
- Measure idle CPU of a scratch instance with `top -l 3 -s 2 -pid <pid> -stats pid,cpu`, not `ps %cpu` (a decaying average that kept a start-up spike for minutes). A slow animated canvas background (three drifting gradient washes at 10 fps) was tried on 2026-10-05 and removed the same day (Ekin didn't like it); it cost about 3 to 4 points of CPU on a one-card canvas because every frame repaints every card, which is why nothing animates the canvas at rest.
- A scratch window behind another app is not redrawn, so `screencapture -l` returns its last drawn frame, not what its cards show now (`ift read` does show it). Bring the window to the front for a couple of seconds before a capture, with a go from Ekin. A shell that exits closes its card: Ctrl+D sent after an agent already quit ended two cards (`card.reopen` brought them back in their slots); quit agents with Ctrl+C or their own exit command only. `ift send` text with `--enter` reaches Claude Code as one paste, so the Enter becomes a newline in its prompt: send `--key enter` separately.

## Driving the app

- cliclick keys never arrive as gpui keystrokes. `screencapture -o` drops the window shadow, which offset every click by ~58 px.
- The palette matches "stress flood" to the calm command first; the driver types "run yes". Palette queries in scenarios must match a real label ("browser open a url", not "new browser"; "workspace: close", not "close workspace").
- System Events `keystroke "0" using {command down, shift down}` drops Shift to produce the character; use `key_code 29 "command down, shift down"` (and 24 for Equal, 27 for Minus) when a scenario needs a shifted chord. Chords are physical now, so scenarios press key codes, not characters.
- The MacBook Air is the small-screen test bed (`ssh air`; deploy by rsync of the bundle, screenshots with `screencapture -x` over ssh, keys by System Events key code). A claim about how the app looks there needs a screenshot from it, taken, not assumed.
- Driving an editor (2026-10-05, `tools/drive/multicursor.sh`, `complete.sh`, `editor-overflow.sh`): a card that is not locked swallows EVERY key (the first run typed nothing and Cmd+D fell through to `card.split.right`), a LOCKED one swallows the palette's keys too, so open the palette, the tree and `ift` first and click into the text last; a click lands on whatever line the card's size puts there, so walk with keys (Cmd+Up, Down, End) instead; a `.txt` or `.md` file wraps, so a line-too-long scenario needs a `.log`, and a `//` comment is drawn in the dim comment colour; `type_text '\"'` is how a quote reaches AppleScript; `ift <file>` opens a card from outside, and `drive_start` starts clean. BSD `sed -i` needs `-i ''`.

## Building, signing and privacy

- cef-rs's bundler sets `LSFileQuarantineEnabled`, so every file a card's shell wrote carried a `com.apple.quarantine` xattr naming infiniterm and Gatekeeper blocked builds made inside a card. `tools/bundle.sh` sets it false. Files already tagged need `xattr -dr com.apple.quarantine <dir>`.
- macOS keys a privacy grant (Screen Recording, Accessibility) to the app's code signature. cargo's ad-hoc, linker-only signature has no stable identity, so the toggle in System Settings flipped itself back off and `tccutil reset` found five dead rows per service, one per rebuild. `tools/sign.sh` signs every bundle with the Developer ID, inside out: the framework's dylibs on their own (a framework's signature does not cover `Libraries/`, and the notary refused each one), the framework, the helpers, the app. The driver's keystrokes and screenshots need those grants on whatever app HOSTS the session running it, not on the scratch instance.
- macOS shows a privacy prompt for a program in a card only if Info.plist has its usage key. `tools/bundle.sh` sets 29 (`usage_keys`, one plain sentence each, over cef-rs's bare "infiniterm" ones) and fails when one is missing; nothing is asked at install or launch (#170). The keys alone were not enough (2026-10-05): the bundle is signed with the hardened runtime, and without the matching entitlements in `tools/entitlements.plist` (camera, audio-input, bluetooth, photos-library, addressbook, calendars, location, apple-events) macOS denies the request silently, with no prompt. `osxphotos --use-photokit` from a card showed the prompt only after they were added; iftd was not the cause (same result under the pty backend). A new privacy usage key needs its entitlement too.

## Cargo, git and the shell

- `cargo fmt` reformats a file under an edit made from a stale copy, and a text patch then silently matches nothing. Re-read after fmt; never `cargo fmt --all` (it touches the cli and hook crates).
- Cargo fmt on the cli and hook crates is not just a preference: a subagent ran `cargo fmt -p infiniterm-cli` and 15 lines of the reference's text went into a commit about something else. The Makefile says never.
- `git checkout <file>` to undo one thing reverted an uncommitted change in the same file. Reverse edits by hand.
- A backtick in a double-quoted shell string is command substitution. A commit message written that way ran `open` and lost three phrases; use a heredoc.
- An edit script run twice (a retry after an unrelated assertion) wrote a test function twice; `make check` failed while `make install` and the push went ahead, because the release build runs no tests. `grep -c "test result: ok"` is not a pass: read `make check`'s exit status.
- An unquoted heredoc around an edit script runs every backticked word as a command; quote it (`<<'EOF'`). Run clippy before committing, and never chain `git commit` after a step that can fail.
- `make check` runs rustfmt and can rewrite a file AFTER you committed it: look at `git status` once it passes and amend the formatted file in, or the merged commit carries unformatted code (the find-bar and forward tests did).
- `gh pr merge` can say merged while the local `master` has not moved, and a push can hang when SSH to GitHub does not answer (ift-publishing pushed over HTTPS); wrap network git and gh calls in `timeout`, then `git fetch` and `git merge --ff-only origin/master`. A PR can also come back "not mergeable" because another session landed a `CHANGELOG.md` line under the same day heading: merge `origin/master` into the branch and keep every line from both sides.
- A fresh worktree has its own `target/`, so the first build takes minutes, and the `infiniterm-cli` install tests and the `iftd` daemon tests fail until `cargo build -p infiniterm-cli -p infiniterm-session -p infiniterm-hook` has run there. The worktree harness blocks most git commands in a session entered with `EnterWorktree` (only `--version` and `rev-parse` passed), so make worktrees by hand with `git worktree add .claude/worktrees/<issue>-<slug> -b <issue>-<slug> origin/master`.
