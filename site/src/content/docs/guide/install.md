---
title: Install and first launch
description: Download the app, put ift on your PATH, and wire up agent state.
---

infiniterm runs on Apple Silicon Macs with macOS 13 or later. Builds are signed with a Developer ID and notarized by Apple, so Gatekeeper opens them without a warning.

## Install

1. Download the `.dmg` from the [latest release](https://github.com/ekinertac/infiniterm-releases/releases/latest).
2. Open it and drag infiniterm into Applications.
3. Launch it once from Applications.
4. In any card, put the `ift` command on your PATH:

   ```sh
   /Applications/infiniterm.app/Contents/MacOS/ift install
   ```

   This symlinks `ift` into `~/.local/bin` and writes its zsh completion. If either directory is not on your `PATH` or `fpath`, it prints the line to add.

## Agent state

Card borders show what an agent in the card is doing. Claude Code and Pi report through hooks, installed once:

```sh
ift install-claude-hooks      # edits ~/.claude/settings.json
ift install-pi-hooks          # installs the Pi extension into ~/.pi/agent
```

`ift install-pi-hooks DIR` takes a different agent directory, for a wrapper that runs Pi against its own. Both accept `--dry-run` to show what they would change.

Other commands need no setup: a zsh started in a card reports each command's start and exit status to the app. See [Card states](../card-states/).

## Shell history per card

Each card gets its own zsh history file, so a new card does not open with every other card's past, and after a reboot a card that ran Claude has `claude --resume <id>` as its last history entry. macOS sets `HISTFILE` before your `.zshrc` runs, so add one line to `.zshrc`:

```zsh
HISTFILE="${INFINITERM_HISTFILE:-$HOME/.zsh_history}"
```

Outside infiniterm the variable is unset and your usual history file is used.

## Updates

The copy in Applications checks the releases repo at launch and every six hours. A newer build is downloaded, its signature and notarization are checked, and it is installed the next time the app restarts. The status bar says "update N ready" until then. "Check for Updates…" in the app menu checks now.

The check is a plain GET of `latest.json` with nothing about you in it.

## Your first minute

- `Cmd T` opens a terminal card. `Cmd W` closes it.
- `Cmd Shift P` is the command palette: everything the app does is in it.
- `Cmd /` lists every shortcut, searchable.
- `Cmd` + scroll zooms, `Cmd` + drag pans. Bare scroll and drag belong to the card under the pointer.
- `Cmd 2` fits every card in the window, `Cmd 1` fits the focused one.
