# Launch plan

Written 2026-10-01. The goal for the first month is users and feedback, not sales: infiniterm on 50 to 100 Macs and a steady stream of real bug reports. The commercial licence ($29 per person) goes on sale when Lemon Squeezy approves the account; the launch does not wait for it.

## Where things stand

- Source public at [ekinertac/infiniterm](https://github.com/ekinertac/infiniterm) under a source-available licence: free for personal use, $29 per person for work.
- Signed, notarized builds, installed three ways: `curl -fsSL https://infiniterm.app/install.sh | sh`, `brew install --cask ekinertac/tap/infiniterm`, or the DMG.
- [infiniterm.app](https://infiniterm.app): landing page, the launch film (HTML, click to play), a real canvas screenshot, docs with keys, commands and settings generated from the app.
- Known limits a newcomer will hit: Apple Silicon only, macOS 13 or later, shell state colours from zsh only, agent hooks for Claude Code and Pi only (Codex and OpenCode not yet).

## How we know it worked

- **Active Macs.** Every running copy fetches `latest.json` at launch and every six hours, 4 to 5 times a day, and GitHub counts the downloads. 50 to 100 active Macs is roughly 250 to 450 fetches a day. `tools/usage-stats.sh` prints it. No telemetry is added for this; the count is what the update check already does.
- **Feedback.** 10 or more issues opened by someone other than Ekin within the month.
- Stars, upvotes and followers are noted but not the goal.

## Week 0: ready (2 to 3 days)

1. `tools/usage-stats.sh`: download counts per release and the daily active-Mac estimate from `latest.json`.
2. A blog on infiniterm.app (`/blog`), the one owned channel. A narrative post on our own domain has done better than a Show HN before (34 points against 1 or 2).
3. The film as an MP4 (typereel renders it) for Reddit and X, which cannot play the HTML version.
4. A first run on a clean Mac: install on the MacBook Air from nothing and screenshot what a stranger sees first. Fix what reads as broken.

## Week 1: private beta

Ekin sends the app to about 10 developers who run Claude Code or Pi, people he knows. One short message, drafted in advance. Whatever breaks for them gets fixed before anything public. Their first reactions are the raw material for the launch post.

## Week 2: launch

- **Blog post 1, the story.** Working hook: "I had 40 terminal tabs and three Claude sessions, and I couldn't tell which one was waiting on me." What broke in iTerm2, why cards on a canvas, the four colours, what it does not do. Film and screenshot in it. AI use disclosed in the body, framed around the decisions Ekin made and the ones he overruled.
- **Same day, r/ClaudeAI.** A post in Ekin's own words that links the blog, not a link drop. It is an AI-agent tool, so this is the launch channel.
- **Same day, X.** A short thread with the MP4.
- **All day:** answer every comment, turn every bug into an issue the same hour.

## Week 3: terminal channels

- Submit to [Terminal Trove](https://terminaltrove.com).
- Pull requests to the lists people browse: awesome-claude-code, macOS terminal and agent tool lists.
- The Homebrew cask and 15 GitHub topics are already in place.

## Week 4: harvest

- **Blog post 2, technical.** One of: why every card has its own small daemon instead of tmux (the ten bugs tmux produced in one evening), or how far a gpui canvas can zoom out before glyphs cost more than the frame (1.7 µs per glyph, the 30,000-cell budget, bars).
- **Hacker News** gets that post. HN is where a launch is harvested, not where it starts.

## Throughout

- Reply to every issue within a day.
- Keep `CHANGELOG.md` moving and say so: each notable feature gets a one-line post on X, so the project visibly ships.
- Every public text follows the house rules: plain engineer-to-peer voice, numbers, limits stated, no em-dashes, no hype words.

## Left out on purpose

- **Product Hunt:** it pays off with a following ready to vote on the day, and works against a users-first month.
- **An email waitlist:** it is a download, not a signup; there is nothing to wait for.
- **Paid ads:** no budget, and the audience ignores them.
- **Automated posting:** subreddits ban it, and it reads as spam.

## Open

- The Lemon Squeezy checkout link; the buy button says "Available soon" until then.
- Which Reddit and X accounts post, and whether they have any history (a brand-new Reddit account posting a link gets filtered).
