// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

mod app;
mod apple_link;
mod companion;
mod components;
mod discord;
mod i18n;
mod launcher_icon;
mod library_cache;
mod lyric_timing;
mod lyrics;
mod mpris;
mod music;
mod notify;
mod palette;
mod player;
mod playlist_export;
mod private_storage;
mod scrobble;
mod segment_loop;
mod session;
mod settings;
mod sleep_timer;
mod style;
mod unplayable;

use relm4::RelmApp;
use relm4::gtk;
use tracing_subscriber::EnvFilter;

#[cfg(not(feature = "broker-test"))]
pub(crate) const APP_ID: &str = "io.github.Jamelade.Jamelade";
#[cfg(feature = "broker-test")]
pub(crate) const APP_ID: &str = "io.github.Jamelade.Jamelade.BrokerTest";
/// A portal-managed sub-launcher whose icon can be replaced without granting
/// the Flatpak access to the host's application directory.
#[cfg(not(feature = "broker-test"))]
pub(crate) const LAUNCHER_ID: &str = "io.github.Jamelade.Jamelade.Launcher";
#[cfg(feature = "broker-test")]
pub(crate) const LAUNCHER_ID: &str = "io.github.Jamelade.Jamelade.BrokerTest.Launcher";
#[cfg(not(feature = "broker-test"))]
pub(crate) const APP_NAME: &str = "Jamelade";
#[cfg(feature = "broker-test")]
pub(crate) const APP_NAME: &str = "Jamelade Broker Test";
#[cfg(not(feature = "broker-test"))]
pub(crate) const MPRIS_BUS_SUFFIX: &str = "Jamelade";
#[cfg(feature = "broker-test")]
pub(crate) const MPRIS_BUS_SUFFIX: &str = "JameladeBrokerTest";
#[cfg(not(feature = "broker-test"))]
pub(crate) const SIDECAR_IDENTITY: &str = "stable";
#[cfg(feature = "broker-test")]
pub(crate) const SIDECAR_IDENTITY: &str = "broker-test";
#[cfg(not(feature = "broker-test"))]
pub(crate) const SIDECAR_PROFILE_NAME: &str = "Jamelade";
#[cfg(feature = "broker-test")]
pub(crate) const SIDECAR_PROFILE_NAME: &str = "JameladeBrokerTest";

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            // Normal launches are intentionally quiet. Even the opt-in info
            // stream records only state, counts and timings — not search terms,
            // titles, playlist names, ids, credentials or local paths.
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("jamelade=warn")),
        )
        .init();

    // `RelmApp::new` calls `gtk::init()` and — because we enable relm4's
    // `libadwaita` feature — `adw::init()` too. So there's deliberately no adw
    // init here.
    let app = RelmApp::new(LAUNCHER_ID);
    let settings = settings::Settings::load();
    setup_icon(settings.launcher_icon);

    // Apply the colour scheme before the window is shown. The model owns the
    // settings from here.
    settings.apply_theme();
    // Before the window exists, so nothing is ever drawn in the wrong accent.
    style::init(
        settings.theme,
        style::Accent::parse(&settings.accent),
        settings.companion,
        settings.player_backdrop,
        settings.glass_strength,
        settings.lyrics_accent_strength,
        settings.lyrics_font_scale,
    );
    app.run::<app::AppModel>(settings);
}

/// Set the selected Jamkin for X11 and desktops that support toplevel icons.
/// The portal-managed desktop entry remains the launcher source of truth.
fn setup_icon(companion: companion::Companion) {
    // Release builds use the installed icon theme, not the build directory.
    #[cfg(debug_assertions)]
    if let Some(display) = relm4::gtk::gdk::Display::default() {
        let theme = gtk::IconTheme::for_display(&display);
        theme.add_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/data/icons"));
    }
    gtk::Window::set_default_icon_name(companion.window_icon_name());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_identities_are_a_consistent_profile() {
        assert!(LAUNCHER_ID.starts_with(&format!("{APP_ID}.")));
        assert!(!APP_NAME.is_empty());
        assert!(!MPRIS_BUS_SUFFIX.contains('.'));
        if cfg!(feature = "broker-test") {
            assert!(APP_ID.ends_with(".BrokerTest"));
            assert_eq!(SIDECAR_IDENTITY, "broker-test");
            assert_eq!(SIDECAR_PROFILE_NAME, "JameladeBrokerTest");
        } else {
            assert_eq!(APP_ID, "io.github.Jamelade.Jamelade");
            assert_eq!(SIDECAR_IDENTITY, "stable");
            assert_eq!(SIDECAR_PROFILE_NAME, "Jamelade");
        }
    }
}
