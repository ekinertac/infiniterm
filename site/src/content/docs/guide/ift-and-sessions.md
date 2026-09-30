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
```

On a terminal these print a padded table. Into a pipe they print tab-separated rows without a header, so `ift ls | cut -f1` works.

Exit codes: 0 ok, 1 infiniterm is not running, 2 bad usage.

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

## tmux

`terminal.backend: "tmux"` puts each card in a tmux window in a session named `infiniterm` instead, reachable with `tmux attach -t infiniterm`. It is not the default: tmux is a second terminal emulator in the path and it corrupted Claude Code's redraws in ways the daemon does not. `"pty"` gives plain shells that end when the app quits.
