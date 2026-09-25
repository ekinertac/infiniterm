---
title: Install and first launch
description: Download the app, put ift on your PATH, and wire up agent state.
---

infiniterm runs on Apple Silicon Macs with macOS 13 or later. It comes two ways: Pro, a signed app you buy, and Lite, which you build from the source yourself.

## Pro

Pro is the signed and notarized app, so Gatekeeper opens it without a warning, and it updates itself. It adds browser cards and carries the licence for using infiniterm at work.

1. Buy it; the receipt links the `.dmg`.
2. Open it and drag infiniterm into Applications, then launch it once.
3. In any card, put `ift` on your PATH:

   ```sh
   /Applications/infiniterm.app/Contents/MacOS/ift install
   ```

   This symlinks `ift` into `~/.local/bin` and writes its zsh completion. If either directory is not on your `PATH` or `fpath`, it prints the line to add.

## Lite

Lite is everything except browser cards, built from the public source, for personal use. You need Rust; the source's README has the steps. Builds are yours to use, not to hand to anyone else.

## Agent hooks

Card borders show what an agent in the card is doing. Claude Code and Pi report through hooks, installed once from any shell:

```sh
ift install-claude-hooks      # edits ~/.claude/settings.json
ift install-pi-hooks          # installs the Pi extension into ~/.pi/agent
```

`ift install-pi-hooks DIR` takes a different agent directory, for a wrapper that runs Pi against its own. Both accept `--dry-run`.

Other commands need no setup: a zsh started in a card reports each command's start and exit status to the app. See [Card states](../card-states/).

## Shell history per card

Each card gets its own zsh history file, so a new card does not open with every other card's past, and after a reboot a card that ran Claude has `claude --resume <id>` as its last history entry. macOS sets `HISTFILE` before your `.zshrc` runs, so add one line to `.zshrc`:

```zsh
HISTFILE="${INFINITERM_HISTFILE:-$HOME/.zsh_history}"
```

Outside infiniterm the variable is unset and your usual history file is used.

## Updates

Pro checks for a new build at launch and every six hours. A newer build is downloaded, its signature and notarization are checked, and it is installed the next time the app restarts. The status bar says "update N ready" until then. "Check for Updates…" in the app menu checks now.

The check is a plain GET of `latest.json` with nothing about you in it.

## Your first minute

- `Cmd T` opens a terminal card. `Cmd W` closes it.
- `Cmd Shift P` is the command palette: everything the app does is in it.
- `Cmd /` lists every shortcut, searchable.
- `Cmd` + scroll zooms, `Cmd` + drag pans. Bare scroll and drag belong to the card under the pointer.
- `Cmd 2` fits every card in the window, `Cmd 1` fits the focused one.
