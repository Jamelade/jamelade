<!--
SPDX-FileCopyrightText: 2026 Jamelade contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Lyrics sources

Jamelade tries sources sequentially and stops at the first usable result.
Successful or empty results stay in a bounded memory cache and are never
written as listening history.

| Source | Default | Data sent |
| --- | --- | --- |
| Apple Music | On with the Apple connection | Numeric catalog song ID through MusicKit's authenticated browser client |
| LRCLIB | Off | Title, artist, album, duration, and IP address |
| Lyrics.ovh | Off | Artist, title, and IP address; the service may query downstream lyric sites |

Apple-provided translations and romanizations appear as selectable variants
when present in the same bounded first-party response. Jamelade does not call a
translation service. A manual timing correction is stored locally as a numeric
catalogue ID and offset; lyric text, title, and artist are not written.

Each third-party source requires its own consent. Its HTTP client has no Apple
header, cookie, token, account ID, or redirect permission. Apple JSON is capped
at 2 MiB; third-party JSON is capped at 256 KiB and requires a JSON content
type. Apple TTML is parsed as bounded streaming XML with DTDs rejected.

Changing provider consent or signing out clears the in-memory cache. Jamelade
does not use scraped service tokens, unofficial account APIs, certificate
bypasses, or silent aggregator fallbacks.

Lyrics remain copyrighted material supplied by their respective service and
rightsholders; Jamelade does not claim ownership or relicensing rights.
