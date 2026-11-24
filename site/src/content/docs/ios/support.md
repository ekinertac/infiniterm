---
title: "infiniterm for iPhone: Support"
description: How to set up infiniterm for iPhone, what to check when it does not connect, and how to contact the developer.
---

infiniterm for iPhone lists the workspaces and cards of infiniterm on your computer, opens a card and lets you answer or prompt an AI coding agent, such as Claude Code, from your phone.

## What you need

- A Mac or Linux computer running [infiniterm](https://infiniterm.app), with ssh turned on. On a Mac: System Settings, General, Sharing, Remote Login.
- A way for the phone to reach it. Tailscale on the computer and the phone, signed in to the same account, works from anywhere. On the same local network, the computer's local address works too.
- For notifications while the app is closed: infiniterm 0.6.11 or later.

## Set up

1. In the app, tap +, then Add host. It shows an `ift phone add` command.
2. Run that command on the computer. It prints a QR code.
3. In the app, tap Scan the host's code and scan it. Without Tailscale, add `--host` and an address the phone can reach to the command.
4. Tap the host in the list. Your workspaces appear.

The QR code carries a key made for this phone only, and the computer limits that key to infiniterm's own commands. To add a host by hand instead: in Settings, General, ssh key, tap Generate key, copy the public key, add it as a new line at the end of `~/.ssh/authorized_keys` on the computer, then tap +, Add host, and fill in the Manual fields.

The Help screen in the app covers every feature.

## When something fails

- **Cannot connect:** check Tailscale on both devices, that the computer is awake, and that infiniterm runs there.
- **Refused:** the phone's key is not in `~/.ssh/authorized_keys` on the computer, or the user name is wrong. For a host added by QR code, delete it and pair again.
- **ift not found:** in the host's Edit page, set Remote ift path. On a Mac it is `/Applications/infiniterm.app/Contents/MacOS/ift`. On Linux, run `command -v ift` on the computer.
- **Fingerprint changed:** continue only if you know why, for example after reinstalling the computer.
- **No notifications with the app closed:** check that Settings, Notifications, "Notify when the app is closed" is on, and that infiniterm on the computer is 0.6.11 or later. A Mac sends nothing while the infiniterm window is in front. Switching the setting off and on again registers the phone afresh.

## Privacy

No account, no analytics, no tracking. See the [privacy policy](/ios/privacy/).

## Contact

Questions, bugs and ideas: tarikkavaz@gmail.com
