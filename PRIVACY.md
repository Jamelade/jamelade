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
session is memory-only. Apple cookies and MusicKit tokens remain inside the
browser boundary; the native process receives only bounded response data.

When an Apple login is required, Jamelade can remember the last entered Apple
ID email and prefill it on the next HTTPS Apple login page. The address stays in
a separate OS-keyring-encrypted, user-only file and is sent only to Apple's
login page when used. Jamelade does not read, store, or bridge the password
field.

Artist biographies may use a separate credential-free request to Apple's
public artist page when the authenticated API response has no biography. That
request sends the numeric artist ID and the requester's IP address to Apple; it
has no cookie jar, Apple token, or account identifier.

Apple applies its own privacy policy and service terms to this traffic.

## Optional connections

- **LRCLIB lyrics:** off by default. When enabled after Apple has no usable
  lyric, LRCLIB receives the track title, artist, album, duration, and the
  requester's IP address. Jamelade sends no Apple cookie or token. LRCLIB had no
  public privacy policy when this notice was reviewed.
- **Lyrics.ovh lyrics:** off by default and contacted only after earlier
  enabled sources have no usable result. It receives the artist, title, and
  requester's IP address and may query downstream lyric sites. Jamelade sends
  no Apple cookie or token.
- **Discord activity:** off by default. When enabled, Jamelade gives the local
  Discord-compatible client the current title, artist, album, selected Jamkin,
  and playback timing. Discord then applies the user's activity and audience
  settings. No Discord token or client secret is used.
- **ListenBrainz scrobbling:** off by default. When enabled, Jamelade sends the
  title, artist, album, duration, listen time, and requester's IP address to
  `api.listenbrainz.org`. It sends no Apple identifier, credential, artwork,
  lyric, or playlist name. The token is encrypted locally with a per-app key
  supplied by the desktop keyring portal.
- **Global shortcuts:** off by default and local only. The desktop portal owns
  the chosen bindings; Jamelade receives only one of four fixed action names.
- **Search history:** on by default and local only. At most 16 normalized
  catalogue queries are kept in a private XDG state file. Disabling recording
  leaves existing entries untouched; removing a pill or choosing Clear History
  updates only that file. Trending Now uses Apple's ordinary storefront chart,
  not a third-party service or scraped query feed.
- **Desktop Jamkin, themes, and collages:** local only. They reuse already
  loaded playback state and cached artwork.
- **Launcher-icon helper:** optional and installed separately for the current
  user. It has no network client, accepts only the three bundled Jamkin names,
  and writes only Jamelade's fixed launcher entry. Without it, Jamelade uses
  the desktop's confirmation portal.

Disabling an optional connection stops new requests to it and clears Jamelade's
in-memory lyric cache when lyric consent changes.

## Local data

Jamelade stores preferences, bounded artwork and playlist caches, the encrypted
Apple-cookie vault, the separately encrypted last-login email, and Chromium's
downloaded Widevine component in its own XDG directories. Search history is a
bounded user-only file under XDG state. Per-song lyric timing
stores only numeric catalogue IDs and offsets. Playlist export writes a
user-chosen file containing visible metadata and public links. It requests no
broad home-directory access in the Flatpak. The AppImage is a native executable
and therefore inherits the invoking user's normal filesystem permissions,
although Jamelade's code continues to use only its documented XDG locations.
Sign out removes Jamelade's saved Apple session but retains the encrypted email
for the next login and local search history until it is cleared from Search.
Deleting application data removes all three. Uninstalling without deleting app
data may leave local settings and caches behind.

## Issue reports

GitHub issues are public. Do not attach cookie vaults, Apple tokens, account
screenshots, private playlist names, unredacted logs, home-directory paths, or
Discord identities. Use [`SECURITY.md`](SECURITY.md) for vulnerability reports.

Changes to data collection or network destinations must update this notice.
