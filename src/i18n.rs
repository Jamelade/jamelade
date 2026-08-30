// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Built-in interface localization with an English fallback.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    System,
    English,
    German,
}

impl Language {
    pub const ALL: [Self; 3] = [Self::System, Self::English, Self::German];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::English => "en",
            Self::German => "de",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "en" => Self::English,
            "de" => Self::German,
            _ => Self::System,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "System Default",
            Self::English => "English",
            Self::German => "Deutsch",
        }
    }

    pub fn from_index(index: u32) -> Self {
        Self::ALL.get(index as usize).copied().unwrap_or_default()
    }

    pub fn index(self) -> u32 {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap_or_default() as u32
    }
}

const ENGLISH: u8 = 0;
const GERMAN: u8 = 1;
static ACTIVE: AtomicU8 = AtomicU8::new(ENGLISH);

pub fn set_language(language: Language) {
    ACTIVE.store(
        match language {
            Language::English => ENGLISH,
            Language::German => GERMAN,
            Language::System => detect_system_language(),
        },
        Ordering::Relaxed,
    );
}

pub fn apple_lyrics_localization() -> (&'static str, &'static str) {
    if ACTIVE.load(Ordering::Relaxed) == GERMAN {
        ("de-DE", "de-Latn")
    } else {
        ("en-US", "en-Latn")
    }
}

/// Locale for Apple catalogue text that should follow Jamelade's interface.
///
/// Storefront chooses availability, not display language. Passing this as
/// Apple's documented `l` query keeps credit headings from silently following
/// the operating-system locale when Jamelade is explicitly set to English.
pub fn apple_music_localization() -> &'static str {
    if ACTIVE.load(Ordering::Relaxed) == GERMAN {
        "de-DE"
    } else {
        "en-US"
    }
}

fn detect_system_language() -> u8 {
    for name in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        let Ok(value) = std::env::var(name) else {
            continue;
        };
        for locale in value.split(':') {
            if let Some(language) = language_code(locale) {
                return language;
            }
        }
    }
    ENGLISH
}

fn language_code(locale: &str) -> Option<u8> {
    let code = locale
        .trim()
        .split(['_', '-', '.', '@'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match code.as_str() {
        "de" => Some(GERMAN),
        "en" | "c" | "posix" => Some(ENGLISH),
        _ => None,
    }
}

pub fn tr(english: &'static str) -> &'static str {
    if ACTIVE.load(Ordering::Relaxed) != GERMAN {
        return english;
    }
    match english {
        "Explore" => "Entdecken",
        "Search" => "Suchen",
        "Lyrics" => "Songtexte",
        "Songs" => "Titel",
        "Albums" => "Alben",
        "Artists" => "Künstler:innen",
        "Playlists" => "Playlists",
        "All" => "Alle",
        "Everything" => "Alles",
        "Discover" => "Entdecken",
        "Library" => "Mediathek",
        "Pin a playlist" => "Playlist anheften",
        "Title" => "Titel",
        "Artist" => "Künstler:in",
        "Album" => "Album",
        "Year" => "Jahr",
        "Recently Added" => "Zuletzt hinzugefügt",
        "Recently Updated" => "Zuletzt aktualisiert",
        "Appearance" => "Darstellung",
        "Language" => "Sprache",
        "Theme" => "Design",
        "Accent Colour" => "Akzentfarbe",
        "Album Liquid Glass" => "Album-Liquid-Glass",
        "Transparency &amp; Blur" => "Transparenz &amp; Unschärfe",
        "Nearby Lyric Colour" => "Farbe naher Songtextzeilen",
        "Lyric Text Size" => "Songtextgröße",
        "Jamkin Companion" => "Jamkin-Begleiter",
        "Companion" => "Begleiter",
        "Jamkin Image Quality" => "Jamkin-Bildqualität",
        "Reduce Jamkin Motion" => "Jamkin-Bewegung reduzieren",
        "App Icon" => "App-Symbol",
        "Desktop Jamkin" => "Desktop-Jamkin",
        "Keep Jamkin When Window Closes" => "Jamkin beim Schließen behalten",
        "Desktop Jamkin Size" => "Größe des Desktop-Jamkins",
        "Desktop Jamkin Opacity" => "Deckkraft des Desktop-Jamkins",
        "Keep Jamkin Above Other Windows" => "Jamkin über anderen Fenstern halten",
        "Edge Walk" => "Randwanderung",
        "Notifications" => "Benachrichtigungen",
        "Notify on track change" => "Bei Titelwechsel benachrichtigen",
        "Connections" => "Verbindungen",
        "Discord Activity" => "Discord-Aktivität",
        "Global Shortcuts" => "Globale Tastenkürzel",
        "Configure…" => "Einrichten…",
        "Reconfigure…" => "Neu einrichten…",
        "ListenBrainz Scrobbling" => "ListenBrainz-Scrobbling",
        "Disable" => "Deaktivieren",
        "Set up…" => "Einrichten…",
        "Lyrics privacy" => "Songtext-Datenschutz",
        "Lyrics from Apple Music" => "Songtexte von Apple Music",
        "Fallback lyrics from LRCLIB" => "Ersatz-Songtexte von LRCLIB",
        "Fallback lyrics from Lyrics.ovh" => "Ersatz-Songtexte von Lyrics.ovh",
        "Original" => "Original",
        "Translation" => "Übersetzung",
        "Romanized" => "Romanisiert",
        "Song Credits" => "Titel-Credits",
        "No credits supplied" => "Keine Credits angegeben",
        "Apple Music did not return credits for this recording." => {
            "Apple Music hat für diese Aufnahme keine Credits geliefert."
        }
        "Nothing playing" => "Keine Wiedergabe",
        "No lyrics found" => "Keine Songtexte gefunden",
        "Instrumental" => "Instrumental",
        "Finding lyrics" => "Songtexte werden gesucht",
        "Toggle Sidebar" => "Seitenleiste umschalten",
        "Settings and App Menu" => "Einstellungen und App-Menü",
        "Search Apple Music" => "Apple Music durchsuchen",
        "Recent Searches" => "Letzte Suchanfragen",
        "Clear History" => "Verlauf löschen",
        "Browse Categories" => "Kategorien durchsuchen",
        "Trending Now" => "Im Trend",
        "Remember Search History" => "Suchverlauf speichern",
        "New Music" => "Neue Musik",
        "Hip-Hop" => "Hip-Hop",
        "Indie" => "Indie",
        "Electronic" => "Elektronisch",
        "Chill" => "Entspannen",
        "No matches" => "Keine Treffer",
        "New Playlist" => "Neue Playlist",
        "_Preferences" => "_Einstellungen",
        "_Show Jamkin" => "_Jamkin anzeigen",
        "_New Playlist…" => "_Neue Playlist…",
        "_Buy Slipmat Creator a Coffee" => "_Slipmat-Entwickler einen Kaffee ausgeben",
        "_Keyboard Shortcuts" => "_Tastenkürzel",
        "_About Jamelade" => "_Über Jamelade",
        "_Sign Out" => "_Abmelden",
        "_Quit" => "_Beenden",
        "Reload" => "Neu laden",
        "Sort" => "Sortieren",
        "What to search for" => "Suchkategorie",
        _ => english,
    }
}

pub fn hide_jamkin(name: &str) -> String {
    if ACTIVE.load(Ordering::Relaxed) == GERMAN {
        format!("{name} ausblenden")
    } else {
        format!("Hide {name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_codes_are_bounded_and_predictable() {
        assert_eq!(language_code("de_DE.UTF-8"), Some(GERMAN));
        assert_eq!(language_code("en-US"), Some(ENGLISH));
        assert_eq!(language_code("ja_JP.UTF-8"), None);
    }
}
