<!--
SPDX-FileCopyrightText: 2026 Miguel Rincon
SPDX-FileCopyrightText: 2026 Jamelade contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

<p align="center">
  <img src="docs/screenshots/icon.png" width="144" alt="JamBun, Jamelade's default Jamkin companion, wearing headphones">
</p>

<h1 align="center">Jamelade</h1>

<p align="center">
  A native-feeling, privacy-conscious Apple Music client for Linux—with live lyrics, album-aware glass, and a Jamkin singing along.
</p>

Jamelade is an unofficial community fork of
[Slipmat](https://github.com/SoftARV/Slipmat). It keeps Slipmat's native
GTK4/libadwaita library and player, then builds a more personal desktop
experience around it: Apple Music Home and Explore, synchronized lyrics,
album-aware glass styling, optional Discord activity, and three project-created
Jamkin companions—**JamBun**, **JamPam**, and **JamJoe**.

Jamelade is not affiliated with or endorsed by Apple Inc. or the upstream
Slipmat project. An active Apple Music subscription is required.

## What Jamelade brings to Slipmat

### Apple Music Home, made native

Explore presents the recommendation groups Apple exposes to the signed-in
account, alongside recent listening, heavy rotation, radio, and storefront
charts. The shelves are native widgets rather than an embedded copy of Apple's
website.

### Live lyrics with privacy-first fallbacks

Jamelade follows synchronized lyrics while keeping the selected Jamkin visible.
It tries Apple Music first, then the separately opt-in LRCLIB fallback.
Successful results stay in memory. [Lyrics sources](docs/LYRICS_SOURCES.md)
explains what each provider receives.

### Jamkin companions

JamBun, JamPam, and JamJoe are project-created, locally bundled companions. Any
of the three can appear beside lyrics, become the app's launcher tile, or live
on the desktop as a small animated pet.

<table>
  <tr>
    <td width="33%" align="center">
      <img src="docs/screenshots/jamkins/jambun.gif" width="240" alt="JamBun dancing in a cheerful head-nod loop">
    </td>
    <td width="33%" align="center">
      <img src="docs/screenshots/jamkins/jampam.gif" width="240" alt="JamPam dancing in a graceful side-to-side loop">
    </td>
    <td width="33%" align="center">
      <img src="docs/screenshots/jamkins/jamjoe.gif" width="240" alt="JamJoe moving in a relaxed bass-nod loop">
    </td>
  </tr>
  <tr>
    <td align="center"><strong>JamBun</strong></td>
    <td align="center"><strong>JamPam</strong></td>
    <td align="center"><strong>JamJoe</strong></td>
  </tr>
</table>

The optional Desktop Jamkin dances during playback, shows lyrics on hover, and
remembers its position and size. Controls cover opacity, reduced motion,
quality, persistence, always-on-top behavior on compatible Wayland desktops,
and periodic movement for OLED care. Jamkin features are local and account-free.

### Album-aware glass

The player, navigation, and controls derive their palette from the current
cover art. Blur and transparency are adjustable, while the selected Jamkin
provides a consistent accent palette. Playlist artwork that Apple does not
supply is composed locally from the playlist's album covers. Jamelade follows
the desktop's normal interface font and does not bundle or select Apple's SF Pro.

### Optional Discord Rich Presence

Discord activity is off by default. If enabled, Jamelade can show the current
song, artist, album, and listening Jamkin through the local Discord or Vesktop
desktop client. It needs no Discord token and never sends lyrics, playlist
names, Apple credentials, or artwork URLs.

### A smaller, hardened web boundary

Apple Music playback on Linux still needs Chromium and Widevine. Jamelade keeps
that machinery in a constrained, normally hidden sidecar while the interface,
library, search, queue, artwork, lyrics, and desktop integration remain native.

The sidecar restricts navigation, permissions, downloads, and IPC. It disables
the persistent Chromium HTTP cache, sanitizes diagnostics, and stores Apple
cookies only in an OS-keyring-encrypted vault. Without a supported keyring, the
session is memory-only. Optional integrations never receive Apple credentials.

## Privacy at a glance

| Capability | Default | What can leave the device |
| --- | --- | --- |
| Apple Music | Required for playback | Sign-in, library, catalogue, playback, and first-party lyric requests go to Apple; lyrics add only the playing catalog ID |
| LRCLIB lyrics | Off | Track title, artist, album, duration, and the requester's IP address |
| Discord activity | Off | Selected song metadata and Jamkin are handed to the local Discord client; Discord then applies its own privacy settings |
| Desktop Jamkin | Off | Nothing extra; it reuses local playback and lyric state |
| Glass palette and playlist collages | Local | Nothing; both are generated and cached on the device |

Turning a network feature off stops its requests. Jamelade has no analytics,
advertising SDK, or Jamelade account.

## Install

Jamelade's public beta is distributed as an **x86_64 Flatpak**. Download the
bundle and `SHA256SUMS` from the
[beta release](https://github.com/Jamelade/jamelade/releases/tag/v0.10.0-beta.1),
then run:

```bash
sha256sum --ignore-missing -c SHA256SUMS
flatpak install --user ./Jamelade-0.10.0-beta.1-x86_64.flatpak
```

The bundle records Flathub as the source for its GNOME runtime. Ubuntu and
Debian users may need to install Flatpak first. Launch **Jamelade** from the
desktop's app grid after installation.

The first installation may fetch the GNOME 49 runtime. Building also downloads
the roughly 200 MB castLabs Electron playback sidecar; the release bundle
already contains that sidecar, while Widevine downloads on first use.

To build from source instead, the project needs Rust 1.93 or newer,
GTK 4.20 or newer, libadwaita 1.8 or newer, Node.js, npm, and the development
packages listed by your distribution:

```bash
make flatpak-bundle
```

### Requirements

- An active Apple Music subscription
- An x86_64 Linux system
- A working desktop keyring for persistent sign-in
- A network connection during playback

## Platform limitations

- **No offline playback.** Linux Widevine does not provide the persistent
  licences Apple Music downloads require.
- **x86_64 only.** A compatible ARM Widevine CDM is not available on Linux.
- **A Chromium sidecar is unavoidable.** WebKitGTK and GStreamer cannot decode
  Apple Music's protected streams.
- **Apple can change the service.** Jamelade relies on Apple's MusicKit web
  playback surface and public-facing catalogue APIs, which can change without
  notice.
- **The Apple integration is unofficial.** Jamelade loads `music.apple.com`,
  reads the developer token used by Apple's site, and reuses it for native API
  requests. This is not Apple's documented MusicKit integration model, which
  expects a developer-owned media identifier and signing key. Apple's current
  service terms also restrict access to Apple software. Treat the app as an
  experimental compatibility client, review Apple's terms for your region, and
  expect Apple to block it without notice.
- **Some library entries may be unavailable.** Apple can leave a delisted track
  in a library even though it can no longer be streamed.

## How it works

Rust and GTK own the interface, Apple API access, lyrics, and desktop
integration. A constrained castLabs Electron sidecar handles Apple sign-in,
MusicKit playback, Widevine, and audio. They communicate through a bounded
line-based protocol; Jamelade does not remove DRM or download decrypted music.

## Development

The main process, sidecar boundary, and project invariants are described in
[ARCHITECTURE.md](ARCHITECTURE.md).

```bash
cargo run
make sidecar-run
cargo clippy --all-targets -- -D warnings
make check
```

## Contributing

Contributions are welcome; read [CONTRIBUTING.md](CONTRIBUTING.md) first. Never
attach Apple cookies, account data, unredacted logs, or copyrighted Apple assets
to an issue. Report vulnerabilities through [SECURITY.md](SECURITY.md).

## Credits

- [Slipmat](https://github.com/SoftARV/Slipmat), created by Miguel Rincon, is
  the foundation of Jamelade. If the underlying player earns a place on your
  desktop, you can [buy Slipmat Creator a
  coffee](https://ko-fi.com/miguelrincon).
- **GPT-5.6 Sol**, working through OpenAI Codex, served as an AI development
  collaborator on substantial implementation, security and privacy review,
  testing, packaging, interface work, and documentation.
- **Anthropic Claude** provided additional AI development assistance during
  the project's earlier work.
- [Sidra](https://github.com/wimpysworld/sidra) and
  [Cider](https://cider.sh) established important prior art for Apple Music
  playback through castLabs Electron on Linux.

AI systems are credited for their assistance; project contributors remain
responsible for the published code.

## Legal and licence

Jamelade is unofficial and is not affiliated with, endorsed by, or approved by
Apple Inc. Service names are used only to describe compatibility. Apple and
Apple Music are trademarks of Apple Inc., registered in the U.S. and other
countries. See [LEGAL.md](LEGAL.md) and [PRIVACY.md](PRIVACY.md).

Jamelade is free software under **GPL-3.0-or-later**. See
[COPYING](COPYING). Slipmat's copyright notices and attribution remain
preserved; [UPSTREAM.md](UPSTREAM.md) identifies the exact upstream baseline
and links to its complete history.
