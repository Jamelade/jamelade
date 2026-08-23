<!--
SPDX-FileCopyrightText: 2026 Jamelade contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Privacy notice

Jamelade has no account service, advertising, analytics, crash-reporting SDK,
or project-operated backend. It does not collect Apple credentials, library
data, listening history, lyrics, Discord activity, or local settings.

GitHub and package hosts receive normal website or download traffic under
their own policies. Public issues and pull requests are visible to everyone.

## Required Apple connection

Apple receives sign-in, subscription, catalogue, library, playback, artwork,
and first-party lyric requests. Sign-in happens on Apple's own page in a
restricted Electron window. Jamelade stores only Apple-domain cookies, in one
OS-keyring-encrypted local vault. If a supported keyring is unavailable, the
session is memory-only. Developer and music-user tokens remain in memory.

Apple applies its own privacy policy and service terms to this traffic.

## Optional connections

- **LRCLIB lyrics:** off by default. When enabled after Apple has no usable
  lyric, LRCLIB receives the track title, artist, album, duration, and the
  requester's IP address. Jamelade sends no Apple cookie or token. LRCLIB had no
  public privacy policy when this notice was reviewed.
- **Discord activity:** off by default. When enabled, Jamelade gives the local
  Discord-compatible client the current title, artist, album, selected Jamkin,
  and playback timing. Discord then applies the user's activity and audience
  settings. No Discord token or client secret is used.
- **Desktop Jamkin, themes, and collages:** local only. They reuse already
  loaded playback state and cached artwork.

Disabling an optional connection stops new requests to it and clears Jamelade's
in-memory lyric cache when lyric consent changes.

## Local data

Jamelade stores preferences, bounded artwork and playlist caches, the encrypted
Apple-cookie vault, and Chromium's downloaded Widevine component in its own XDG
directories. It requests no broad home-directory access in the Flatpak. Sign
out removes Jamelade's saved Apple session. Uninstalling without deleting app
data may leave local settings and caches behind.

## Issue reports

GitHub issues are public. Do not attach cookie vaults, Apple tokens, account
screenshots, private playlist names, unredacted logs, home-directory paths, or
Discord identities. Use [`SECURITY.md`](SECURITY.md) for vulnerability reports.

Changes to data collection or network destinations must update this notice.
