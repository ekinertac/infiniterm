---
title: Install and first launch
description: Download the app, put ift on your PATH, and wire up agent state.
---

infiniterm runs on Apple Silicon Macs with macOS 13 or later. It is free for personal use; paid work, freelance included, needs a [commercial licence](https://infiniterm.lemonsqueezy.com/checkout/buy/0b6837dd-c037-493e-a530-72f6273fdf3c), $29 per person, one time.

## Install

```sh
curl -fsSL https://infiniterm.app/install.sh | sh
```

It downloads the latest release, checks its checksum, its signature and its notarization the way the app's own updater does, copies it into `/Applications`, and puts `ift` on your PATH. An app already installed is left alone, since it updates itself. [Read the script](/install.sh) first if you like.

Agent hooks edit the agent's config, so the script asks before wiring them, and only for the agents it finds (Claude Code, Pi, Codex, OpenCode). With nobody at a terminal to answer (an agent running the install, a CI job) it skips them and prints the commands. To decide up front, end the line with `| sh -s -- --hooks claude,pi,codex,opencode` (any of them) or `| sh -s -- --no-hooks`.

## Homebrew

```sh
brew install --cask ekinertac/tap/infiniterm
```

The cask links `ift` onto your PATH too. The app updates itself, so `brew upgrade` leaves it alone. If you installed the app from the DMG before, add `--adopt` so Homebrew takes over the copy you have.

## Download by hand

The app is signed and notarized, so Gatekeeper opens it without a warning, and it updates itself.

1. Download the `.dmg` from the [latest release](https://github.com/ekinertac/infiniterm/releases/latest).
2. Open it and drag infiniterm into Applications, then launch it once.
3. In any card, put `ift` on your PATH:

   ```sh
   /Applications/infiniterm.app/Contents/MacOS/ift install
   ```

   This symlinks `ift` into `~/.local/bin` and writes its zsh completion. If either directory is not on your `PATH` or `fpath`, it prints the line to add.

## Build from source

The source is public. You need Rust, the CEF binary distribution and a checkout of cef-rs for its bundler; the README has the steps. Builds are yours to use, not to hand to anyone else.

## Agent hooks

Card borders show what an agent in the card is doing. Each agent reports through hooks, installed once from any shell:

```sh
ift install-claude-hooks      # Claude Code: edits ~/.claude/settings.json
ift install-pi-hooks          # Pi: installs an extension into ~/.pi/agent
ift install-codex-hooks       # Codex: edits ~/.codex/hooks.json (or $CODEX_HOME)
ift install-opencode-hooks    # OpenCode: writes a plugin to ~/.config/opencode/plugins
```

Codex asks you to approve new hooks once: run `/hooks` inside Codex after installing. `ift install-pi-hooks DIR` takes a different agent directory, for a wrapper that runs Pi against its own. All four accept `--dry-run`, which prints what would change and changes nothing.

They are safe to run again, after an update or from a dotfiles script: they add nothing the second time. `install-claude-hooks` merges eight hook entries into your `settings.json` and leaves everything else in it alone (permissions, env, other hooks), and it refuses to touch a file that does not parse. Each prints the file it changed.

An agent session that was already running keeps the settings it started with, so start a new session to see its card change colour. To check it works, give the agent a prompt: the card's border goes violet. If it does not, `agent.log` in `~/Library/Application Support/dev.ekinertac.infiniterm/` shows whether any hook event arrived.

To remove them: delete the entries whose command runs `infiniterm-hook` from `~/.claude/settings.json` and `~/.codex/hooks.json`, and delete `~/.pi/agent/extensions/infiniterm.ts` and `~/.config/opencode/plugins/infiniterm.js`.

Other commands need no setup: a zsh started in a card reports each command's start and exit status to the app. See [Card states](../card-states/).

## Shell history per card

Each card gets its own zsh history file, so a new card does not open with every other card's past, and after a reboot a card that ran Claude has `claude --resume <id>` as its last history entry. macOS sets `HISTFILE` before your `.zshrc` runs, so add one line to `.zshrc`:

```zsh
HISTFILE="${INFINITERM_HISTFILE:-$HOME/.zsh_history}"
```

Outside infiniterm the variable is unset and your usual history file is used.

## Registering a licence

Optional. The app never asks for the key and works the same without it; the key in your receipt email is your proof of purchase either way. To have the About window say who the Mac is licensed to:

```sh
ift licence you@example.com XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX
```

It asks Lemon Squeezy once whether the key is an infiniterm licence bought with that email, and saves it in `licence.json` beside the canvas. Nothing re-checks it later. Bare `ift licence` says whether this Mac is registered. It exits 3 when the key or email is refused and 4 when Lemon Squeezy cannot be reached.

## Updates

The app checks for a new build at launch and every six hours. A newer build is downloaded, its signature and notarization are checked, and it is installed the next time the app restarts. The status bar says "update N ready" until then. "Check for Updates…" in the app menu checks now. "About infiniterm" in the app menu (or "App: about infiniterm" in the palette) shows the version and build you are running.

The check is a plain GET of `latest.json` with nothing about you in it.

## Your first minute

- `Cmd T` opens a terminal card. `Cmd W` closes it.
- `Cmd Shift P` is the command palette: everything the app does is in it.
- A first launch opens a "Start here" page beside the first terminal; "Help: open the welcome card" brings it back.
- `Cmd /` lists every shortcut, searchable.
- "Help: open the docs" in the palette opens these pages inside the app, as a card with the page list beside it.
- `Cmd` + scroll zooms, `Cmd` + drag pans. Bare scroll and drag belong to the card under the pointer.
- `Cmd 2` fits every card in the window, `Cmd 1` fits the focused one.
