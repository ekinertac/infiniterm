---
title: infiniterm
description: Terminal cards on an infinite canvas, with the state of every coding agent visible at a glance.
template: splash
hero:
  tagline: Terminal cards on an infinite canvas, with the state of every coding agent visible at a glance.
  actions:
    - text: Install
      link: guide/install/
      icon: right-arrow
    - text: Read the guide
      link: guide/canvas/
      variant: minimal
---

<iframe src="film.html?play" title="infiniterm in 48 seconds" loading="lazy" style="display:block;width:100%;aspect-ratio:16/9;border:0;border-radius:8px;background:#0e0f13"></iframe>

```sh
curl -fsSL https://ekinertac.github.io/infiniterm-releases/install.sh | sh
```

It started as a way out of iTerm2: ten to fifteen tabs, two or three splits in each, and finding one session among twenty to fifty meant opening them one by one. Here every session is a card on one canvas and keeps its place. Running coding agents made the old way worse, because you cannot tell which are working, which are waiting on you and which have finished without cycling through them. So each card's border says it.

Nothing flashes, nothing steals focus, nothing sends a notification. You switch agents when you are ready, not when one finishes.

A native macOS app in Rust: gpui draws the canvas, `alacritty_terminal` parses the shells, Chromium (CEF) runs the browser cards. No Electron, no account, no telemetry. Apple Silicon, macOS 13 or later. Early, and in active development.

## Start here

1. [Install and first launch](guide/install/)
2. [The canvas](guide/canvas/): cards, panning, zooming, where new cards go
3. [Card states](guide/card-states/): the four border colours
4. [`ift` and sessions](guide/ift-and-sessions/): driving the app from a shell, and shells that outlive the window
5. [Configuration](guide/configuration/): settings, keybindings, themes, snippets

The [keys](reference/keys/), [commands](reference/commands/) and [settings](reference/settings/) pages are generated from the app's own tables, so they match the release.
