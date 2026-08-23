// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! Persistent **preferences**, in `~/.config/jamelade/settings.ini`.
//!
//! Preferences only. Apple cookies live in an encrypted keyring-backed vault
//! and tokens are harvested into memory — nothing secret goes in this file. If you
//! find yourself adding a field whose value would be embarrassing in a
//! plain-text file under `~/.config`, it belongs somewhere else.
//!
//! A missing or corrupt file is not an error: it means defaults. This is a
//! single-user app on one machine, and refusing to start because an ini file
//! got mangled would be absurd.

use relm4::gtk::glib::{self, KeyFile, KeyFileFlags};

use crate::companion::Companion;

mod jamkin_quality;
pub use jamkin_quality::JamkinQuality;

const GROUP: &str = "Jamelade";

/// The first-run balance between a quiet native surface and visibly liquid
/// album glass. Kept as a percentage so it remains stable if the rendering
/// formula evolves later.
pub const DEFAULT_GLASS_STRENGTH: u8 = 75;
/// How strongly the previous and upcoming lyric lines borrow the selected
/// Jamkin accent. Kept separate from glass strength so testing legibility does
/// not unexpectedly change the rest of the window.
pub const DEFAULT_LYRICS_ACCENT_STRENGTH: u8 = 75;
pub const DEFAULT_DESKTOP_JAMKIN_SIZE: u16 = 175;
pub const MIN_DESKTOP_JAMKIN_SIZE: u16 = 72;
pub const MAX_DESKTOP_JAMKIN_SIZE: u16 = 384;
pub const DEFAULT_DESKTOP_JAMKIN_OPACITY: u8 = 100;
pub const MIN_DESKTOP_JAMKIN_OPACITY: u8 = 30;
pub const MAX_DESKTOP_JAMKIN_OPACITY: u8 = 100;
pub const DEFAULT_LYRICS_FONT_SCALE: u8 = 125;
pub const MIN_LYRICS_FONT_SCALE: u8 = 80;
pub const MAX_LYRICS_FONT_SCALE: u8 = 160;
pub const DEFAULT_DESKTOP_JAMKIN_MARGIN: i32 = 24;
pub const MAX_DESKTOP_JAMKIN_MARGIN: i32 = 32_768;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    /// Index in the Preferences combo row, and back.
    pub fn from_index(i: u32) -> Self {
        match i {
            1 => Self::Light,
            2 => Self::Dark,
            _ => Self::System,
        }
    }

    pub fn index(self) -> u32 {
        match self {
            Self::System => 0,
            Self::Light => 1,
            Self::Dark => 2,
        }
    }
}

/// Which sidebar section the app opens on. Persisted, so it reopens where you
/// left it rather than always on the same one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Library,
    Explore,
    Lyrics,
    Albums,
    Artists,
    Playlists,
    Catalog,
}

impl Section {
    fn as_str(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Explore => "explore",
            Self::Lyrics => "lyrics",
            Self::Albums => "albums",
            Self::Artists => "artists",
            Self::Playlists => "playlists",
            Self::Catalog => "catalog",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "explore" => Self::Explore,
            "lyrics" => Self::Lyrics,
            "catalog" => Self::Catalog,
            "albums" => Self::Albums,
            "artists" => Self::Artists,
            "playlists" => Self::Playlists,
            _ => Self::Library,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub theme: Theme,
    /// Accent colour id; see `style::Accent`.
    pub accent: String,
    /// The local Jamkin shown beside lyrics and used by the matching palette.
    pub companion: Companion,
    /// Rounded Jamkin tile used by the desktop launcher. Independent of the
    /// singing companion so either can be changed without surprising the
    /// other.
    pub launcher_icon: Companion,
    /// Local bundled-art selection only. This never downloads a model or sends
    /// an image anywhere; both frame sets ship with the application.
    pub jamkin_quality: JamkinQuality,
    /// Show the selected Jamkin in its own small movable desktop window.
    /// On for a fresh install so the companion feature is immediately visible.
    /// A saved preference always takes precedence on later starts.
    pub desktop_jamkin: bool,
    /// Square pixel size of the optional desktop actor. It is ordinary UI
    /// state, not a screen coordinate, and is safe to persist.
    pub desktop_jamkin_size: u16,
    /// Sprite opacity only; the hover lyric bubble remains fully legible.
    pub desktop_jamkin_opacity: u8,
    /// Keep the independent companion surface visible while the main player
    /// window is hidden and playback continues in the background.
    pub desktop_jamkin_stay_visible: bool,
    /// Layer-shell placement, measured inward from the monitor's right and
    /// bottom edges. Local layout state only; clamped before use so a changed
    /// monitor cannot strand the companion off-screen.
    pub desktop_jamkin_right: i32,
    pub desktop_jamkin_bottom: i32,
    /// Ask a capable Wayland compositor to draw the desktop Jamkin above other
    /// windows. On for a fresh install and contains no private data.
    pub desktop_jamkin_above: bool,
    /// Legacy storage name for Edge Walk. Periodically move the compositor-
    /// overlay Jamkin to reduce the chance of marking an OLED panel.
    pub desktop_jamkin_oled_care: bool,
    /// Freeze decorative Jamkin frame animation and make Edge Walk instant.
    /// The desktop-wide GTK reduced-motion preference is always respected too.
    pub jamkin_reduced_motion: bool,
    /// How the Songs list is ordered. Stored as the id string, so an unknown
    /// value from a hand-edited or future ini falls back rather than breaking
    /// startup.
    pub sort: String,
    /// Whether the user flipped the sort's natural direction.
    pub sort_reversed: bool,
    /// The grids sort separately, because their keys differ from the songs
    /// list's — an album has a date added, a playlist has no artist, an artist
    /// has only a name.
    pub album_sort: String,
    pub album_sort_reversed: bool,
    pub artist_sort: String,
    pub artist_sort_reversed: bool,
    pub playlist_sort: String,
    pub playlist_sort_reversed: bool,
    pub section: Section,
    pub show_sidebar: bool,
    /// Whether the current cover is painted behind the whole window, blurred.
    /// On by default.
    pub player_backdrop: bool,
    /// Combined material transparency and artwork blur, from 0 (subtle) to 100
    /// (fully clear). It is presentation only and contains no private data.
    pub glass_strength: u8,
    /// Accent mix for the previous, next and following synchronized lyric
    /// lines, from 0 (neutral) to 100 (most colourful).
    pub lyrics_accent_strength: u8,
    /// Percentage applied to full-page and hover-bubble lyric text. Bounded so
    /// a hand-edited preference cannot create unusable geometry.
    pub lyrics_font_scale: u8,
    /// Notify when the track changes. Off by default (`bool`'s default).
    pub notify_track_change: bool,
    /// Share the visible current-track metadata with the local Discord client.
    /// Off by default because Discord can forward that activity to an account
    /// and its audience; this never implies consent merely from Discord being
    /// installed.
    pub discord_activity: bool,
    /// Permit metadata for the current track to be sent to LRCLIB after Apple
    /// Music has no useful lyric. Off by default: this is a third-party
    /// disclosure and requires an explicit opt in even though it never contains
    /// Apple credentials.
    pub lyrics_enabled: bool,
    /// Playlists pinned to the sidebar, in the order they were put there.
    ///
    /// **Library ids only** (`p.…`, as `/me/library/playlists` returns them).
    /// A pinned row is a playlist you own, and the two id spaces are not
    /// interchangeable — a catalog id 404s against `/me/library`.
    pub pinned_playlists: Vec<String>,
}

/// Separates pins in the ini. KeyFile's own list separator, so a hand-edited
/// file reads the way anyone would expect.
const PIN_SEP: char = ';';
const PIN_MAX_COUNT: usize = 256;
const PIN_MAX_ID_BYTES: usize = 512;

fn valid_pin(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= PIN_MAX_ID_BYTES
        && !id.contains(PIN_SEP)
        && !id.chars().any(char::is_control)
}

/// Split the stored pin list, dropping anything that cannot be a pin.
///
/// **Both directions are ours, deliberately.** glib 0.22 binds `string_list`
/// for reading but no `set_string_list` to pair with it, and writing through
/// `set_string` while reading through `string_list` would put an unescaped
/// write against an escaping read — which works right up until an id needs
/// escaping. Owning both sides costs a few lines and cannot drift.
///
/// Duplicates are dropped because two pins of one playlist are two identical
/// rows, and no click could tell them apart.
fn parse_pins(stored: &str) -> Vec<String> {
    let mut pins: Vec<String> = Vec::new();
    for id in stored.split(PIN_SEP) {
        let id = id.trim();
        if valid_pin(id) && !pins.iter().any(|seen| seen == id) {
            pins.push(id.to_owned());
            if pins.len() == PIN_MAX_COUNT {
                break;
            }
        }
    }
    pins
}

/// Join pins for storage, dropping any that would corrupt the format.
///
/// No real Apple library playlist id contains the separator — measured against
/// a real library — but an id that did would silently become two broken pins,
/// and losing one pin beats resurrecting two that point nowhere.
fn join_pins(pins: &[String]) -> String {
    let mut safe: Vec<&str> = Vec::new();
    for id in pins {
        let id = id.trim();
        if valid_pin(id) && !safe.contains(&id) {
            safe.push(id);
            if safe.len() == PIN_MAX_COUNT {
                break;
            }
        }
    }
    safe.join(&PIN_SEP.to_string())
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Light,
            // The selected Jamkin supplies the colour unless someone chooses a
            // conventional or system accent in Preferences.
            accent: "jamkin".into(),
            companion: Companion::default(),
            launcher_icon: Companion::default(),
            jamkin_quality: JamkinQuality::High,
            desktop_jamkin: true,
            desktop_jamkin_size: DEFAULT_DESKTOP_JAMKIN_SIZE,
            desktop_jamkin_opacity: DEFAULT_DESKTOP_JAMKIN_OPACITY,
            desktop_jamkin_stay_visible: true,
            desktop_jamkin_right: DEFAULT_DESKTOP_JAMKIN_MARGIN,
            desktop_jamkin_bottom: DEFAULT_DESKTOP_JAMKIN_MARGIN,
            desktop_jamkin_above: true,
            desktop_jamkin_oled_care: true,
            jamkin_reduced_motion: false,
            // Apple's own order.
            sort: "title".into(),
            sort_reversed: false,
            album_sort: "title".into(),
            album_sort_reversed: false,
            artist_sort: "title".into(),
            artist_sort_reversed: false,
            playlist_sort: "title".into(),
            playlist_sort_reversed: false,
            section: Section::default(),
            // Visible on a first run: a sidebar nobody has hidden yet should
            // be there to be found.
            show_sidebar: true,
            // Not `bool`'s default: the backdrop is the app's own look, and
            // #145 explicitly asked for it to stay on for everyone else.
            player_backdrop: true,
            glass_strength: DEFAULT_GLASS_STRENGTH,
            lyrics_accent_strength: DEFAULT_LYRICS_ACCENT_STRENGTH,
            lyrics_font_scale: DEFAULT_LYRICS_FONT_SCALE,
            notify_track_change: false,
            discord_activity: false,
            lyrics_enabled: false,
            // Nothing pinned until somebody pins something. An app that
            // guesses which playlists matter to you gets it wrong.
            pinned_playlists: Vec::new(),
        }
    }
}

fn path() -> Option<std::path::PathBuf> {
    let dir = glib::user_config_dir().join("jamelade");
    Some(dir.join("settings.ini"))
}

impl Settings {
    /// Read preferences, falling back to defaults for anything missing.
    pub fn load() -> Self {
        let Some(path) = path() else {
            return Self::default();
        };

        const SETTINGS_MAX_BYTES: usize = 64 * 1024;
        let Ok(data) = crate::private_storage::read_to_string(&path, SETTINGS_MAX_BYTES) else {
            return Self::default();
        };

        let settings = Self::from_data(&data);
        tracing::debug!("loaded settings");
        settings
    }

    /// Overlay explicitly stored values onto the current fresh-install
    /// defaults. Keeping this separate makes the "saved settings win" rule
    /// directly testable without touching a real user's configuration file.
    fn from_data(data: &str) -> Self {
        let mut settings = Self::default();

        let file = KeyFile::new();
        if file.load_from_data(data, KeyFileFlags::NONE).is_err() {
            // No file yet, or unreadable. Defaults, quietly — this is the
            // normal first-run path, not a failure.
            return settings;
        }

        if let Ok(theme) = file.string(GROUP, "theme") {
            settings.theme = Theme::parse(&theme);
        }
        if let Ok(notify) = file.boolean(GROUP, "notify-track-change") {
            settings.notify_track_change = notify;
        }
        if let Ok(enabled) = file.boolean(GROUP, "discord-activity") {
            settings.discord_activity = enabled;
        }
        if let Ok(enabled) = file.boolean(GROUP, "lyrics-enabled") {
            settings.lyrics_enabled = enabled;
        }
        if let Ok(section) = file.string(GROUP, "section") {
            settings.section = Section::parse(&section);
        }
        if let Ok(show) = file.boolean(GROUP, "show-sidebar") {
            settings.show_sidebar = show;
        }
        if let Ok(on) = file.boolean(GROUP, "player-backdrop") {
            settings.player_backdrop = on;
        }
        if let Ok(strength) = file.integer(GROUP, "glass-strength") {
            settings.glass_strength = strength.clamp(0, 100) as u8;
        }
        if let Ok(strength) = file.integer(GROUP, "lyrics-accent-strength") {
            settings.lyrics_accent_strength = strength.clamp(0, 100) as u8;
        }
        if let Ok(scale) = file.integer(GROUP, "lyrics-font-scale") {
            settings.lyrics_font_scale = scale.clamp(
                i32::from(MIN_LYRICS_FONT_SCALE),
                i32::from(MAX_LYRICS_FONT_SCALE),
            ) as u8;
        }
        if let Ok(accent) = file.string(GROUP, "accent") {
            settings.accent = accent.into();
        }
        if let Ok(companion) = file.string(GROUP, "companion") {
            settings.companion = Companion::parse(&companion);
        }
        if let Ok(companion) = file.string(GROUP, "launcher-icon") {
            settings.launcher_icon = Companion::parse(&companion);
        }
        if let Ok(quality) = file.string(GROUP, "jamkin-quality") {
            settings.jamkin_quality = JamkinQuality::parse(&quality);
        }
        if let Ok(enabled) = file.boolean(GROUP, "desktop-jamkin") {
            settings.desktop_jamkin = enabled;
        }
        if let Ok(size) = file.integer(GROUP, "desktop-jamkin-size") {
            settings.desktop_jamkin_size = size.clamp(
                i32::from(MIN_DESKTOP_JAMKIN_SIZE),
                i32::from(MAX_DESKTOP_JAMKIN_SIZE),
            ) as u16;
        }
        if let Ok(opacity) = file.integer(GROUP, "desktop-jamkin-opacity") {
            settings.desktop_jamkin_opacity = opacity.clamp(
                i32::from(MIN_DESKTOP_JAMKIN_OPACITY),
                i32::from(MAX_DESKTOP_JAMKIN_OPACITY),
            ) as u8;
        }
        if let Ok(visible) = file.boolean(GROUP, "desktop-jamkin-stay-visible") {
            settings.desktop_jamkin_stay_visible = visible;
        }
        if let Ok(right) = file.integer(GROUP, "desktop-jamkin-right") {
            settings.desktop_jamkin_right = right.clamp(0, MAX_DESKTOP_JAMKIN_MARGIN);
        }
        if let Ok(bottom) = file.integer(GROUP, "desktop-jamkin-bottom") {
            settings.desktop_jamkin_bottom = bottom.clamp(0, MAX_DESKTOP_JAMKIN_MARGIN);
        }
        if let Ok(above) = file.boolean(GROUP, "desktop-jamkin-above") {
            settings.desktop_jamkin_above = above;
        }
        if let Ok(enabled) = file.boolean(GROUP, "desktop-jamkin-oled-care") {
            settings.desktop_jamkin_oled_care = enabled;
        }
        if let Ok(reduced) = file.boolean(GROUP, "jamkin-reduced-motion") {
            settings.jamkin_reduced_motion = reduced;
        }
        if let Ok(sort) = file.string(GROUP, "sort") {
            settings.sort = sort.into();
        }
        if let Ok(rev) = file.boolean(GROUP, "sort-reversed") {
            settings.sort_reversed = rev;
        }
        for (key, into) in [
            ("album-sort", &mut settings.album_sort),
            ("artist-sort", &mut settings.artist_sort),
            ("playlist-sort", &mut settings.playlist_sort),
        ] {
            if let Ok(value) = file.string(GROUP, key) {
                *into = value.into();
            }
        }
        for (key, into) in [
            ("album-sort-reversed", &mut settings.album_sort_reversed),
            ("artist-sort-reversed", &mut settings.artist_sort_reversed),
            (
                "playlist-sort-reversed",
                &mut settings.playlist_sort_reversed,
            ),
        ] {
            if let Ok(value) = file.boolean(GROUP, key) {
                *into = value;
            }
        }
        if let Ok(pinned) = file.string(GROUP, "pinned-playlists") {
            settings.pinned_playlists = parse_pins(&pinned);
        }
        settings
    }

    /// Write preferences. Best-effort: failing to save a preference must never
    /// interrupt playback.
    pub fn save(&self) {
        let Some(path) = path() else {
            return;
        };
        let Some(dir) = path.parent() else {
            return;
        };

        let file = KeyFile::new();
        file.set_string(GROUP, "theme", self.theme.as_str());
        file.set_boolean(GROUP, "notify-track-change", self.notify_track_change);
        file.set_boolean(GROUP, "discord-activity", self.discord_activity);
        file.set_boolean(GROUP, "lyrics-enabled", self.lyrics_enabled);
        file.set_string(GROUP, "section", self.section.as_str());
        file.set_boolean(GROUP, "show-sidebar", self.show_sidebar);
        file.set_boolean(GROUP, "player-backdrop", self.player_backdrop);
        file.set_integer(GROUP, "glass-strength", i32::from(self.glass_strength));
        file.set_integer(
            GROUP,
            "lyrics-accent-strength",
            i32::from(self.lyrics_accent_strength),
        );
        file.set_integer(
            GROUP,
            "lyrics-font-scale",
            i32::from(self.lyrics_font_scale),
        );
        file.set_string(GROUP, "accent", &self.accent);
        file.set_string(GROUP, "companion", self.companion.id());
        file.set_string(GROUP, "launcher-icon", self.launcher_icon.id());
        file.set_string(GROUP, "jamkin-quality", self.jamkin_quality.as_str());
        file.set_boolean(GROUP, "desktop-jamkin", self.desktop_jamkin);
        file.set_integer(
            GROUP,
            "desktop-jamkin-size",
            i32::from(self.desktop_jamkin_size),
        );
        file.set_integer(
            GROUP,
            "desktop-jamkin-opacity",
            i32::from(self.desktop_jamkin_opacity),
        );
        file.set_boolean(
            GROUP,
            "desktop-jamkin-stay-visible",
            self.desktop_jamkin_stay_visible,
        );
        file.set_integer(GROUP, "desktop-jamkin-right", self.desktop_jamkin_right);
        file.set_integer(GROUP, "desktop-jamkin-bottom", self.desktop_jamkin_bottom);
        file.set_boolean(GROUP, "desktop-jamkin-above", self.desktop_jamkin_above);
        file.set_boolean(
            GROUP,
            "desktop-jamkin-oled-care",
            self.desktop_jamkin_oled_care,
        );
        file.set_boolean(GROUP, "jamkin-reduced-motion", self.jamkin_reduced_motion);
        file.set_string(GROUP, "sort", &self.sort);
        file.set_boolean(GROUP, "sort-reversed", self.sort_reversed);
        file.set_string(GROUP, "album-sort", &self.album_sort);
        file.set_boolean(GROUP, "album-sort-reversed", self.album_sort_reversed);
        file.set_string(GROUP, "artist-sort", &self.artist_sort);
        file.set_boolean(GROUP, "artist-sort-reversed", self.artist_sort_reversed);
        file.set_string(GROUP, "playlist-sort", &self.playlist_sort);
        file.set_boolean(GROUP, "playlist-sort-reversed", self.playlist_sort_reversed);
        file.set_string(
            GROUP,
            "pinned-playlists",
            &join_pins(&self.pinned_playlists),
        );

        if crate::private_storage::ensure_dir(dir).is_err() {
            tracing::warn!("could not create config directory");
            return;
        }
        let data = file.to_data();
        if data.len() > 64 * 1024 || crate::private_storage::write(&path, data.as_bytes()).is_err()
        {
            tracing::warn!("could not save settings");
        }
    }

    /// Apply the colour scheme. Called at startup before the window is shown,
    /// so there is no flash of the wrong theme, and again whenever it changes.
    pub fn apply_theme(&self) {
        let manager = relm4::adw::StyleManager::default();
        manager.set_color_scheme(match self.theme {
            Theme::System => relm4::adw::ColorScheme::Default,
            Theme::Light => relm4::adw::ColorScheme::ForceLight,
            Theme::Dark => relm4::adw::ColorScheme::ForceDark,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_round_trips_through_its_string_form() {
        for theme in [Theme::System, Theme::Light, Theme::Dark] {
            assert_eq!(Theme::parse(theme.as_str()), theme);
        }
    }

    #[test]
    fn an_unknown_theme_falls_back_to_system() {
        // A hand-edited or future-version ini must not break startup.
        assert_eq!(Theme::parse("solarized"), Theme::System);
        assert_eq!(Theme::parse(""), Theme::System);
    }

    #[test]
    fn theme_round_trips_through_its_combo_index() {
        for theme in [Theme::System, Theme::Light, Theme::Dark] {
            assert_eq!(Theme::from_index(theme.index()), theme);
        }
    }

    #[test]
    fn an_out_of_range_index_falls_back_to_system() {
        assert_eq!(Theme::from_index(99), Theme::System);
    }

    #[test]
    fn section_round_trips_and_falls_back() {
        for section in [
            Section::Library,
            Section::Explore,
            Section::Lyrics,
            Section::Albums,
            Section::Artists,
            Section::Playlists,
            Section::Catalog,
        ] {
            assert_eq!(Section::parse(section.as_str()), section);
        }
        // A hand-edited or future-version ini must not break startup. This
        // once used "playlists" as the unknown value, which stopped being one.
        assert_eq!(Section::parse("radio"), Section::Library);
        assert_eq!(Section::parse(""), Section::Library);
    }

    #[test]
    fn the_sidebar_starts_visible() {
        // Not bool's default: a sidebar nobody has hidden should be findable.
        assert!(Settings::default().show_sidebar);
    }

    #[test]
    fn the_backdrop_starts_on() {
        // Not `bool`'s default. #145 asked for a way *out* of the backdrop and
        // said to leave it on for everyone else, so a forgotten `..default()`
        // that silently turned it off would be the opposite of the request.
        assert!(Settings::default().player_backdrop);
        assert_eq!(Settings::default().theme, Theme::Light);
    }

    #[test]
    fn glass_starts_visibly_translucent() {
        let strength = Settings::default().glass_strength;
        assert_eq!(strength, DEFAULT_GLASS_STRENGTH);
        assert!((50..=80).contains(&strength));
    }

    #[test]
    fn nearby_lyrics_start_with_a_distinct_but_restrained_accent() {
        let strength = Settings::default().lyrics_accent_strength;
        assert_eq!(strength, DEFAULT_LYRICS_ACCENT_STRENGTH);
        assert!((50..=80).contains(&strength));
    }

    #[test]
    fn lyric_text_starts_at_the_curated_size_and_has_safe_bounds() {
        let scale = Settings::default().lyrics_font_scale;
        assert_eq!(scale, DEFAULT_LYRICS_FONT_SCALE);
        assert!((MIN_LYRICS_FONT_SCALE..=MAX_LYRICS_FONT_SCALE).contains(&scale));
    }

    #[test]
    fn third_party_lyrics_start_disabled() {
        assert!(!Settings::default().lyrics_enabled);
    }

    #[test]
    fn notifications_are_off_by_default() {
        // One notification per song is noise; opting in is the user's choice.
        assert!(!Settings::default().notify_track_change);
    }

    #[test]
    fn discord_activity_is_an_explicit_opt_in() {
        assert!(!Settings::default().discord_activity);
    }

    #[test]
    fn jambun_is_the_default_companion() {
        assert_eq!(Settings::default().companion, Companion::JamBun);
        assert_eq!(Settings::default().launcher_icon, Companion::JamBun);
        assert_eq!(Settings::default().jamkin_quality, JamkinQuality::High);
        assert_eq!(Settings::default().accent, "jamkin");
    }

    #[test]
    fn jamkin_quality_round_trips_and_auto_is_conservative() {
        for quality in JamkinQuality::ALL {
            assert_eq!(JamkinQuality::parse(quality.as_str()), quality);
            assert_eq!(JamkinQuality::from_index(quality.index()), quality);
        }
        assert!(!JamkinQuality::Auto.uses_high_resolution(320, 1));
        assert!(JamkinQuality::Auto.uses_high_resolution(321, 1));
        assert!(JamkinQuality::Auto.uses_high_resolution(142, 2));
        assert!(JamkinQuality::High.uses_high_resolution(72, 1));
        assert!(!JamkinQuality::Performance.uses_high_resolution(384, 3));
        assert_eq!(JamkinQuality::from_index(99), JamkinQuality::Auto);
    }

    #[test]
    fn desktop_jamkin_starts_at_a_useful_mid_size() {
        let size = Settings::default().desktop_jamkin_size;
        assert_eq!(size, DEFAULT_DESKTOP_JAMKIN_SIZE);
        assert!((MIN_DESKTOP_JAMKIN_SIZE..=MAX_DESKTOP_JAMKIN_SIZE).contains(&size));
        assert_eq!(
            Settings::default().desktop_jamkin_right,
            DEFAULT_DESKTOP_JAMKIN_MARGIN
        );
        assert_eq!(
            Settings::default().desktop_jamkin_bottom,
            DEFAULT_DESKTOP_JAMKIN_MARGIN
        );
        assert!(Settings::default().desktop_jamkin);
        assert!(Settings::default().desktop_jamkin_above);
        assert!(Settings::default().desktop_jamkin_oled_care);
        assert_eq!(
            Settings::default().desktop_jamkin_opacity,
            DEFAULT_DESKTOP_JAMKIN_OPACITY
        );
        assert!(Settings::default().desktop_jamkin_stay_visible);
        assert!(!Settings::default().jamkin_reduced_motion);
    }

    #[test]
    fn stored_preferences_override_every_changed_fresh_install_default() {
        let stored = Settings::from_data(
            "[Jamelade]\n\
             theme=dark\n\
             accent=blue\n\
             companion=jamjoe\n\
             launcher-icon=jampam\n\
             jamkin-quality=performance\n\
             desktop-jamkin=false\n\
             desktop-jamkin-size=213\n\
             desktop-jamkin-opacity=65\n\
             desktop-jamkin-stay-visible=false\n\
             desktop-jamkin-above=false\n\
             desktop-jamkin-oled-care=false\n\
             player-backdrop=false\n\
             glass-strength=31\n\
             lyrics-accent-strength=44\n\
             lyrics-font-scale=90\n\
             notify-track-change=true\n\
             discord-activity=true\n\
             lyrics-enabled=true\n",
        );

        assert_eq!(stored.theme, Theme::Dark);
        assert_eq!(stored.accent, "blue");
        assert_eq!(stored.companion, Companion::JamJoe);
        assert_eq!(stored.launcher_icon, Companion::JamPam);
        assert_eq!(stored.jamkin_quality, JamkinQuality::Performance);
        assert!(!stored.desktop_jamkin);
        assert_eq!(stored.desktop_jamkin_size, 213);
        assert_eq!(stored.desktop_jamkin_opacity, 65);
        assert!(!stored.desktop_jamkin_stay_visible);
        assert!(!stored.desktop_jamkin_above);
        assert!(!stored.desktop_jamkin_oled_care);
        assert!(!stored.player_backdrop);
        assert_eq!(stored.glass_strength, 31);
        assert_eq!(stored.lyrics_accent_strength, 44);
        assert_eq!(stored.lyrics_font_scale, 90);
        assert!(stored.notify_track_change);
        assert!(stored.discord_activity);
        assert!(stored.lyrics_enabled);
    }

    #[test]
    fn pins_round_trip_in_the_order_they_were_put_there() {
        // Order is the feature: pin order is what the sidebar draws, so a round
        // trip that sorted or reversed them would silently rearrange somebody's
        // sidebar between launches.
        let pins = vec![
            "p.EYWrg13SzrKxYBb".to_owned(),
            "p.e5Ukqg18xa".to_owned(),
            "p.rXAJKDruDkOY0Eg".to_owned(),
        ];
        assert_eq!(parse_pins(&join_pins(&pins)), pins);
    }

    #[test]
    fn nothing_pinned_stays_nothing() {
        assert_eq!(join_pins(&[]), "");
        assert!(parse_pins("").is_empty());
        // A key left behind by hand-editing is the same as no key.
        assert!(parse_pins(";;  ;").is_empty());
    }

    #[test]
    fn a_playlist_cannot_be_pinned_twice() {
        // Two pins of one playlist are two identical rows, and no click could
        // tell them apart.
        let stored = "p.one;p.two;p.one";
        assert_eq!(parse_pins(stored), vec!["p.one", "p.two"]);
    }

    #[test]
    fn an_id_that_would_corrupt_the_format_is_dropped_not_split() {
        // No real library id contains the separator, but one that did would
        // come back as two pins pointing nowhere. Losing it beats that.
        let pins = vec!["p.fine".to_owned(), "p.b;roken".to_owned()];
        assert_eq!(join_pins(&pins), "p.fine");
    }

    #[test]
    fn surrounding_space_from_a_hand_edited_file_is_forgiven() {
        assert_eq!(parse_pins(" p.one ; p.two "), vec!["p.one", "p.two"]);
    }

    #[test]
    fn pin_storage_is_bounded_and_rejects_control_characters() {
        let many: Vec<String> = (0..PIN_MAX_COUNT + 10)
            .map(|index| format!("p.{index}"))
            .collect();
        assert_eq!(parse_pins(&join_pins(&many)).len(), PIN_MAX_COUNT);
        assert_eq!(
            parse_pins(&format!("p.ok;p.{}", "x".repeat(PIN_MAX_ID_BYTES))).len(),
            1
        );
        assert_eq!(join_pins(&["p.ok".into(), "p.bad\nvalue".into()]), "p.ok");
    }
}
