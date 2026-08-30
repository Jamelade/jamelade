# Changelog

## Unreleased

- Added a compact 0.5×–2× playback-speed slider in 0.1× steps behind the
  far-right rate button while preserving the centred play button and clocks.
- Refined visual hierarchy with stronger secondary-text contrast, distinct
  value/toggle/slider preference rows, an inset compact-player progress track,
  a quieter search field, and roomier glass-backed media-grid hover states.
- Limited Add to Playlist to library playlists Apple explicitly marks editable,
  themed the chooser consistently, fixed empty successful MusicKit responses,
  warns before adding a duplicate, and refreshes the affected open playlist
  after Apple accepts an append.
- Retuned JamJoe's accent from muted brown to the lively, accessible burnt
  orange used in his own artwork.
- Replaced the empty Search prompt with local recent-search pills, per-entry and
  clear-all removal, five category shortcuts, and a storefront Trending Now row
  backed by Apple's documented chart. Recording is optional and disabling it
  preserves existing history.
- Increased the colour presence of the Blossom, Tidepool, and Vermilion named
  themes while retaining their existing light-surface contrast.
- Added `Hide JamBun`, `Hide JamPam`, or `Hide JamJoe` to the active Desktop
  Jamkin's right-click menu, plus a synchronized `Show Jamkin` toggle in the
  primary app menu. Hiding preserves the selected companion and placement.
- Added bounded keyring-readiness and cookie-vault restore retries. A failed
  restore now preserves the encrypted session and disables anonymous-cookie
  writes until an explicit Apple login supersedes it.
- Added keyring-encrypted storage for the previous Apple ID email. A bounded
  main-process helper can observe and prefill fixed email/username-like fields
  only during a visible login in validated Apple frames. It waits for a
  complete, stable address instead of persisting a typing prefix; the auth
  preload remains empty and passwords stay outside Jamelade's storage and IPC.

## 2.1.0 — 2026-08-25

- Replaced the heavy four-sided adaptive text outline with a softer,
  luminance-aware halo over fully exposed album artwork.
- Kept sidebar typography on the selected theme instead of inheriting the
  artwork foreground treatment.

## 2.0.0 — 2026-08-25

- Added playlist creation and song additions, credits, sleep and A–B timers,
  global shortcuts, optional ListenBrainz scrobbling, and richer artist pages.
- Added named colour themes and refined the compact and expanded players.
- Added Apple lyric translations and romanization, per-track timing adjustment,
  and strictly verified synchronization from an enabled LRCLIB fallback.
- Hardened MusicKit request recovery and kept optional services separated from
  Apple credentials.
- Verified both app profiles, the browser broker, private storage, dependency
  advisories, and the narrowly scoped launcher-icon helper before packaging.
- Fixed song-credit localization to follow Jamelade's interface language and
  made About display the currently selected Jamkin tile.
- Added a bounded automatic sidecar restart when Chromium's Widevine component
  updater stalls during a cold launch.

## 1.0.0 — 2026-08-25

- First stable Jamelade release.
