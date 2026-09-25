---
title: Install and first launch
description: Download the app, put ift on your PATH, and wire up agent state.
---

infiniterm runs on Apple Silicon Macs with macOS 13 or later. Builds are signed with a Developer ID and notarized by Apple, so Gatekeeper opens them without a warning.

## Install

```sh
curl -fsSL https://ekinertac.github.io/infiniterm-releases/install.sh | sh
```

It downloads the latest release, checks its checksum, its signature against the developer's team and its notarization, copies it into `/Applications`, and puts `ift` on your PATH (a symlink in `~/.local/bin`, plus zsh completion). If infiniterm is already installed it leaves the app alone, since the app updates itself. [Read the script](/infiniterm-releases/install.sh) first if you like; it is about 120 lines of sh.

### Agent hooks

Card borders show what an agent in the card is doing. Claude Code and Pi report through hooks, which means editing their config, so the script asks first when it finds them. When nobody is there to answer (an agent running the install, a CI job), it skips the hooks and prints the commands. To decide up front:

```sh
curl -fsSL https://ekinertac.github.io/infiniterm-releases/install.sh | sh -s -- --hooks claude,pi
curl -fsSL https://ekinertac.github.io/infiniterm-releases/install.sh | sh -s -- --no-hooks
```

Or later, from any shell:

```sh
ift install-claude-hooks      # edits ~/.claude/settings.json
ift install-pi-hooks          # installs the Pi extension into ~/.pi/agent
```

`ift install-pi-hooks DIR` takes a different agent directory, for a wrapper that runs Pi against its own. Both accept `--dry-run`.

Other commands need no setup: a zsh started in a card reports each command's start and exit status to the app. See [Card states](../card-states/).

### Without the script

1. Download the `.dmg` from the [latest release](https://github.com/ekinertac/infiniterm-releases/releases/latest).
2. Drag infiniterm into Applications and launch it once.
3. In any card: `/Applications/infiniterm.app/Contents/MacOS/ift install`, then the hook commands above.

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
