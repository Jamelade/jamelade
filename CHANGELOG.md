# Changelog

## Unreleased

- Added an optional bounded local catalogue-search history with recent-query
  reuse, right-click removal, a full-history wipe button, and a recording toggle
  that deliberately preserves existing entries when switched off.

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
