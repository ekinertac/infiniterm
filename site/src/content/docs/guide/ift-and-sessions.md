---
title: ift and sessions
description: Drive the canvas from a shell, and keep shells running when the app quits.
---

`ift` is the app's command-line side. Run from inside a card, it acts on the canvas; run from any terminal, it reaches the running app over its socket.

## Opening things

```sh
ift                    # launch infiniterm, or focus it if running
ift ~/Code/project     # an editor card with a file tree rooted there
ift src/main.rs        # edit the file over this terminal card, like vim
ift -n src/main.rs     # the file in a card of its own, returning at once
ift src/main.rs:42     # ...at line 42 (file:42:7 for a column)
ift diff               # changes against git HEAD, as a card
ift diff src/          # only under a path
```

Run inside a terminal card, `ift <file>` lays an editor over that card and waits until you close it, so `EDITOR=ift` works for `git commit`. A file that does not exist yet opens empty and the first save creates it, so any word that is not one of `ift`'s commands is taken as a file name. See [Editor cards](../editor/).

A path beats a verb: a directory called `ls` in front of you opens the directory.

## Naming and grouping the card you are in

```sh
ift name "api server"
ift group backend      # creates the group if needed
```

## Listing

```sh
ift ls                 # cards: id, group, directory, state, remote, number
ift commands           # every command: id, label, key
ift usage 30           # which commands and gestures you used in 30 days, and which you never did
ift omni rust          # what the address bar would show for "rust"
ift --version          # the version of this ift
```

On a terminal these print a padded table. Into a pipe they print tab-separated rows without a header, so `ift ls | cut -f1` works.

`ift ls --agents` lists only the cards that run an agent, one tab-separated line each: card number, agent, session id, state, directory. The session id is the one `claude --resume` takes. For a Cursor card the lost-session resume line is `cursor agent --resume <id>`.

Exit codes: 0 ok, 1 infiniterm is not running, 2 bad usage.

## Driving another card

These take a card by its number (`7` or `#7`, the one on its label, which stays the same after a reboot) or by its id.

```sh
ift send 7 "npm test" --enter     # type into card 7's shell and press Enter
ift send 7 --key ctrl-c           # just a key
ift read 7                        # what card 7's terminal shows now
ift read 7 --lines 20             # only the last 20 lines
ift read 7 --all                  # the scrollback too
ift close 7                       # close it, like Cmd+W
ift run canvas.tidy               # run any command from `ift commands`
```

`ift send` types into the card without moving the focus or the view. The text goes first, then the keys in the order you give them. `--enter` presses Enter; `--key` takes `enter`, `esc`, `tab`, `backspace`, `ctrl-c`, `ctrl-d`, `ctrl-l`, `ctrl-z`, `up`, `down`, `left` or `right`, and can be given more than once.

`ift read` prints the live screen, also when the window is hidden. `ift close` follows the same rules as `Cmd W`, so a closed terminal is parked first; a protected card (`Cmd Shift L`) refuses and says how to unprotect it. `ift run` runs a command on the card in focus, as the palette would, and refuses an id it does not know.

Together they let a script watch one card and answer in it. [tools/restart-claude.sh](https://github.com/ekinertac/infiniterm/blob/master/tools/restart-claude.sh) is one: it finds every Claude card with `ift ls --agents`, sends `/exit`, reads the resume command Claude prints, and types `claude --resume` back into the same card. It is an example, not a feature: it clears a prompt line you had not sent, and it does not keep a `CLAUDE_CONFIG_DIR` other than the default.

## Sessions that outlive the window

Each terminal card's shell runs under its own small daemon, `iftd`, which holds the pty. Quitting the app leaves your shells running; reopening it replays what they printed into the same emulator, byte for byte, so a Claude Code session comes back as it was. This is the default backend (`terminal.backend: "daemon"`).

A daemon keeps the last 4 MiB of output (`terminal.sessionBuffer`) and writes it to disk every two seconds while output arrives. After a power cut or a reboot, the card replays that above a fresh shell with a `[session lost]` line, and a card that ran Claude has `claude --resume <id>` in its history: Up, Enter.

Nothing keeps a process alive across a reboot.

## Reaching a shell from elsewhere

```sh
ift sessions           # card number and label for each running session
ift sessions --full    # every column: id, pid, cwd, command, started, card, label
ift attach 7           # this terminal becomes card #7's shell
```

`ift attach` takes the card's number (the `#7` on its label), which survives a reboot; the session id does not. It works whether the app is running or not, so you can reach a card over ssh from your phone.

A session has one client at a time. Attaching takes it from the app: the card shows a banner and ignores keys until you detach with `Ctrl \`, then the app takes it back within a second.

## Servers

```sh
ift connect me@server             # a second infiniterm for that server
ift connect me@server --check     # only test the host
ift connect me@server --install   # first put ift and iftd on a Linux host
ift connect me@server --name prod --color 3b82f6
```

`ift connect` opens a second infiniterm with its own window, Dock icon and canvas. Its terminal cards run on the server over ssh, in the server's home folder. The shells live in `iftd` on the server, so closing the window leaves them running, and connecting again brings the same cards back. Each server has its own colour (a tinted title bar, a chip with its name, a badge on the Dock icon), its own settings and no browser cards. A new server starts with the default settings, not a copy of yours. A server you connected with an earlier version keeps the copy it got then; delete `~/.infiniterm/remotes/<host>/config` to start it from the defaults.

It needs key login (ssh runs without a password prompt) and `ift` and `iftd` on the server. `--install` puts them there on Linux, x86_64 or aarch64: this Mac downloads the package for its own version from the GitHub release, checks its checksum and sends it through ssh, so the server needs no internet. Root gets `/usr/local/bin`, any other user `~/.local/bin`. `--from <package>` installs from a file instead. A server that is a Mac uses the app already installed there. Without `--install`, `ift connect` says what is missing, and it tells you when the server's `ift` is a different version from yours.

Agent states and `ift` work inside a server's cards too. One extra ssh connection carries them: a Claude or Codex session on the server shows working, waiting and done on its card, and `ift ls`, `ift send` and the rest reach the window from a server shell. Run `ift install-claude-hooks` (or `install-codex-hooks`) on the server once so the agent there calls the hook.

Not supported: editor and diff cards on the server's files, and Windows servers.

## tmux

`terminal.backend: "tmux"` puts each card in a tmux window in a session named `infiniterm` instead, reachable with `tmux attach -t infiniterm`. It is not the default: tmux is a second terminal emulator in the path and it corrupted Claude Code's redraws in ways the daemon does not. `"pty"` gives plain shells that end when the app quits.
