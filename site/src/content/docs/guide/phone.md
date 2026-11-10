---
title: iPhone app
description: The iPhone companion app, in development. See your cards and their agent state, answer an agent from your phone, and pair it with a Mac using ift phone add.
---

> **In development.** The iPhone app is not released. It is not in the App Store yet. It will be published there later, so do not look for it there now. This page describes how it will work.

The iPhone app shows the canvas of a Mac that runs infiniterm. Its main use is to answer or prompt a Claude Code agent when you are away from the Mac.

## What it does

- Lists the Mac's workspaces and cards with their agent state: waiting on you, working, failed or done.
- Opens one terminal card. You see its output and its history, and you can send it text and keys. A Yes and No bar shows while a card is waiting for you.
- Shows the canvas, so you can move cards.
- Uses the snippets from the Mac.
- Opens a page served on the Mac's `localhost`, such as a dev server, in a browser inside the app. The page goes through the ssh connection to the Mac.
- Has an app lock, and shows English or Turkish.

The app is an ssh client. It reaches the Mac through Tailscale, to the Mac's own ssh server, and runs `ift proxy join`, the same protocol as `ift join`. There is no account. The app uses the Mac's own ssh and starts no server of its own. When Tailscale SSH is on, `ift phone add` also sets up a Tailscale port forward to the Mac's ssh, as [Pairing the iPhone app](../ift-and-sessions/#pairing-the-iphone-app) says.

## What you need

1. Tailscale on the Mac and on the phone, in the same tailnet.
2. Remote Login on in macOS: System Settings, General, Sharing, Remote Login.
3. infiniterm running on the Mac, and the Mac awake. The phone cannot wake it.
4. infiniterm 0.6.13 or newer on the Mac, which has `ift phone add`.

## Pair the phone with the Mac

1. On the Mac, in a terminal card, run:

   ```sh
   ift phone add iphone-12
   ```

   The name is yours: letters, digits, `-` and `_`, up to 32 characters. The command makes a key for that phone and prints a QR code.

2. On the phone, open the app. On the host list, tap "Scan the Mac's code" and point the camera at the code. The iPhone Camera app also works.

3. The Mac appears in the host list, ready to connect.

`ift phone list` shows the paired phones. To take a phone's access back, run:

```sh
ift phone remove iphone-12
```

The key stops working at once.

## What to know about the QR code

- The code holds the phone's private key. Anyone who sees it can use it, until you run `ift phone remove`.
- The code stays in that terminal card's saved output. Show it only to the phone.
- A paired key can run only a short list of infiniterm commands on the Mac. It cannot open a shell.

How the key is made and limited, the options `--host` and `--port`, and what happens when Tailscale SSH is on are in [Pairing the iPhone app](../ift-and-sessions/#pairing-the-iphone-app).

You can also add a Mac by hand, without a QR code:

1. In the app's Settings, make an ssh key and copy its public line.
2. On the Mac, add that line to `~/.ssh/authorized_keys`.
3. In the app, add the host: a name, the Mac's Tailscale name, the port, the user, and the path of `ift` on the Mac. For the app bundle, the path is `/Applications/infiniterm.app/Contents/MacOS/ift`.

`ift phone add` limits its key to infiniterm commands. A key you add by hand has only the limits you write in `authorized_keys`. With none, it can run a shell.

## Notifications

Notifications while the app is closed are optional. They go through one relay that is run by the app's signer. The relay sees card and workspace names while it passes them on, and keeps them for up to 24 hours. It is used only for notifications and widget updates. Without notifications, the app works as above and nothing goes through the relay.
