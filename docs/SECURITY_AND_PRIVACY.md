<!--
SPDX-FileCopyrightText: 2026 Jamelade contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Security and privacy

Jamelade is an unofficial Apple Music client. It has no telemetry, ads,
Jamelade account, remote configuration, or silent application updater. Apple
playback still requires a networked Chromium/Widevine boundary.

## Package isolation

The Flatpak is the recommended package because its manifest adds an outer
application sandbox. The AppImage runs as an ordinary native user process: the
same network allowlists, credential boundary, bounded storage, and Chromium
renderer sandbox still apply, but the host does not confine its filesystem
access. The AppImage never supplies a Chromium sandbox-disabling flag and fails
closed on hosts that prohibit Chromium's available sandbox mechanisms.

### Flatpak permissions

| Permission | Purpose |
| --- | --- |
| Network | Apple sign-in, APIs, playback, artwork, Widevine updates, and enabled lyric providers |
| Wayland/fallback X11, PulseAudio, DRI, shared IPC | Display, audio, accelerated GTK, and Electron sandbox integration |
| `org.freedesktop.secrets` | Encrypt persisted Apple cookies; without it the session is memory-only |
| MPRIS and Jamelade launcher names | Desktop media controls and the portal-managed launcher |
| `io.github.Jamelade.IconHelper` | Optional three-choice same-user launcher helper |
| Three narrow runtime paths | Local Discord/Vesktop IPC only after Rich Presence is enabled |

There is no general home-folder, Documents, Downloads, removable-media, SSH,
browser-profile, microphone, camera, location, screen-capture, input-monitoring,
or host-command permission. Inspect the installed grant with:

```bash
flatpak info --show-permissions io.github.Jamelade.Jamelade
```

## Network destinations

- Apple domains (`apple.com`, `applemusic.com`, `mzstatic.com`,
  `cdn-apple.com`, `icloud.com`, `icloud-content.com`, `apple-dns.net`, and
  `itunes.com`) provide sign-in, MusicKit, APIs, artwork, and audio. The
  credential-bearing browser accepts only validated HTTPS/WSS Apple origins.
- `clients2.google.com` is Chromium's Widevine component updater. Jamelade does
  not send it listening metadata.
- `lrclib.net` receives title, artist, album, duration, and IP address only
  after its separate opt-in.
- `api.lyrics.ovh` receives artist, title, and IP address only after its
  separate opt-in; its server may query other lyric sites.
- Discord activity goes through a same-user local socket only after opt-in. It
  includes song, artist, album, and selected Jamkin—not lyrics, Apple
  credentials, playlist names, or artwork URLs.

Opening project links is an explicit user action in the default browser.

## Browser broker

The native process owns the interface and local model. A constrained castLabs
Electron sidecar owns Apple sign-in, MusicKit API transport, protected
playback, Chromium, and Widevine. Rust supplies only allowlisted relative Apple
paths and receives bounded JSON; it cannot supply origins, headers, cookies,
tokens, or arbitrary URLs.

Renderer windows use sandboxing, context isolation, no Node.js, no webviews,
no release developer tools, no downloads, and deny web permissions. Navigation
and popups are restricted to validated Apple destinations. Both bridge layers
enforce exact event schemas and bounded payloads. Chromium's persistent HTTP
cache is disabled.

The separate public artist-biography fallback has no cookie jar or token. It
accepts only a numeric artist ID, validates one canonical Apple redirect, caps
the response, extracts bounded plain text, and creates no disk cache.

## Local storage

Flatpak state lives below `~/.var/app/io.github.Jamelade.Jamelade/`. The
AppImage follows the normal XDG config and cache roots instead.

| Data | Handling |
| --- | --- |
| Apple session | Validated secure Apple cookies only, encrypted through the desktop keyring, mode 0600 inside a mode-0700 directory |
| Apple tokens | Remain inside Apple/MusicKit; Rust never stores them |
| Settings and queue | Bounded user-only files |
| Library metadata and artwork | App-private bounded caches; artwork filenames do not contain titles |
| Lyrics | Bounded process-memory cache only |
| Widevine | Chromium component files in the private app directory; absent from source and packages |

Explicit sign-out stops vault writes, clears cookies/web storage and
account-derived caches, removes the vault, and clears Discord activity.

## Logging and updates

Normal logs contain fixed states, counts, timings, and status codes—not raw
protocol lines, tokens, cookies, lyric URLs, artwork URLs, titles, playlist
names, account IDs, or local paths. Debug logs may still reveal timing and
listening behavior and must be reviewed before sharing.

Widevine can update through Chromium. The Chromium/Electron engine is pinned by
version, URL, and SHA-256 in each Jamelade build. A weekly workflow may open a
dependency PR, but it cannot merge, sign, publish, or install it; applying an
engine update requires a reviewed Jamelade rebuild.

## Limits

The Apple page, Apple APIs, Electron, Chromium, Widevine, lyrics providers,
Discord, and the keyring remain external dependencies. Flatpak's network grant
is host-wide, so hostname restrictions are enforced in code. A compromised app
process can observe data it legitimately needs while running. This design
reduces persistence and cross-component access but is not a hardware security
boundary or an independent audit.

Report controllable vulnerabilities through [SECURITY.md](../SECURITY.md), not
a public issue.
