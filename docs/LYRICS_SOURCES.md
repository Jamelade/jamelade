<!--
SPDX-FileCopyrightText: 2026 Jamelade contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Lyrics sources

Jamelade does not bundle lyrics or save fetched lyrics to disk. It tries
providers in order and stops after the first usable result.

## Apple Music

Apple Music is the default source for signed-in users. Jamelade sends the
numeric catalogue song ID to Apple's API and parses the returned timed lyrics
locally. This adds no recipient beyond the service already used for playback.

## LRCLIB

LRCLIB is an optional fallback and is off by default. When enabled, it receives
the track title, artist, album, duration, and the requester's IP address. It
receives no Apple cookie, token, account ID, or playlist data.

LRCLIB's software is open source, but that licence does not grant rights in the
lyric text returned by its database. Jamelade therefore treats fetched lyrics
as transient display data rather than redistributable project content.

## Privacy and safety limits

- Apple and LRCLIB use separate HTTP clients.
- Requests use fixed HTTPS origins and refuse redirects.
- Apple responses are capped at 2 MiB; third-party responses at 256 KiB.
- TTML and JSON are validated before display.
- At most 64 results remain in memory.
- The cache clears on sign-out or when lyrics consent changes.

New providers must use HTTPS, minimize track metadata, avoid borrowed account
tokens and scraping, document where requests go, and remain separately
opt-in. A provider's code licence must not be mistaken for a licence to
republish song lyrics.
