<!--
SPDX-FileCopyrightText: 2026 Jamelade contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Jamelade architecture

Jamelade is a native Rust and GTK application with one deliberately narrow
Electron sidecar. The native process owns the interface, typed Apple data,
local caches, settings, desktop integration, and optional services. The sidecar
owns the authenticated Apple web session, MusicKit API transport, protected
playback, Chromium, and Widevine.

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
   backoff instead of presenting a dead player as healthy. Widevine readiness
   is bounded too: a stalled component updater exits into that same recovery
   path rather than leaving the interface on “Preparing playback” forever.
7. **Keep credentials inside the browser boundary.** Do not copy MusicKit
   tokens into the preload bridge or Rust. Persist Apple cookies only in the
   keyring-encrypted vault; if secure encryption is unavailable, use an
   ephemeral session. Never log raw protocol lines or credential-bearing URLs.
8. **Never block the GTK main thread.** Network requests, image decoding,
   palette work, disk-heavy operations, and child-process I/O belong in bounded
   asynchronous tasks or workers.
9. **Parse at the boundary.** Remote JSON and sidecar messages become bounded,
   typed values before reaching UI code. Keep the complete sidecar protocol in
   `src/player/protocol.rs`; components should not parse remote shapes.

## Trust boundaries

- `src/music/client.rs` sends typed methods and relative paths to the browser
  broker and turns bounded Apple responses, including TTML lyrics, into native
  data. It has no Apple credential, header map, origin, or arbitrary URL.
- `src/music/biography.rs` is a separate credential-free client for the fixed
  public US artist-page fallback. It accepts only numeric IDs, one validated
  canonical `music.apple.com` redirect, bounded HTML, and the explicit
  `artist-bio` JSON entry; it has no cookie jar or browser token.
- `sidecar/page-hook.js` allowlists Apple API routes, invokes MusicKit's own
  authenticated client, waits a bounded three seconds when session readiness
  precedes API-method readiness, and emits capped responses plus
  credential-free session state.
- `src/music/client.rs` retries only idempotent GETs, four times with bounded
  backoff, after a 502/503/504 response. It never automatically repeats writes,
  authentication failures, rate limits, or arbitrary statuses.
- `sidecar/security.js` restricts privileged Chromium navigation, network
  destinations, renderer events, and diagnostic text.
- The Flatpak launch forces Zypak's nested mimic sandbox because its newer
  portal-spawn strategy hangs on current Flatpak/bubblewrap. Chromium's GPU is
  disabled and kept in-process so that fallback needs no GPU helper; the Apple
  renderer remains sandboxed with context isolation and no Node access.
- `sidecar/session-vault.js` validates Apple cookies and stores only an
  encrypted, size-bounded snapshot with private filesystem permissions.
  `sidecar/persistence.js` owns bounded keyring readiness and restore retries;
  failure preserves the old vault with writes disabled, and only a visibly
  completed login may supersede it.
- `sidecar/auth-preload.js` remains empty. During an explicitly visible login,
  `sidecar/login-email-assist.js` attaches a bounded listener only to fixed
  email/username-like fields in validated HTTPS Apple frames. Navigation drops
  the listener with its frame. The result is validated again and stored
  separately through keyring encryption by `sidecar/login-email.js`; no
  password selector or renderer IPC is added.
- `src/private_storage.rs` provides bounded, no-follow reads and atomic
  user-only writes for library, playback, and preference data.
- `src/lyrics.rs` parses Apple TTML as bounded streaming XML and uses a separate
  credential-free HTTP client for independently opted-in fallbacks. A fallback
  clock reaches Apple localizations only after exact normalized line alignment;
  successful lyrics remain in memory.
- `src/lyric_timing.rs` stores only numeric catalogue IDs and bounded offsets;
  it never writes lyric text or listening metadata.
- `src/scrobble.rs` is off by default, encrypts its token with the desktop
  Secret portal, and can submit only bounded visible metadata to one fixed
  ListenBrainz HTTPS endpoint.
- `src/app/global_shortcuts.rs` uses the XDG portal and receives only four fixed
  action IDs. It never observes or stores the user's key combinations.
- Playlist writes expose only Apple's documented create and append operations.
  Export files contain visible metadata and public links, not library IDs.
- `src/discord.rs` is off by default and talks only to a validated same-user
  local Discord socket.
- `src/apple_link.rs` accepts only bounded HTTPS URLs on `music.apple.com`.
- `icon-helper/` is an optional same-user D-Bus service. Its only method accepts
  three fixed Jamkin names and replaces only Jamelade's launcher.
- `scripts/update-electron-runtime.mjs` resolves only stable castLabs WVCUS
  tags and checksummed release assets. Its scheduled workflow may open a PR;
  it cannot merge, install, publish, or sign a release.

Changes to authentication, network allowlists, navigation, storage, logging,
the sidecar protocol, dependencies, or packaging need an explicit security and
privacy review. See [CONTRIBUTING.md](CONTRIBUTING.md) and
[SECURITY.md](SECURITY.md).
