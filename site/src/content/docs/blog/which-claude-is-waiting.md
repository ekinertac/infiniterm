---
title: I couldn't tell which Claude was waiting on me
description: Forty terminal tabs, three agents, and why every session in infiniterm is a card with a coloured border.
draft: true
sidebar:
  order: 1
---

For years my terminal was iTerm2 with ten to fifteen tabs, two or three splits in each, and a few panes blown up to full screen. Somewhere past twenty sessions I lost the trail. Finding the one I wanted meant clicking through tabs until I recognised a prompt.

Coding agents made it worse. With three Claude Code sessions running in three of those tabs, the question was never "where is it" but "which one needs me". One is still working, one has been sitting on a permission prompt for ten minutes, one finished a while ago. From the tab bar they look the same. I cycled through them to find out, and every cycle broke whatever I was doing.

## Cards on a canvas

infiniterm puts every session on one canvas you pan and zoom. Each terminal is a card, and a card stays where you put it: nothing re-tiles, and a new card lands in the next free slot of a grid that starts at the top left, so after a week you know where things are the way you know where things are on your desk.

The part I built it for is the border. A card's border has four colours, because there are four questions:

- violet: working
- yellow: waiting on you, a permission prompt or a question
- red: failed
- green: done

Claude Code and Pi report through hooks. Any other command reports through zsh: a build that runs five seconds or more goes violet, then green, and a command that fails goes red however quick it was. A shell doing nothing has no colour.

Zoomed out to fit thirty cards, the text turns into bars, but the borders stay readable. I look up, see one yellow card, and go there.

## What it does not do

Nothing flashes, nothing steals focus, nothing sends a notification. That was deliberate. The state is there when I look, and I look when I am ready, not when an agent finishes.

It is also not trying to be every terminal. It is a native macOS app for Apple Silicon, zsh is the only shell that reports its commands so far, and the agent hooks cover Claude Code and Pi (Codex and OpenCode are not there yet). There is no Electron, no account and no telemetry; the one request it makes on its own is the update check.

## How it was built

Most of the code was written by Claude Code sessions running in cards on this same canvas. My part was the product: what a card is, what goes on the border, and a lot of saying no. Some of the calls I made along the way:

- Failed was its own colour late. It started as part of "waiting", until I realised a question and a crash are different news.
- "Working" was Claude's clay orange at first. At fit-all zoom it blurred into red, so it went violet.
- A group of cards has a frame, and the frame carries no colour. It holds several sessions, and one colour could not say which of them wants you.
- Shells survive quitting the app. That first ran on tmux, for one evening: it produced ten bugs, so each card now has its own small daemon that holds the terminal and replays it byte for byte.
- Zoomed-out text was tried as word shapes and reverted within the hour. Flat bars read better and cost less.

The repository's `CLAUDE.md` holds the long list, with the reasons.

## Try it

```sh
curl -fsSL https://infiniterm.app/install.sh | sh
```

or `brew install --cask ekinertac/tap/infiniterm`. It is free for personal use; paid work needs a $29 licence per person. The source is on [GitHub](https://github.com/ekinertac/infiniterm), and bug reports are the thing I want most right now.
