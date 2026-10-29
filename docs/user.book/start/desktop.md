---
id: 01M64TR9D5EGS2CJXNXTASG3J2
title: Desktop, phone and offline
vellum: 1
---

# Desktop, phone and offline

Vellum is one app in three shells: the browser, an installed web app on
your phone or computer, and the desktop app for macOS, Windows and Linux.
They look and behave the same, and all three keep working when the
network drops.

## Install it {#install}

- **Desktop**: download the app for your system from the Vellum release
  page, then sign in to your server.
- **Phone or tablet**: open your Vellum address, then choose **Add to
  Home Screen** (iOS) or **Install app** (Android).
- **Computer, without the desktop app**: in Chrome or Edge, choose
  **Install Vellum** in the address bar.

The desktop app signs you in through your system browser, so single
sign-on and passkeys work as usual. It keeps your sign-in encrypted with
the operating system's keychain.

On a phone the navigation rail becomes a bar at the bottom of the
screen.

## Working offline {#offline}

Keep working when the network drops. Pages you opened recently, your
issues, tasks and collection rows stay on the device, and every change
you make is saved there first, so the screen never waits on the server.

When you are back online, Vellum sends your changes in the order you made
them and merges them with everyone else's. Two people editing the same
paragraph both keep their words: page edits merge character by
character. A change the server refuses, say because you lost access to
the page meanwhile, is dropped and Vellum tells you which one.

::: tip
The web app keeps your most recent pages, up to 50 MB. The desktop app
keeps whole spaces, so everything you can open is there on a plane.
:::

Signing out deletes everything Vellum stored on the device.

## Notifications on this device {#notifications}

**Inbox → Notification settings → This device → Turn on push** turns
on notifications for the browser or app you are using. Vellum only asks
the browser for permission when you click, and a notification only ever
opens Vellum. On iPhone and iPad, add Vellum to your Home Screen first.

## Private spaces on the desktop {#private-spaces}

A private space is encrypted on your device before anything reaches the
server, so the server stores only ciphertext. Your device needs its own
key to open it:

1. In the desktop app, go to **Private spaces** at
   `/settings/private-spaces`.
   A space manager chooses **Make private and enrol this device** on a
   shared drive; everyone else chooses **Enrol this device**.
2. Someone whose device can already open the space chooses **Reseal
   pages for every device**, so the new device can open them.
3. Search and Claude work on the device itself; see
   [Private spaces](../ai/README.md#private) for connecting Claude.

Enrolling or removing a device asks you to have signed in within the
last 10 minutes. After you remove a lost laptop, reseal the pages from
another device so it can't open new versions.
