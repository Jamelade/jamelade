<!--
SPDX-FileCopyrightText: 2026 Jamelade contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Legal notice

This is a project notice, not legal advice.

## Unofficial compatibility project

Jamelade is an independent community fork of Slipmat. It is not affiliated
with, endorsed by, sponsored by, or approved by Apple Inc. Apple can change or
block the web services Jamelade relies on at any time.

Jamelade loads `music.apple.com` and asks MusicKit's authenticated browser
client to perform a narrow, allowlisted set of Apple API requests. Apple
cookies and MusicKit tokens stay inside that browser boundary; the native
process receives only bounded response data. This remains an unofficial
compatibility technique rather than Apple's documented third-party MusicKit
integration model. Apple's terms may restrict this use, and Apple can change
or block the service at any time.

Relevant Apple sources:

- [Apple Media Services Terms and Conditions](https://www.apple.com/legal/internet-services/itunes/)
- [MusicKit developer overview](https://developer.apple.com/musickit/)
- [Apple Developer agreements](https://developer.apple.com/support/terms/)

## Trademarks

Apple and Apple Music are trademarks of Apple Inc., registered in the U.S. and
other countries. Other product and service names are the property of their
respective owners and are used only to identify compatibility or a network
destination.

Jamelade does not include an Apple logo, Apple Music application icon, Apple
badge, or Apple font in its current source tree or release bundle. Marketing
must follow Apple's current
[trademark guidelines](https://www.apple.com/legal/intellectual-property/guidelinesfor3rdparties.html)
and
[Apple Music identity guidelines](https://marketing.services.apple/apple-music-identity-guidelines).

## Music, artwork, metadata, and lyrics

Music, cover art, editorial artwork, metadata, artist biographies, and lyrics
displayed at runtime come from Apple or an optional lyrics service. Their
rights remain with their respective owners. Jamelade's GPL licence does not
grant permission to republish or commercially reuse that material. LRCLIB and
Lyrics.ovh are off by default; a service or software licence does not by itself
license the text it returns.

## Jamkin artwork

The Jamkin assets are project-created artwork. Contributors license their
contributions and any rights they hold under GPL-3.0-or-later. See
[`data/companions/README.md`](data/companions/README.md).

## Software licence and warranty

Jamelade is distributed under GPL-3.0-or-later. The complete terms, including
the warranty disclaimer, are in [`COPYING`](COPYING). Slipmat and third-party
components retain their own licences and notices; builds preserve the notices
shipped with Rust dependencies, GTK components, Electron, and Chromium.
