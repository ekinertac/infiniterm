---
title: Servers
description: Connect to a server with ift connect, install ift on it, and see its agents on a canvas of its own.
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

- Root gets `/usr/local/bin`. Any other user gets `~/.local/bin`.
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

## Not built yet

Editor and diff cards on the server's files, so `ift file` and `EDITOR=ift` work in a server window. Windows servers.
