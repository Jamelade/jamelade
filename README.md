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
rich artist pages, album-aware glass styling, optional Discord activity, and
three project-created Jamkin companions—**JamBun**, **JamPam**, and **JamJoe**.

Jamelade is not affiliated with or endorsed by Apple Inc. or the upstream
Slipmat project. An active Apple Music subscription is required.

## What Jamelade brings to Slipmat

### Apple Music Home, made native

Explore presents the recommendation groups Apple exposes to the signed-in
account, alongside recent listening, heavy rotation, radio, and storefront
charts. The shelves are native widgets rather than an embedded copy of Apple's
website.

### Rich artist pages

Artist pages combine the latest release, top songs, albums, and the biography
Apple supplies at runtime. Select the artist portrait to open or close the
biography without leaving the page.

### Search that starts useful

An empty Search page shows up to 16 recent device-local queries, five quick
category shortcuts, and a **Trending Now** row from Apple's documented
storefront chart. Apple does not expose trending query strings, so Jamelade
does not scrape or mislabel them. Search recording can be switched off without
deleting existing entries; each pill, its right-click action, and **Clear
History** remove only local state.

### Live lyrics with privacy-first fallbacks

Jamelade follows synchronized lyrics while keeping the selected Jamkin visible.
It tries Apple Music first. LRCLIB and Lyrics.ovh are independent, off-by-
default fallbacks; Jamelade contacts only sources the user has enabled. An
enabled LRCLIB may upgrade plain Apple lyrics to a verified synchronized match;
Lyrics.ovh is used only when earlier sources have no text. Results stay in memory.
When Apple supplies a translation or romanization, it appears as a selectable
variant. If LRCLIB supplies the missing clock and its original lines exactly
match Apple's, the Apple variants inherit that clock. A per-song timing offset
can correct early or late synchronized lyrics without storing titles or artists.
[Lyrics sources](docs/LYRICS_SOURCES.md) explains what each provider receives.

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

The Desktop Jamkin dances during playback, keeps lyrics visible while hovered,
and remembers its position and size. Controls cover opacity, reduced motion,
quality, persistence, always-on-top behavior on compatible Wayland desktops,
and **Edge Walk**, which periodically moves it around screen edges for OLED
care. Right-clicking the active companion hides it by name; the main app menu's
**Show Jamkin** toggle restores it without changing its identity or position.
Fresh installs start with JamBun at 175 px; saved preferences always win on
later launches. Jamkin features are local and account-free.

### Album-aware glass

The main window can derive its palette from the current cover art. Blur and
transparency are adjustable, while the selected Jamkin provides a consistent
accent palette. Blossom, Tidepool, and Vermilion use richer named-theme
surfaces. The compact and expanded players keep the selected theme as stable,
readable surfaces. Playlist artwork that Apple does not supply is
composed locally from the playlist's album covers. Jamelade uses SF Pro Display
when it is already installed and otherwise falls back to the desktop's normal
sans-serif font. It does not bundle Apple's font.

### Native navigation, links, and focused looping

Artist and album names in the expanded player open their native Jamelade
pages. Song, album, and playlist menus can copy or share public Apple Music
links without exposing an Apple session. The expanded player also has a
process-local **A–B** control for repeating a chosen section; it does not alter
the normal queue repeat mode or write loop points to disk. See
[A–B loop](docs/AB_LOOP.md).

### Playlist and playback tools

Jamelade can create an Apple Music playlist, add a song to an existing one,
and export a playlist as M3U8, CSV, or JSON. Exports contain visible metadata
and public Apple Music links, not account or library identifiers. The expanded
player also includes a sleep timer, a bounded 0.5×–2× MusicKit playback-speed
slider in 0.1× steps, and song credits. Optional global shortcuts use the
desktop portal, so the desktop—not Jamelade—owns the chosen keys.

Apple's documented API exposes playlist creation and appending, but not safe
rename, removal, or reordering operations. Jamelade does not guess at
undocumented destructive endpoints. The add dialog includes only playlists
Apple explicitly marks editable; saved editorial playlists remain browse-only.
Jamelade checks the chosen playlist and asks before appending a duplicate song.

### Optional Discord Rich Presence

Discord activity is off by default. If enabled, Jamelade can show the current
song, artist, album, and listening Jamkin through the local Discord or Vesktop
desktop client. It needs no Discord token and never sends lyrics, playlist
names, Apple credentials, or artwork URLs.

### Optional ListenBrainz scrobbling

ListenBrainz scrobbling is off by default. If enabled, Jamelade submits title,
artist, album, duration, and listen time after the normal scrobble threshold.
Its token is encrypted with a per-app key from the desktop keyring; Apple
identifiers, credentials, artwork, and lyrics are never included.

The interface follows the system language by default and currently includes
English and German. A language can also be selected in Preferences.

### A smaller, hardened web boundary

Apple Music playback on Linux still needs Chromium and Widevine. Jamelade keeps
that machinery in a constrained, normally hidden sidecar while the interface,
library, search, queue, artwork, lyrics, and desktop integration remain native.

The sidecar restricts navigation, permissions, downloads, and IPC. It disables
the persistent Chromium HTTP cache, sanitizes diagnostics, and stores Apple
cookies only in an OS-keyring-encrypted vault. Without a supported keyring, the
session is memory-only. Startup gives an existing vault a bounded keyring retry;
if decryption still fails, Jamelade preserves the encrypted file and disables
writes instead of replacing it with anonymous cookies. Optional integrations
never receive Apple credentials.

Jamelade can also remember the last Apple ID email entered in Apple's login
window. It is stored separately, encrypted by the same desktop keyring, and
prefilled by a bounded main-process helper only into fixed email/username-like
fields in a validated HTTPS Apple frame. The helper waits for a complete,
stable address and exists only for the visible login flow. The password field
is never queried, read or stored, and the empty auth preload receives no value
or IPC capability.

## Privacy at a glance

| Capability | Default | What can leave the device |
| --- | --- | --- |
| Apple Music | Required for playback | Sign-in, library, catalogue, playback, and first-party lyric requests go to Apple; lyrics add only the playing catalog ID |
| LRCLIB lyrics | Off | Track title, artist, album, duration, and the requester's IP address |
| Lyrics.ovh lyrics | Off | Artist, title, and the requester's IP address; the service may query downstream lyric sites |
| Discord activity | Off | Selected song metadata and Jamkin are handed to the local Discord client; Discord then applies its own privacy settings |
| ListenBrainz | Off | Title, artist, album, duration, listen time, and the requester's IP address |
| Global shortcuts | Off | Nothing leaves the device; the desktop portal stores the selected bindings |
| Search history | On | Nothing extra; up to 16 queries are stored in a private local state file, and Apple receives a query only when it is searched |
| Desktop Jamkin | On | Nothing extra; it reuses local playback and lyric state |
| Glass palette and playlist collages | Local | Nothing; both are generated and cached on the device |

Turning a network feature off stops its requests. Jamelade has no analytics,
advertising SDK, or Jamelade account.

### Optional launcher-icon helper

The app can use the desktop's standard confirmation portal to change its
Jamkin launcher tile. For a direct change without creating a second launcher,
an optional per-user helper can be installed separately from a source checkout:

```bash
./scripts/icon-helper.sh install
```

It accepts only `JamBun`, `JamPam`, or `JamJoe`, writes only Jamelade's fixed
launcher entry, and has no network access. Remove it with
`./scripts/icon-helper.sh uninstall`; the portal remains available without it.

## Install

Jamelade release packages are available on the
[releases page](https://github.com/RizziU/jamelade/releases). **Flatpak is
recommended** because it adds an outer application sandbox:

```bash
sha256sum --ignore-missing -c SHA256SUMS
flatpak install --user ./Jamelade-3.0.0-x86_64.flatpak
```

The secondary AppImage targets Ubuntu 24.04 or newer and contemporary Fedora:

```bash
sha256sum --ignore-missing -c SHA256SUMS
chmod +x Jamelade-3.0.0-x86_64.AppImage
./Jamelade-3.0.0-x86_64.AppImage
```

The AppImage keeps Chromium's renderer sandbox enabled but, like any native
executable, is not itself confined from the user's files. It deliberately
fails instead of adding a sandbox-disabling fallback on incompatible hosts.
See [`packaging/appimage/README.md`](packaging/appimage/README.md).

To build the Flatpak from source, install the tools and runtimes in
[`packaging/flatpak/README.md`](packaging/flatpak/README.md), then run
`make flatpak-bundle`. Both packages download Widevine through Chromium on
first use; Widevine is absent from the source and release artifacts.

### Requirements

- An active Apple Music subscription
- An x86_64 Linux system
- A working desktop keyring for persistent sign-in
- A network connection during playback

## Platform limitations

- **No offline playback.** Linux Widevine does not provide the persistent
  licences Apple Music downloads require.
- **x86_64 only.** A compatible ARM Widevine CDM is not available on Linux.
- **AppImage has no outer sandbox.** It inherits the invoking user's normal
  filesystem permissions. Use Flatpak when application confinement matters.
- **A Chromium sidecar is unavoidable.** WebKitGTK and GStreamer cannot decode
  Apple Music's protected streams.
- **Apple can change the service.** Jamelade relies on Apple's MusicKit web
  playback surface and public-facing catalogue APIs, which can change without
  notice.
- **The Apple integration is unofficial.** Jamelade loads `music.apple.com`
  and asks MusicKit's authenticated browser client to perform a narrow set of
  Apple API requests. Rust never receives Apple cookies or MusicKit tokens.
  This is still not Apple's documented third-party MusicKit integration model.
  Review Apple's terms for your region and expect Apple to change or block the
  service without notice.
- **Some library entries may be unavailable.** Apple can leave a delisted track
  in a library even though it can no longer be streamed.

## How it works

Rust and GTK own the interface, typed data, lyrics, and desktop integration. A
constrained castLabs Electron sidecar owns Apple sign-in, authenticated
MusicKit API transport, playback, Widevine, and audio. Rust sends only
allowlisted relative Apple paths and receives bounded responses over a typed
line protocol; Jamelade does not remove DRM or download decrypted music.

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
