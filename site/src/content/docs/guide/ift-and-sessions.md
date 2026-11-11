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
ift -t src/main.rs     # the file as a tab of an editor card
ift -t src/main.rs 2   # ...of card 2 (a number like 2 or #2, or an id)
ift src/main.rs:42     # ...at line 42 (file:42:7 for a column)
ift terminal ~/a ~/b   # a terminal card per folder, its shell starting there
ift diff               # changes against git HEAD, as a card (see Editor cards)
ift diff src/          # only under a path
```

Run inside a terminal card, `ift <file>` lays an editor over that card and waits until you close it, so `EDITOR=ift` works for `git commit`. A file that does not exist yet opens empty and the first save creates it, so any word that is not one of `ift`'s commands is taken as a file name. See [Editor cards](../editor/).

`ift terminal <folder>...` makes a terminal card for each folder on the workspace you are looking at and focuses the last one. It starts infiniterm first when it is not running.

`ift terminal --workspace <id|name|new> <folder>...` opens the cards on another workspace and shows it. Give the workspace's id or its exact name. `new` makes a workspace first, and `--name <text>` names it. Without `--workspace`, nothing changes. `ift workspaces` lists the workspaces from left to right: id, name, number of cards, and `*` on the one on screen. It prints a table on a terminal and tab-separated rows without a header into a pipe.

`ift -t <path> [card]` adds the file as a tab of an editor card. Without a card it uses the focused editor card, and opens a card of its own when the focus is not on one. A file that is already open is switched to, not opened twice. A terminal, an unknown card and a directory are refused, and the message says why.

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
ift browse index.html  # open a local page, or a web address, in a browser card
ift update status      # the app's update state on one line: off, checking, current, ready, failed
ift push list          # the phones that get pushes (also push add, push remove)
ift omni rust          # what the address bar would show for "rust"
ift --version          # the version of this ift
```

On a terminal these print a padded table. Into a pipe they print tab-separated rows without a header, so `ift ls | cut -f1` works.

`ift ls --agents` lists only the cards that run an agent, one tab-separated line each: card number, agent, session id, state, directory. The session id is the one `claude --resume` takes. For a Cursor card the lost-session resume line is `cursor agent --resume <id>`, and for an Antigravity card `agy --conversation <id>`.

Exit codes: 0 ok, 1 infiniterm is not running, 2 bad usage.

`ift browse` opens a browser card for each argument, beside the card you ran it in (or beside the focused card), and returns at once. An address with a scheme (`http://localhost:5173`, `https://…`, `file://…`) goes to the browser as it is. A path that exists, relative to the shell's folder or starting with `~`, opens as a `file://` address, and spaces are fine. A path that is not there is refused with a message: a missing page is almost always a typo. A joined window or a remote instance has no browser cards, and says so.

## Pairing the iPhone app

```sh
ift phone add iphone   # a QR code for the infiniterm iPhone app to scan
ift phone list         # the paired phones
ift phone remove iphone
```

The iPhone app reaches this Mac through the Mac's own ssh, over Tailscale. `ift phone add` needs Remote Login on (System Settings, General, Sharing). It makes an ed25519 key for the phone, writes the public half to `~/.ssh/authorized_keys` with the comment `infiniterm-phone:<name>`, and prints a QR code with the phone's private key, the Mac's ssh host key and its address. The app scans it once and needs nothing else. `--host <name>` changes the address in the code from the Mac's Tailscale name, and `--port <n>` the ssh port.

If Tailscale SSH is on (`tailscale set --ssh`), Tailscale answers on port 22 of the tailnet address, ignores `authorized_keys` and asks for a login the app cannot show, so a phone cannot use it. `ift phone add` then runs `tailscale serve --bg --tcp=2222 tcp://localhost:22` for you, which forwards tailnet port 2222 (or the next free port) to the Mac's own ssh, and puts that port in the code. It needs you to be the Tailscale operator; if it says no, run `tailscale set --operator=$USER` once. The Mac's ssh may also allow password login, and from your tailnet that login is then reachable on that port too: the command says so. `ift phone remove` of the last phone closes the port again, or run `tailscale serve --tcp=2222 off`. With `--port <n>` nothing is forwarded.

The key is limited. It opens no shell: sshd runs `ift phone-gate` for it, which allows only the commands the app sends (joining the canvas, reading a card, the update check, the push list) and refuses all others. A phone may also open tunnels to this Mac's own localhost, for its browser card. Anyone who sees the QR code can use the key, and the terminal card keeps the printed code in its session buffer. If anyone saw it, run `ift phone remove <name>`, which deletes that one line and leaves the rest of `authorized_keys` as it was. A device name is 1 to 32 letters, digits, `-` or `_`.

## Driving another card

These take a card by its number (`7` or `#7`, the one on its label, which stays the same after a reboot) or by its id.

```sh
ift send 7 "npm test" --enter     # type into card 7's shell and press Enter
ift send 7 --key ctrl-c           # just a key
ift read 7                        # what card 7's terminal shows now
ift read 7 --lines 20             # only the last 20 lines
ift read 7 --all                  # the scrollback too
ift close 7                       # close it, like Cmd+W
ift focus 7                       # go to it: show its workspace, focus it, fit it
ift run canvas.tidy               # run any command from `ift commands`
```

`ift focus` moves the view to the card, as picking it in the palette does.

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

## Raycast

Tarık Kavaz made a Raycast extension that drives infiniterm through `ift`. Install it from the [Raycast store](https://www.raycast.com/tarikkavaz/infiniterm). It is not part of infiniterm, and it needs infiniterm 0.6.4 or newer. Its commands:

- **Open in Infiniterm** opens the Finder selection, or the folder of the front Finder window, in a workspace you pick or a new one. A second command does the same on the workspace on screen.
- **New Terminal Card** opens a terminal card in a folder you choose.
- **Open File in Infiniterm** opens the selected Finder file as an editor card, and **Open Git Diff in Infiniterm** shows the git changes of the selection as a diff card.
- **Open Infiniterm Settings File** and **Open Infiniterm Keybindings File** open `settings.json` and `keybindings.json` as editor cards.
- **Send Text to Card** types text into a card you choose without moving the focus. **Run Saved Command in Card** keeps the commands you send often.
- **Go to Card** picks a card from any workspace and brings it on screen.

The extension runs the `ift` named in its "ift Path" preference. The default is the copy inside the app, `/Applications/infiniterm.app/Contents/MacOS/ift`. The path must be absolute, because `~` is not expanded. On an older infiniterm, some commands say they need a newer `ift`.

## Servers

`ift connect me@server` opens a second window whose terminal cards run on that server. Installing `ift` there, colours, agents and troubleshooting have a page of their own: [Servers](../servers/). `ift join <host>` opens a window on another Mac's running canvas instead, described there too.

## tmux

`terminal.backend: "tmux"` puts each card in a tmux window in a session named `infiniterm` instead, reachable with `tmux attach -t infiniterm`. It is not the default: tmux is a second terminal emulator in the path and it corrupted Claude Code's redraws in ways the daemon does not. `"pty"` gives plain shells that end when the app quits.
