---
title: Card states
description: What the four border colours mean and where they come from.
---

A card's border has four colours because there are four questions:

| Colour | State | Means |
| --- | --- | --- |
| Violet | working | an agent is mid-turn, or a command has been running for 5 seconds or more |
| Yellow | waiting | blocked on you: a permission prompt, or a question |
| Red | failed | a command exited non-zero, or the agent's turn failed |
| Green | done | the turn or the long command finished |

A shell doing nothing has no colour. Four hues rather than shades of one, because two oranges are one colour at 40% zoom.

## Agents

Claude Code and Pi report through hooks (`ift install-claude-hooks`, `ift install-pi-hooks`). The card goes violet when a turn starts, yellow when the agent asks for permission or asks you something, and green when the turn ends.

Green is news, so it does not stay: once you have looked at a done card for a moment (about a second and a half) or typed into it, it goes grey. "Card: clear the state colour" in the palette greys any card's ring and dot by hand once you have seen what it had to say.

Codex and OpenCode have no adapter yet.

## Any command

A zsh started in a card reports each command's start and exit status (OSC 133 marks) through a small shim the app installs, which then loads your own zsh files as usual. From that:

- A command that runs 5 seconds or more goes violet, then green if it exits 0. A quick `ls` never colours the card, or every shell card would be green and green would say nothing.
- A command that fails goes red however quick it was. Ctrl+C (exit 130) does not count as a failure.
- The next command clears the last result.
- A full-screen program (vim, htop, less) never colours the card.

Programs that print OSC 9;4 progress or OSC 9, 777 or 99 notifications colour the card too, whatever the shell.

Only zsh gets the marks for now, and only shells started after you installed or updated the app: a shell already running keeps the environment it started with.

## Workspace tabs

Each workspace tab wears one dot per card, in reading order, grey until that card has something to say. A done card keeps its dot green until you have looked at that card, so a workspace you only passed through still tells you which turns finished there.

## When a colour looks wrong

Every state change is appended to `agent.log` in `~/Library/Application Support/dev.ekinertac.infiniterm/`: the time, the card, the event, and what it did. Shell commands appear as `[shell]` lines with how long they ran. The log is capped at half a megabyte and never leaves the machine.

Agent state is not saved across restarts, so nothing comes back claiming to be working days after the agent died.
