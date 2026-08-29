# Changelog

## Unreleased

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
