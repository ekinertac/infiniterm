---
title: Servers
description: Connect to a server with ift connect, install ift on it, and join another Mac's canvas with ift join.
---

`ift connect` opens a second infiniterm for a server. It has its own window, its own Dock icon and its own canvas. Its terminal cards run on the server over ssh. Nothing from your own canvas moves there.

## What you need

- A Mac with infiniterm, and `ssh` login to the server **with a key**. ssh must not ask for a password. A host name from your `~/.ssh/config` works as well as `user@host`.
- `ift` and `iftd` on the server. `iftd` is the small daemon that keeps each shell running. A Linux server (x86_64 or aarch64) gets both from `--install`. A server that is a Mac uses the app already installed there.
- Windows servers are not supported.

## Connect for the first time

1. Test the host. Nothing is changed.

   ```sh
   ift connect me@server --check
   ```

2. If it says `ift` or `iftd` is missing on a Linux server, install them:

   ```sh
   ift connect me@server --install
   ```

3. Connect. The first window opens with one terminal card in the server's home folder.

   ```sh
   ift connect me@server
   ```

Next time, run step 3 again. It brings the same cards back.

## What `--install` does

This Mac downloads the package for its own version from the GitHub release and checks its checksum on this Mac. It sends the package to the server through ssh. The server needs no internet, no `curl` and no Rust.

- Root gets `/usr/local/bin`. Any other user gets `~/.local/bin`. `ift connect` looks in both places, so it finds `ift` even when `~/.local/bin` is not on the PATH of a non-interactive ssh login.
- `--platform linux-x86_64` or `--platform linux-aarch64` skips asking the host which one it is.
- `--from <package>` installs from a file you already have instead of downloading.
- If the server's `ift` has a different version from your app, `ift connect` tells you. Run `--install` again to update it.

## Closing the window and coming back

The shells live in `iftd` on the server, one per card. Closing the window, quitting the app or losing the network leaves them running. `ift connect me@server` brings back the same cards, with their scrollback.

From any other shell, `ift sessions` on the server lists the sessions, and `ift attach 7` takes over card number 7. See [ift and sessions](../ift-and-sessions/#sessions-that-outlive-the-window).

## One window per server

Each server has its own look and its own files, so you can tell the windows apart at a glance.

- **Colour and name.** The title bar leans toward a colour, a chip shows the server's name, and the Dock icon has a badge. The colour comes from the host name, the same every time. `--color 3b82f6` and `--name prod` set them. "Window: change the title bar and Dock colour" in the palette changes the colour later.
- **Settings.** A new server starts with the default settings, not a copy of yours. It keeps its own from then on. To reset one, delete `~/.infiniterm/remotes/<host>/config`.
- **Files.** The canvas and settings of a server are saved under `~/.infiniterm/remotes/<host>/`. A long host name is shortened there, because a socket path has a length limit.
- **No browser cards.** A browser would run on your Mac, not on the server, so the window refuses them.

## Agents on the server

Claude Code, Codex and the other agents show their state on the card in a server window, as on your own canvas. Run the install command for the agent **on the server**, once:

```sh
ift install-claude-hooks
ift install-codex-hooks
```

One extra ssh connection carries the hook messages back to the window. It also lets `ift ls`, `ift send` and the rest reach the window from a shell on the server. The server side of that connection is a socket at `/tmp/infiniterm.sock`. See [Install](../install/#agent-hooks) for the other agents.

## When it does not connect

| `ift connect` says | Do this |
| --- | --- |
| `ift is not installed on the server` | Run `ift connect me@server --install`. |
| `iftd is not installed on the server` | Run `ift connect me@server --install`. |
| `ssh could not log in without a prompt` | Set up key login for that host, and test with `ssh me@server true`. |
| the server's `ift` is another version | Run `ift connect me@server --install` to match your app. |

ssh runs in batch mode, so it never stops to ask for a password or a question.

## Joining a canvas that already exists

`ift connect` makes a new canvas that runs on a server. `ift join` is the other way round: it opens a window on the canvas of **another Mac's infiniterm** and gives you full control of it, as if you sat at that Mac.

```sh
ift join me@mac-mini            # the host: an ssh alias, user@IP or an IP
ift join me@mac-mini --check    # only test whether the host can be joined
```

You see the host's cards, labels and agent colours, and a live terminal for each terminal card. What you do happens on the host: typing goes to the host's shell, closing a card closes it there, `Cmd T` starts a shell on the host, and moving, resizing, grouping, renaming and workspaces change the host's canvas. Your zoom, position and focus stay your own. Browser cards show as a card that says what it is and where, and they move and close like any card. Editor, file tree, diff, transcript and page cards work on the host's files, as the next list says. If you and the host change the canvas at the same moment, both changes stay; your change wins only when you both change the same thing on the same card.

- **Login.** Key login works without a prompt. The first time, ssh asks its usual questions in the terminal you ran `ift join` from: the host's fingerprint and, if there is no key, the password. Run from a launcher or a script, it shows the same questions as macOS dialogs. A host that only takes a password is asked once, and the window's connections share that one login. A host key that has changed is an error with the new fingerprint, never a question.
- **The host's app can be closed.** `ift join` then starts `ift serve` on the host and tells you so. `ift serve` holds that Mac's saved canvas and keeps its cards running for viewers. Plain `ift serve` runs in the foreground until `Ctrl C`. `ift serve --detach` runs it in the background and returns once it answers; its output goes to `serve.log` in the data folder. `ift serve --stop` ends it, waiting up to 15 seconds, and the cards keep running. `ift join` runs `--detach` for you on a host with no app. When you start infiniterm on that Mac, it stops `ift serve` itself (no flag, no command), loads the canvas serve just saved and takes over, with the same cards and shells still running. Whoever is joined reconnects by itself within seconds. If `ift serve` does not stop within 10 seconds, the app shows a dialog naming the process and exits. A host whose `ift` is too old to have `serve` is told to update.
- **Linux hosts.** A Linux server that runs infiniterm's sessions but never had the app shows one card for every shell already running. You can type in them, add cards (`Cmd T` makes a shell) and close them.
- **Files.** Every file you open in a joined window is a file on the host. An editor card shows the host's file. You can edit and save it: the save goes to the host. If the file changed on the host after you opened it, the host refuses the save and your text stays. Save again to overwrite on purpose. The card follows changes on the host and keeps its text if the host cannot be reached. A file it could not read stays locked, so it cannot be saved over. A file deleted on the host says so once and keeps its text; saving writes it again.
- **File tree.** The editor's tree lists the host's folders, for example after `ift -n .` or `Cmd K` on a host card. It says "Loading from the host..." until the list arrives, and shows the host's error as text if a folder cannot be read. The tree and the open file follow changes on the host within about half a second.
- **Git marks.** An editor card shows the added and changed lines in its gutter, from the host's git. A commit on the host shows in the marks within about 10 seconds. A file outside a repository has no marks.
- **Diff and transcript cards.** A diff card lists the files that changed in the host's git, shows both sides of a file, and supports blame and `Cmd K`. It follows the file as it changes on the host. A git error from the host is shown as text in the card. A transcript card reads the agent's session file on the host and follows it as the session grows. It shows only what the host has. Page cards (Markdown) show the host's page, and a link in it opens on the host.
- **Pictures and backgrounds.** A picture file in an editor card is fetched from the host and drawn, and follows the file if it changes there. In the host's canvas background list, a bundled name is the app's own picture. Any other entry, such as `~/Pictures/x.jpg`, is a file on the host. The copies are kept in the `host-pictures` folder of the app's data folder.
- **Paths.** `Cmd` + click on a path in a joined terminal looks for the path on the host. The host opens an editor or folder card beside the terminal, and your window moves to it and focuses it. A path that is not on the host gives a notice. `ift ./file` typed in a joined terminal shows its editor in your window, and `Cmd W` closes it there and on the host.
- **Drops and pastes.** A file dropped on a joined terminal, or an image pasted with `Cmd V` or `Ctrl V`, goes to the host first. It lands in `~/.infiniterm/drops/<id>/` on the host (an image as `clipboard.png`), and the card is given that path. `Ctrl V` with text on your clipboard, or with no image, still goes to the program as a key. A folder, a file over 64 MB, or a host with an older `ift` uses ssh for the copy instead. The host removes drop folders older than a week at the next send.
- **Settings.** The window uses the host's settings and keybindings: font, theme, keys. A change in the host's file reaches the open window within a second. A host file with an error is not applied. Your host colour, chip and window frame stay your own. "App: open settings" and "App: open keybindings" open the host's files, beside the host's defaults, as editor cards that save to the host. The first time, the window asks the host where its settings folder is. A save is applied on the host and reaches every window joined to it. Changing a setting any other way from the joined window is refused with a notice: change it on the host.
- **Lost connections.** A card whose connection ends keeps its last screen and shows a short red line, "Lost the connection to the host; joining again", with the cause in a smaller line under it. Both lines are cut at the card's edge. The card then joins its session again by itself, waiting 2 seconds and then longer, up to 30. A key typed into it meanwhile is refused with a notice. `Cmd R` makes a card join its session again at once. If the host's app quits or restarts, the window waits and joins again.
- **Restarting the window.** "App: restart", and the restart that installs an update, bring the joined window back as the same viewer of the same host. If a window for the host is already open, `ift join` says so, with its process number, and brings that window to the front.
- **Closing cards.** A close in a joined window closes only the cards you closed in that window, on the host, the normal way. `Cmd Shift T` brings them back, and a shell is kept alive for a while. Cards that are only missing from your copy of the canvas stay open on the host. If the host app was restarted since your last change, that change is dropped and a notice says "the host was restarted; your last change was not applied".
- **Versions.** Use the same build, or a newer one, on both Macs. Update the window and the host together. A window from before the close change cannot close a card on the host. An older `ift` on the host shows no git marks, polls files every 2 seconds, and sends drops over ssh. A browser card from the host stays a placeholder: its page does not load in your window.
- **Switching it off.** The host's join socket is owner-only. The setting `join.enabled` is on by default; set it to `false` and the app closes the join socket, so nothing can join. It applies to the app: `ift serve` runs only when someone asked to join. Nothing is shown or sent unless something joins.

## Not built yet

For `ift connect`: editor and diff cards on the server's files, so `ift file` and `EDITOR=ift` work in a server window. Windows servers.
