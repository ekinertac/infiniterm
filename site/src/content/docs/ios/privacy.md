---
title: "infiniterm for iPhone: Privacy Policy"
description: What infiniterm for iPhone keeps on your phone, what it sends to your own computers, and what the optional push relay stores.
---

Effective 24 November 2026.

infiniterm for iPhone ("the app") is a companion to infiniterm, a terminal app for the Mac that also runs on Linux. It is made by Tarık Kavaz, an individual developer. This policy says what the app does with your data.

## The short version

- There is no account and no sign-up.
- The app has no analytics, no advertising and no tracking.
- The app talks to your own computers directly over ssh. Nothing you type in a terminal goes through any server of ours.
- If you turn on "Notify when the app is closed", a small push relay keeps what it needs to send you notifications. It keeps card and workspace names for up to 24 hours. Everything else it keeps is deleted when you turn the setting off.

## What stays on your iPhone

- **Your hosts:** the name, address, user name, port and ssh host key of each computer you add, and the groups you sort them into.
- **Your ssh keys:** the private keys the app uses to sign in to your hosts. They are kept in the iPhone's Keychain and never leave the phone.
- **Settings and small lists:** your app settings, recently used snippets, recently sent messages and unsent drafts.
- **The widget's data:** the list of cards that need you, shared between the app, its notification extension and its widget on the phone.

This data is not sent to us. It is removed when you delete the app (Keychain items may stay until the iPhone is erased, as with any iOS app).

## What goes to your hosts

The app opens an ssh connection to each host you add, usually through Tailscale or your local network. Over it, the app reads your workspaces and terminal cards, shows their output, sends the text you type, makes the changes you ask for (such as moving or resizing a card, or saving a workspace), opens files read-only, and opens web pages that the host serves on its own `localhost`. This traffic goes only between your iPhone and your host, encrypted by ssh. We never see it.

## The push relay (only if you turn on notifications)

Apple only lets a server wake a closed iPhone app. So that your host can notify you while the app is closed, the developer runs a push relay on Cloudflare Workers. It is used only when you turn on Settings, Notifications, "Notify when the app is closed". If you never turn this on, the app does not contact the relay.

When it is on, the relay stores:

- **A push id**, a random identifier made by the relay. The app gives it to each of your hosts so they can send you notifications.
- **Your device's push token** (from Apple) and whether it is a test or a release build, so the relay can deliver notifications through Apple.
- **The widget's push token**, so the widget can be refreshed while the app is closed.
- **A read key**, a random secret that only your iPhone holds, so that only your phone can read back the card list below.
- **The last card list from each host:** card names, workspace names, their state (waiting, failed, done) and the time, plus the host's computer name. Each list is kept for up to 24 hours and is replaced whenever that host sends a new one.

The relay sees card and workspace names while it delivers a notification. These are not end-to-end encrypted: they are protected by HTTPS on the way to the relay and from the relay to Apple, but the relay itself can read them. The relay does not log notification contents.

**Deleting it:** turn off "Notify when the app is closed". The app tells the relay to forget your push id, and the relay deletes the push token, widget token, read key and stored card lists with it.

The relay's host, Cloudflare, and Apple's push service (APNs) process these requests, including network details such as IP addresses, to deliver them, under their own privacy policies.

## GitHub

To tell you when a newer version of infiniterm for your host is available, the app downloads a small public file from GitHub, where infiniterm's releases are published. GitHub sees that request like any web request, including your IP address. No personal data is sent with it.

## Permissions the app asks for

- **Camera:** only to scan the QR code your host shows when you add it. Images are not stored or sent anywhere.
- **Local network:** to reach your host on your network.
- **Face ID:** only if you turn on the app lock. Face ID is handled by iOS, and the app never sees your face data.
- **Notifications:** for the banners and the badge.

## Children

The app is a developer tool and is not directed at children.

## Changes

If this policy changes, the new version will be posted at this address with a new effective date.

## Contact

Questions about privacy: tarikkavaz@gmail.com
