<!--
SPDX-FileCopyrightText: 2026 Jamelade contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Jamelade architecture

Jamelade is a native Rust and GTK application with one deliberately narrow
Electron sidecar. The native process owns the interface, Apple Music API
access, local caches, settings, desktop integration, and optional services.
The sidecar exists only because protected Apple Music playback on Linux needs
Chromium and Widevine.

## Design rules

1. **Prefer native platform components.** Use GTK4 and libadwaita widgets and
   desktop portals before custom CSS, shell commands, or filesystem access.
2. **Keep application state in the model.** Widgets emit intent and render a
   projection; they do not become an independent source of truth.
3. **Treat playback views as projections.** MusicKit owns the live queue and
   playback state. Load a queue as a queue so track boundaries remain gapless;
   do not rebuild it one track at a time or silently overwrite its state.
4. **Make the Apple-page boundary small and loud.** `sidecar/page-hook.js` is
   the fragile integration surface. If Apple's page changes, reject unexpected
   data and report a bounded failure instead of guessing or failing silently.
5. **Use stable identities across asynchronous work.** Rows move, requests
   finish out of order, and views rebuild. Correlate results with resource IDs
   or explicit generation keys rather than stale list positions.
6. **Supervise the sidecar.** Bound every pipe, queue, line, message, and retry.
   Detect process death, report it to the model, and restart with capped
   backoff instead of presenting a dead player as healthy.
7. **Keep credentials out of ordinary storage and logs.** Harvest MusicKit
   tokens only for the current process. Persist Apple cookies only in the
   keyring-encrypted vault; if secure encryption is unavailable, use an
   ephemeral session. Never log raw protocol lines or credential-bearing URLs.
8. **Never block the GTK main thread.** Network requests, image decoding,
   palette work, disk-heavy operations, and child-process I/O belong in bounded
   asynchronous tasks or workers.
9. **Parse at the boundary.** Remote JSON and sidecar messages become bounded,
   typed values before reaching UI code. Keep the complete sidecar protocol in
   `src/player/protocol.rs`; components should not parse remote shapes.

## Trust boundaries

- `src/music/client.rs` sends credentials only to the fixed Apple Music API
  origin, refuses redirects, and turns Apple's bounded TTML lyrics into native
  line data before they reach components.
- `sidecar/security.js` restricts privileged Chromium navigation, network
  destinations, renderer events, and diagnostic text.
- `sidecar/session-vault.js` validates Apple cookies and stores only an
  encrypted, size-bounded snapshot with private filesystem permissions.
- `src/private_storage.rs` provides bounded, no-follow reads and atomic
  user-only writes for library, playback, and preference data.
- `src/lyrics.rs` parses Apple TTML as bounded streaming XML and uses a separate
  credential-free HTTP client for independently opted-in fallbacks. Successful
  lyrics remain in memory.
- `src/discord.rs` is off by default and talks only to a validated same-user
  local Discord socket.
- `src/apple_link.rs` produces only bounded public `https://music.apple.com`
  links and never includes an Apple cookie, token, or account identifier.
- `icon-helper/` is an optional same-user D-Bus service with no network client.
  It accepts only the three public Jamkin names and can replace only Jamelade's
  fixed launcher entry.

Changes to authentication, network allowlists, navigation, storage, logging,
the sidecar protocol, dependencies, or packaging need an explicit security and
privacy review. See [CONTRIBUTING.md](CONTRIBUTING.md) and
[SECURITY.md](SECURITY.md).
