// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! The app's accent colour, album glass palette and player backdrop.
//!
//! ARCHITECTURE.md says not to reach for CSS where a libadwaita widget would do. This
//! is the exception it allows: an **accent colour** is not a widget. libadwaita
//! 1.6 exposes it as CSS variables (`--accent-bg-color` and friends) and there
//! is no API to set an app-specific one, so a provider is the only route.
//!
//! Eight providers, deliberately:
//!
//! - a **theme** one for an optional named surface palette;
//! - a **base** one, replaced only when the accent preference changes;
//! - a **material** one carrying the user-selected transparency;
//! - a **glass** one carrying two locally-derived album colours;
//! - a **backdrop** one carrying the cover behind the window, replaced on
//!   every track; and
//! - an **adaptive foreground** one used only at the fully clear endpoint; and
//! - two tiny **lyrics** providers carrying nearby-line colour and text scale.
//!
//! Keeping them apart means a new cover does not reparse the full theme, and a
//! missing backdrop or palette cannot take the other surfaces with it.

use relm4::adw;
use relm4::gtk::{self, gdk};

use crate::companion::Companion;
use crate::palette::AlbumPalette;
use crate::settings::Theme;

mod lyrics;
mod theme;
pub use lyrics::{
    set_accent_strength as set_lyrics_accent_strength, set_font_scale as set_lyrics_font_scale,
};

/// Accent choices offered in Preferences.
///
/// The chosen Jamkin is the default: it gives Jamelade a coherent identity
/// without taking away the system and conventional colour choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Accent {
    #[default]
    Jamkin,
    System,
    Blue,
    Purple,
    Green,
    Orange,
}

impl Accent {
    pub const ALL: [Self; 6] = [
        Self::Jamkin,
        Self::System,
        Self::Blue,
        Self::Purple,
        Self::Green,
        Self::Orange,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Jamkin => "Match Jamkin",
            Self::System => "Follow System",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
            Self::Green => "Green",
            Self::Orange => "Orange",
        }
    }

    /// What lands in the ini file.
    pub fn id(self) -> &'static str {
        match self {
            Self::Jamkin => "jamkin",
            Self::System => "system",
            Self::Blue => "blue",
            Self::Purple => "purple",
            Self::Green => "green",
            Self::Orange => "orange",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "system" => Self::System,
            "blue" => Self::Blue,
            "purple" => Self::Purple,
            "green" => Self::Green,
            "orange" => Self::Orange,
            _ => Self::Jamkin,
        }
    }

    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|a| *a == self).unwrap_or(0) as u32
    }

    pub fn from_index(i: u32) -> Self {
        Self::ALL.get(i as usize).copied().unwrap_or_default()
    }

    /// `(background, foreground, secondary)`. `None` means "leave libadwaita alone", which
    /// is how Follow System works — the desktop's own accent is already in
    /// those variables and the right move is to write nothing over it.
    fn colors(self, companion: Companion) -> Option<(&'static str, &'static str, &'static str)> {
        match self {
            Self::Jamkin => {
                let palette = companion.palette();
                Some((palette.accent, palette.foreground, palette.secondary))
            }
            Self::System => None,
            Self::Blue => Some(("#3584e4", "#ffffff", "#d99a28")),
            Self::Purple => Some(("#9141ac", "#ffffff", "#d99a28")),
            Self::Green => Some(("#147d55", "#ffffff", "#d99a28")),
            Self::Orange => Some(("#b34b00", "#ffffff", "#744a9e")),
        }
    }
}

thread_local! {
    /// Named surface palettes. Empty for stock Light, Dark and Follow System.
    static THEME: gtk::CssProvider = gtk::CssProvider::new();
    static BASE: gtk::CssProvider = gtk::CssProvider::new();
    /// Surface opacity, replaced when the glass slider moves. Separate from the
    /// album palette so dragging it does not reset a colour transition.
    static MATERIAL: gtk::CssProvider = gtk::CssProvider::new();
    /// Only two custom properties. Replaced on every frame of a track palette
    /// transition without reparsing the complete application stylesheet.
    static GLASS: gtk::CssProvider = gtk::CssProvider::new();
    /// The cover behind the main window. Separate from `BASE` for the
    /// reason the module opens with: this one is replaced on every track, and
    /// recolouring the player should not reparse the accent rules.
    static BACKDROP: gtk::CssProvider = gtk::CssProvider::new();
    /// White or dark text once the cover becomes exposed. Empty below that
    /// point, so system foreground colours remain untouched for ordinary glass.
    static ADAPTIVE_TEXT: gtk::CssProvider = gtk::CssProvider::new();
    /// Whether that cover is painted at all — the Preferences toggle (#145).
    ///
    /// Here rather than gated at the call site because this module already owns
    /// *what is on screen*. `SHOWN_ART` is what the theme-flip handler repaints
    /// from, so a caller-side gate would put the cover back on the next
    /// light/dark flip with nothing having asked for it.
    /// The one saved switch owns both halves of album-aware glass: the cover
    /// image and its extracted colours. Off, a named theme supplies the glass
    /// colours instead and no track artwork is painted.
    static ALBUM_GLASS_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    static ACTIVE_THEME: std::cell::Cell<Theme> = const {
        std::cell::Cell::new(Theme::Light)
    };
    /// Persisted 0–100 material strength. Read at backdrop-paint time so theme
    /// flips and cover cross-fades always use the current slider value.
    static GLASS_STRENGTH: std::cell::Cell<u8> = const {
        std::cell::Cell::new(crate::settings::DEFAULT_GLASS_STRENGTH)
    };
}

/// Install the providers. Called once, before the window is shown.
pub fn init(
    theme: Theme,
    accent: Accent,
    companion: Companion,
    backdrop: bool,
    glass_strength: u8,
    lyrics_accent_strength: u8,
    lyrics_font_scale: u8,
) {
    let Some(display) = gdk::Display::default() else {
        // No display means no styling to do, and certainly nothing to fail on.
        return;
    };
    THEME.with(|p| {
        gtk::style_context_add_provider_for_display(
            &display,
            p,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 5,
        )
    });
    BASE.with(|p| {
        gtk::style_context_add_provider_for_display(
            &display,
            p,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        )
    });
    MATERIAL.with(|p| {
        gtk::style_context_add_provider_for_display(
            &display,
            p,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        )
    });
    GLASS.with(|p| {
        gtk::style_context_add_provider_for_display(
            &display,
            p,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
        )
    });
    BACKDROP.with(|p| {
        // Above both material and palette: it owns the image layers, while the
        // lower providers supply their colour and surface opacity.
        gtk::style_context_add_provider_for_display(
            &display,
            p,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 3,
        )
    });
    ADAPTIVE_TEXT.with(|p| {
        gtk::style_context_add_provider_for_display(
            &display,
            p,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 6,
        )
    });
    lyrics::install(&display, lyrics_accent_strength, lyrics_font_scale);
    ALBUM_GLASS_ON.with(|c| c.set(backdrop));
    set_theme(theme);
    set_accent(accent, companion);
    set_glass_strength(glass_strength);

    // **Repaint the backdrop when the theme flips.** The veil's alphas differ
    // per theme (see `Veil`), so a cover painted while dark stays painted with
    // dark's numbers until the next track — which on a paused player is never.
    // Switching to light then left the drawer wearing an 0.86 white veil, which
    // is the washed-out state this pair of numbers exists to avoid.
    //
    // The shown paths already hold what is on screen, so repainting is just
    // re-emitting the same images through the current theme's numbers.
    adw::StyleManager::default().connect_dark_notify(|_| repaint_shown_backdrop());
}

/// Apply a full-window surface palette without changing the independent accent
/// preference. Stock Light, Dark and Follow System deliberately clear it.
pub fn set_theme(theme: Theme) {
    ACTIVE_THEME.with(|active| active.set(theme));
    THEME.with(|provider| provider.load_from_string(&theme::css(theme)));
    refresh_album_palette();
    repaint_shown_backdrop();
    refresh_adaptive_text();
}

/// Centre the one album-art surface explicitly. This used to fall out of an
/// animation's keyframes; without it CSS defaults to the top-left corner.
const COVER_LAYOUT: &str = ".jamelade-window { background-size: cover, cover, cover, cover;
                            background-position: center, center, center, center; }";

/// The expanded player is an application surface, not another photograph.
/// Keeping it on the selected theme prevents a maximised album cover from
/// turning the drawer into a noisy second backdrop.
const PLAYER_SHEET_SURFACE: &str = ".np-sheet {
         background-color: @window_bg_color;
         background-image: none;
         color: @window_fg_color;
         text-shadow: none;
     }
     .np-sheet .player-state-control {
         color: @window_fg_color;
         opacity: 0.45;
         transition: opacity 120ms ease;
     }
     .np-sheet .player-state-control.player-control-active,
     .np-sheet .player-state-control:active,
     .np-sheet .player-state-control:checked {
         color: @window_fg_color;
         opacity: 1;
     }";

/// Prefer SF Pro Display only when the user has installed it locally. Jamelade
/// neither bundles nor redistributes Apple's font. `system-ui` is deliberately
/// next, so a machine without it gets its desktop's own UI face rather than a
/// font the app happens to prefer.
const UI_FONT_STACK: &str = "\"SF Pro Display\", system-ui, sans-serif";

/// Apply an accent, and the handful of rules that go with it.
pub fn set_accent(accent: Accent, companion: Companion) {
    let accent_rules = match accent.colors(companion) {
        Some((bg, fg, secondary)) => format!(
            ":root {{
                 --accent-bg-color: {bg};
                 --accent-fg-color: {fg};
                 --accent-color: {bg};
                 --jam-secondary-color: {secondary};
                 --glass-primary-color: {bg};
                 --glass-secondary-color: {secondary};
                 --theme-glass-primary-color: {bg};
                 --theme-glass-secondary-color: {secondary};
                 --jamelade-headerbar-color: @headerbar_bg_color;
                 --art-fg-color: @window_fg_color;
                 --art-shadow-color: @window_bg_color;
             }}"
        ),
        None => ":root {
                     --jam-secondary-color: #d99a28;
                     --glass-primary-color: var(--accent-bg-color);
                     --glass-secondary-color: #d99a28;
                     --theme-glass-primary-color: var(--accent-bg-color);
                     --theme-glass-secondary-color: #d99a28;
                     --jamelade-headerbar-color: @headerbar_bg_color;
                     --art-fg-color: @window_fg_color;
                     --art-shadow-color: @window_bg_color;
                 }"
        .into(),
    };

    // A favourite is yellow everywhere else it appears — Apple's own star, the
    // one on your phone — so it does not follow the accent. Hard-coded to
    // Adwaita's yellow rather than a `.warning`, which means something else.
    let css = format!(
        "{accent_rules}
         .favorite-star {{ color: #f5c211; }}
         .player-metadata-link {{
             color: var(--accent-color);
             min-height: 0;
             padding: 2px 8px;
             border-radius: 9999px;
             transition: 150ms ease;
         }}
         .player-metadata-link:hover {{
             background-color: alpha(var(--accent-color), 0.10);
         }}
         .player-metadata-link:focus-visible {{
             box-shadow: inset 0 0 0 2px alpha(var(--accent-color), 0.42);
         }}

         /* A restrained liquid-glass material. GTK has no backdrop-filter or
            iOS-style refraction shader, so the depth comes from translucent
            layers, edge highlights and soft shadows over the current album.
            Jamkin colours stay on controls and names, and stock controls
            remain Adwaita controls with their original contrast. */
         .jamelade-window {{
             font-family: {UI_FONT_STACK};
             background-image: linear-gradient(
                     128deg,
                     alpha(var(--glass-primary-color), 0.115),
                     transparent 42%
                 ),
                 linear-gradient(
                     312deg,
                     alpha(var(--glass-secondary-color), 0.085),
                     transparent 46%
                 );
         }}
         /* Secondary copy stays quiet without fading into pale themed
            surfaces. Artwork mode has its own adaptive rule below. */
         .jamelade-window .dim-label {{
             color: alpha(@window_fg_color, 0.73);
         }}
         .jamelade-window headerbar {{
             background-color: alpha(var(--jamelade-headerbar-color), 0.78);
             background-image: linear-gradient(
                 180deg,
                 alpha(#ffffff, 0.12),
                 transparent 72%
             );
             box-shadow:
                 inset 0 -1px alpha(currentColor, 0.09),
                 0 8px 24px alpha(#000000, 0.055);
         }}
         .jam-glass-sidebar {{
             background-color: alpha(@window_bg_color, 0.76);
             background-image:
                 radial-gradient(
                     circle at 18% 4%,
                     alpha(var(--glass-primary-color), 0.24),
                     transparent 43%
                 ),
                 linear-gradient(
                     158deg,
                     alpha(#ffffff, 0.14),
                     alpha(var(--glass-primary-color), 0.075) 48%,
                     alpha(var(--glass-secondary-color), 0.09)
                 );
             box-shadow:
                 inset -1px 0 alpha(#ffffff, 0.12),
                 12px 0 34px alpha(#000000, 0.10);
         }}
         .jam-glass-sidebar scrolledwindow,
         .jam-glass-sidebar scrolledwindow > viewport,
         .jam-glass-sidebar .navigation-sidebar {{
             background: none;
         }}
         .jamelade-window .jam-glass-sidebar headerbar {{
             background-image:
                 linear-gradient(
                     180deg,
                     alpha(#ffffff, 0.16),
                     transparent 78%
                 ),
                 linear-gradient(
                     110deg,
                     alpha(var(--glass-primary-color), 0.15),
                     alpha(var(--glass-secondary-color), 0.055)
                 );
             box-shadow:
                 inset 0 -1px alpha(#ffffff, 0.10),
                 0 10px 24px alpha(#000000, 0.07);
         }}
         .jam-glass-sidebar .navigation-sidebar row {{
             margin: 2px 7px;
             border-radius: 13px;
             border: 1px solid transparent;
             transition: 150ms ease;
         }}
         .jam-glass-sidebar .navigation-sidebar row:hover {{
             background-color: alpha(@window_fg_color, 0.065);
             background-image: linear-gradient(
                 135deg,
                 alpha(#ffffff, 0.105),
                 alpha(var(--glass-primary-color), 0.055)
             );
             border-color: alpha(#ffffff, 0.075);
         }}
         .jam-glass-sidebar .navigation-sidebar row:selected {{
             background-color: alpha(var(--glass-primary-color), 0.20);
             background-image:
                 linear-gradient(
                     135deg,
                     alpha(#ffffff, 0.17),
                     transparent 46%
                 ),
                 linear-gradient(
                     112deg,
                     alpha(var(--glass-primary-color), 0.22),
                     alpha(var(--glass-secondary-color), 0.095)
                 );
             border-color: alpha(#ffffff, 0.13);
             box-shadow:
                 inset 0 1px alpha(#ffffff, 0.16),
                 0 7px 18px alpha(#000000, 0.10);
         }}
         .jam-glass-sidebar .navigation-sidebar row:selected image {{
             color: var(--accent-color);
         }}
         /* A fixed, unpainted stage above the independently scrolling lyrics.
            The companion artwork itself is a transparent sprite: deliberately
            no card, clipping radius or square-shaped shadow here. */
         .jamkin-stage {{ padding: 0 8px; }}
         .jamkin-portrait {{
             border-radius: 20px;
             box-shadow:
                 inset 0 1px alpha(#ffffff, 0.24),
                 0 7px 20px alpha(#000000, 0.18);
         }}
         .launcher-tile-preview {{
             border-radius: 16px;
             box-shadow:
                 inset 0 1px alpha(#ffffff, 0.22),
                 0 6px 17px alpha(#000000, 0.16);
         }}
         .jamkin-name {{
             color: var(--accent-color);
             font-weight: 800;
         }}

         /* Jamkin Mode is a real tiny toplevel, not a screenshot-shaped card.
            Clearing every layer here is what lets the sprite's alpha reach the
            desktop. The hover lyric is a separate popup surface, so none of
            its closed area can intercept clicks behind the companion. */
         window.desktop-jamkin-window,
         .desktop-jamkin-window,
         .desktop-jamkin-window > windowhandle,
         .desktop-jamkin-handle {{
             font-family: {UI_FONT_STACK};
             background-color: transparent;
             background-image: none;
             border: none;
             box-shadow: none;
         }}
         .desktop-jamkin-handle {{ padding: 5px; }}
         .desktop-jamkin-sprite {{
             margin: 1px;
         }}
         popover.jamkin-lyrics-popover > contents,
         .jamkin-lyrics-popover > contents {{
             min-width: 245px;
             border-radius: 20px;
             background-color: alpha(@window_bg_color, 0.84);
             background-image:
                 linear-gradient(
                     140deg,
                     alpha(#ffffff, 0.20),
                     transparent 42%
                 ),
                 linear-gradient(
                     115deg,
                     alpha(var(--glass-primary-color), 0.18),
                     alpha(var(--glass-secondary-color), 0.09)
                 );
             border: 1px solid alpha(currentColor, 0.11);
             box-shadow:
                 inset 0 1px alpha(#ffffff, 0.24),
                 0 14px 34px alpha(#000000, 0.24);
         }}
         .jamkin-bubble-current {{
             font-size: 1.08em;
             font-weight: 750;
         }}
         .jamkin-bubble-next {{ font-weight: 550; }}

         /* The compact player is a stable piece of the selected theme. Album
            artwork remains behind the main window, never duplicated or
            blurred inside this pill. The progress line stays outside the
            padded row so it can still reach both edges. */
         .np-bar,
         .np-bar:backdrop {{
             background-image: none;
             color: @window_fg_color;
             text-shadow: none;
         }}
         .np-bar .dim-label,
         .np-bar:backdrop .dim-label {{
             color: alpha(@window_fg_color, 0.73);
         }}
         .np-row {{
             margin: 7px;
             padding: 10px;
             border-radius: 22px;
             background-color: @window_bg_color;
             background-image: linear-gradient(
                 145deg,
                 alpha(#ffffff, 0.13),
                 transparent 42%,
                 alpha(var(--theme-glass-primary-color), 0.09)
             );
             box-shadow:
                 inset 0 1px alpha(#ffffff, 0.20),
                 inset 0 0 0 1px alpha(currentColor, 0.09),
                 0 9px 24px alpha(#000000, 0.10);
         }}

         {PLAYER_SHEET_SURFACE}

         .player-cover-link {{
             padding: 0;
             min-width: 0;
             min-height: 0;
             border-radius: 16px;
             transition: 150ms ease;
         }}
         .player-cover-link:hover {{
             box-shadow:
                 inset 0 0 0 1px alpha(#ffffff, 0.22),
                 0 12px 30px alpha(#000000, 0.22);
         }}
         .player-cover-link:focus-visible {{
             box-shadow:
                 inset 0 0 0 2px alpha(var(--accent-color), 0.72),
                 0 12px 30px alpha(#000000, 0.18);
         }}

         /* The bar's progress line. Thin and inset, with enough unplayed track
            to read as intentional playback progress instead of a loading
            indicator attached to the window edge.

            GTK draws a progressbar as trough > progress, and Adwaita gives
            both a radius and the trough a margin that would inset the line
            from the ends. Every one of these is undoing that. */
         .np-progress,
         .np-progress > trough,
         .np-progress > trough > progress {{
             min-height: 3px;
             border-radius: 9999px;
             padding: 0;
         }}
         .np-progress {{ margin: 0 18px 4px; }}
         .np-progress > trough {{ background-color: alpha(currentColor, 0.20); }}

         /* Search and filter are one family even though HeaderBar allocates
            them separately. Matching surfaces keep the filter from floating. */
         .jamelade-search-entry,
         .jamelade-search-filter > button {{
             background-color: alpha(@window_fg_color, 0.055);
             box-shadow: inset 0 0 0 1px alpha(currentColor, 0.075);
         }}

         /* Preferences retain Adwaita's native rows while making their three
            interaction types faster to scan. */
         .jamelade-preferences headerbar windowtitle > label.title {{
             font-size: 1.08em;
             font-weight: 700;
         }}
         .jamelade-preferences .dim-label {{
             color: alpha(@window_fg_color, 0.74);
         }}
         .jamelade-preferences .preferences-surface-group .boxed-list {{
             background-color: alpha(@window_bg_color, 0.92);
             box-shadow:
                 inset 0 0 0 1px alpha(currentColor, 0.08),
                 0 8px 22px alpha(#000000, 0.075);
         }}
         .preferences-value-row {{ min-height: 50px; }}
         .preferences-toggle-row {{
             min-height: 56px;
             padding-top: 2px;
             padding-bottom: 2px;
         }}
         .preferences-slider-row {{
             min-height: 62px;
             padding-top: 4px;
             padding-bottom: 4px;
         }}
         .preferences-slider-row scale {{ margin-left: 12px; }}

         /* Playlist-write sheets belong to the selected Jamelade theme, not
            the host's stock light dialog palette. Keep their one choice row
            consistent with Preferences without adding another glass layer. */
         .jamelade-themed-dialog,
         .jamelade-themed-dialog > contents {{
             color: @window_fg_color;
             background-color: @window_bg_color;
             background-image: none;
         }}
         .jamelade-themed-dialog .preferences-surface-group .boxed-list {{
             background-color: alpha(@window_bg_color, 0.92);
             box-shadow: inset 0 0 0 1px alpha(currentColor, 0.08);
         }}

         {COVER_LAYOUT}

         /* A scroller wrapping a `view` widget paints the `view` background,
            which is a shade darker than the window. That was invisible while
            this one was clamped to 800px and centred — it read as the list's
            own surface — and became a dark band across the whole window the
            moment the scroller spanned it, which is what moving the clamp
            *inside* (as `AdwClampScrollable`) did.

            Cleared on the scroller and on the viewport GTK may put inside it,
            because which of the two paints depends on whether the child
            implements `GtkScrollable`. The grid below needs the same thing for
            the same reason. */
         .plain-scroller,
         .plain-scroller > viewport {{
             background: none;
         }}

         /* Same reason. A GridView draws its own background, so insetting it
            with a margin shows a band of the window around every grid. */
         .tile-grid {{
             padding: 14px;
             padding-bottom: 26px;
             /* A GridView paints the `view` background, which is a shade
                darker than the window. The results list next door carries
                `navigation-sidebar` and is transparent, so the two sections
                did not match. */
             background: none;
         }}
         .media-tile {{
             border-radius: 15px;
             transition: 150ms ease;
         }}
         .media-tile:hover,
         .tile-grid > child:hover .media-tile {{
             background-color: alpha(@window_bg_color, 0.16);
             box-shadow:
                 inset 0 1px alpha(#ffffff, 0.15),
                 inset 0 0 0 1px alpha(currentColor, 0.07),
                 0 7px 18px alpha(#000000, 0.07);
         }}
         .tile-grid > child:focus-visible .media-tile {{
             box-shadow: inset 0 0 0 2px alpha(var(--accent-color), 0.42);
         }}

         /* Empty Search is a dashboard, not an error page. Its pills and cards
            are intentionally flatter than the main player and preferences so
            glass still describes hierarchy rather than coating everything. */
         .search-landing flowboxchild {{
             padding: 0;
             background: none;
         }}
         .search-clear-history {{
             color: var(--accent-color);
             font-weight: 700;
         }}
         .search-history-pill {{
             min-height: 44px;
             padding: 0 4px;
             border-radius: 9999px;
             background-color: alpha(@window_fg_color, 0.055);
             box-shadow: inset 0 0 0 1px alpha(currentColor, 0.055);
             transition: 140ms ease;
         }}
         .search-history-pill:hover {{
             background-color: alpha(var(--glass-primary-color), 0.12);
             box-shadow: inset 0 0 0 1px alpha(currentColor, 0.085);
         }}
         .search-history-open {{ padding-left: 10px; }}
         .search-category-card {{
             padding: 14px;
             border-radius: 18px;
             color: #ffffff;
             box-shadow:
                 inset 0 1px alpha(#ffffff, 0.20),
                 inset 0 0 0 1px alpha(#ffffff, 0.10),
                 0 8px 20px alpha(#000000, 0.12);
             transition: 150ms ease;
         }}
         .search-category-card:hover {{
             box-shadow:
                 inset 0 1px alpha(#ffffff, 0.25),
                 inset 0 0 0 1px alpha(#ffffff, 0.15),
                 0 11px 25px alpha(#000000, 0.16);
         }}
         .search-category-title {{
             color: #ffffff;
             text-shadow: 0 1px 4px alpha(#000000, 0.72);
         }}
         .category-new {{
             background-image: linear-gradient(135deg, #c77892, #698ea0);
         }}
         .category-hiphop {{
             background-image: linear-gradient(135deg, #2c1d26, #8b4b2f);
         }}
         .category-indie {{
             background-image: linear-gradient(135deg, #66727b, #28353f);
         }}
         .category-electronic {{
             background-image: linear-gradient(135deg, #1742a7, #0ba1d2);
         }}
         .category-chill {{
             background-image: linear-gradient(135deg, #ad718b, #df9fba);
         }}
         .search-trending-card {{
             min-width: 210px;
             padding: 9px;
             border-radius: 16px;
             background-color: alpha(@window_bg_color, 0.30);
             box-shadow:
                 inset 0 1px alpha(#ffffff, 0.14),
                 inset 0 0 0 1px alpha(currentColor, 0.065);
             transition: 140ms ease;
         }}
         .search-trending-card:hover {{
             background-color: alpha(var(--glass-primary-color), 0.13);
             box-shadow:
                 inset 0 1px alpha(#ffffff, 0.18),
                 inset 0 0 0 1px alpha(currentColor, 0.09),
                 0 7px 18px alpha(#000000, 0.08);
         }}

         /* Explore is still a native widget hierarchy; these rules provide
            hierarchy that no stock libadwaita shelf widget exists to supply.
            The accent appears only as a quiet edge and tint, so album covers
            remain the page's colour rather than competing with a painted UI. */
         .explore-hero {{
             padding: 24px 26px;
             border-radius: 24px;
             background-color: alpha(@window_bg_color, 0.56);
             background-image:
                 linear-gradient(
                     135deg,
                     alpha(#ffffff, 0.15),
                     transparent 38%
                 ),
                 linear-gradient(
                     115deg,
                     alpha(var(--glass-primary-color), 0.16),
                     alpha(var(--glass-secondary-color), 0.075)
                 );
             box-shadow:
                 inset 0 1px alpha(#ffffff, 0.22),
                 inset 0 0 0 1px alpha(currentColor, 0.09),
                 0 13px 32px alpha(#000000, 0.10);
         }}
         .explore-hero:hover {{
             background-image:
                 linear-gradient(
                     135deg,
                     alpha(#ffffff, 0.19),
                     transparent 40%
                 ),
                 linear-gradient(
                     115deg,
                     alpha(var(--glass-primary-color), 0.20),
                     alpha(var(--glass-secondary-color), 0.09)
                 );
         }}
         .artist-biography-toggle {{
             min-height: 0;
             padding: 5px 0;
             border-radius: 10px;
         }}
         .artist-biography-toggle:hover {{
             color: var(--accent-color);
             background-color: alpha(var(--accent-color), 0.075);
         }}
         .artist-biography-toggle:focus-visible {{
             box-shadow: inset 0 0 0 2px alpha(var(--accent-color), 0.40);
         }}
         .artist-biography-preview {{
             opacity: 0.78;
         }}
         .artist-biography-full {{
             padding-top: 15px;
             border-top: 1px solid alpha(currentColor, 0.11);
         }}
         .explore-kicker {{
             color: var(--accent-color);
             font-weight: 800;
             letter-spacing: 0.08em;
         }}
         .explore-shelf,
         .explore-shelf > viewport {{
             background: none;
         }}
         .explore-card {{
             padding: 6px;
             border-radius: 12px;
         }}
         .explore-card:hover {{
             background-color: alpha(currentColor, 0.075);
         }}

         /* LRCLIB supplies line timestamps rather than word timings. Rust
            animates each active-line opacity and the scroll adjustment; CSS
            supplies a stable glass highlight without changing the line's size
            and causing the whole verse to jump. */
         .lyrics-line {{
             padding: 8px 24px;
             border-radius: 20px;
             font-size: 1.42em;
             font-weight: 600;
             /* The shadow follows the active light/dark surface colour. On a
                bright cover it supports dark text; on a dark cover it supports
                light text. This keeps artwork-derived palettes legible without
                putting every inactive line inside another glass pill. */
             text-shadow:
                 0 1px 2px alpha(var(--art-shadow-color), 0.90),
                 0 0 10px alpha(var(--art-shadow-color), 0.62);
         }}
         .lyrics-line-button {{
             padding: 0;
             min-height: 0;
             border: none;
             background: none;
             box-shadow: none;
         }}
         .lyrics-line-button:focus-visible .lyrics-line {{
             box-shadow: inset 0 0 0 2px alpha(var(--accent-color), 0.46);
         }}
         .lyrics-current {{
             color: var(--accent-color);
             background-color: alpha(@window_bg_color, 0.56);
             background-image: linear-gradient(
                 120deg,
                 alpha(#ffffff, 0.16),
                 alpha(var(--glass-primary-color), 0.15) 48%,
                 alpha(var(--glass-secondary-color), 0.07)
             );
             font-weight: 800;
             box-shadow:
                 inset 0 1px alpha(#ffffff, 0.20),
                 inset 0 0 0 1px alpha(var(--glass-primary-color), 0.18),
                 0 9px 25px alpha(#000000, 0.09);
         }}
         .lyrics-plain-line {{
             padding: 8px 18px;
             font-size: 1.18em;
             font-weight: 500;
         }}
         .lyrics-live-source {{
             color: var(--accent-color);
             font-weight: 800;
         }}
         .lyrics-tools {{ margin-top: 4px; }}
         .lyrics-tool-pill,
         .lyrics-tool-picker > button {{
             min-height: 36px;
             border-radius: 12px;
             background-color: alpha(@window_fg_color, 0.075);
             background-image: linear-gradient(
                 145deg,
                 alpha(#ffffff, 0.13),
                 alpha(var(--glass-primary-color), 0.055)
             );
             box-shadow: inset 0 0 0 1px alpha(currentColor, 0.07);
         }}
         .lyrics-tool-pill {{ padding: 0 3px; }}

         /* Dragging a row in a reorderable list — the queue, and the sidebar's
            pinned playlists. The argument ARCHITECTURE.md asks for: libadwaita has no
            reorderable list, and a drop has to say where it will land before it
            happens. An inset shadow rather than a border, because a border would
            change the row's height and shove the list down 2px as the pointer
            crossed it.

            Named for what they do rather than for the queue: two lists reorder
            now, and a second copy of these three rules would be two places to
            change the colour. */
         .drop-above {{ box-shadow: inset 0 2px 0 var(--accent-bg-color); }}
         .drop-below {{ box-shadow: inset 0 -2px 0 var(--accent-bg-color); }}
         .dragging {{ opacity: 0.35; }}

         /* The volume panel's shape. GTK's `.osd` brings the colours — the
            translucent slab the Shell uses — and stops there, so the corners
            are square and it reads as a dialog rather than a readout.

            A pill is what every GNOME surface that shows a level uses, and
            there is no libadwaita widget for an OSD panel to inherit it from,
            which is the argument this rule needs. Two properties, no colours:
            whatever `.osd` resolves to in either theme still applies. */
         .volume-osd {{
             border-radius: 9999px;
             padding: 10px 18px;
         }}

         /* A button that has to be exactly as big as the 16px spinner it
            swaps with. GTK's own button metrics are built for a hit target,
            not for sitting inside a sidebar row, and the default padding
            alone made every library row taller. */
         .row-action {{
             padding: 0;
             min-width: 16px;
             min-height: 16px;
         }}

         /* The empty artwork slot, drawn as a case rather than left as a
            floating icon: with nothing playing the bar should still read as
            having a place the cover goes. The left edge is a touch lighter,
            which is enough to suggest a spine. */
         .np-cover-empty {{
             border-radius: 6px;
             background-image: linear-gradient(
                 to right,
                 alpha(currentColor, 0.16) 0%,
                 alpha(currentColor, 0.16) 3px,
                 alpha(currentColor, 0.07) 3px,
                 alpha(currentColor, 0.10) 100%
             );
             box-shadow: inset 0 0 0 1px alpha(currentColor, 0.12);
             color: alpha(currentColor, 0.45);
         }}

         /* Two grey bars where the title and artist go. Static, not pulsing:
            a pulsing skeleton would say something is loading, and nothing is. */
         .np-skeleton {{
             border-radius: 4px;
             background-color: alpha(currentColor, 0.13);
         }}"
    );

    BASE.with(|p| p.load_from_string(&css));
}

#[derive(Debug, Clone, Copy)]
struct MaterialOpacity {
    header: f32,
    sidebar: f32,
    feature: f32,
}

const CLEAR_FADE_START: u8 = 70;
const ADAPTIVE_TEXT_START: u8 = 77;
const ADAPTIVE_TEXT_FULL: u8 = 92;

/// Preserve the established 70% appearance, then ease every material layer to
/// zero together. The old curve stopped near half opacity at 99 and jumped to
/// zero at 100, which made the final slider step look like a mode switch.
fn glass_opacity(at_zero: f32, at_seventy: f32, strength: u8) -> f32 {
    let strength = strength.min(100);
    if strength <= CLEAR_FADE_START {
        let t = f32::from(strength) / f32::from(CLEAR_FADE_START);
        return at_zero + (at_seventy - at_zero) * t;
    }
    let t = f32::from(strength - CLEAR_FADE_START) / f32::from(100 - CLEAR_FADE_START);
    at_seventy * (1.0 - ease(t))
}

fn material_opacity(strength: u8) -> MaterialOpacity {
    MaterialOpacity {
        header: glass_opacity(0.82, 0.61, strength),
        sidebar: glass_opacity(0.84, 0.602, strength),
        feature: glass_opacity(0.68, 0.498, strength),
    }
}

/// Only the solid part of each glass surface. Highlights, borders and shadows
/// stay fixed, which preserves the material's shape while this layer becomes
/// clearer. The backdrop veil is updated separately by [`paint_backdrop`].
fn material_css(strength: u8) -> String {
    let opacity = material_opacity(strength);
    let clear_fill = if strength >= 100 {
        "background-image: none;"
    } else {
        ""
    };
    format!(
        ".jamelade-window headerbar {{
             background-color: alpha(var(--jamelade-headerbar-color), {:.3});
             {clear_fill}
         }}
         .jam-glass-sidebar {{
             background-color: alpha(@window_bg_color, {:.3});
             {clear_fill}
         }}
         .explore-hero, .lyrics-current {{
             background-color: alpha(@window_bg_color, {:.3});
             {clear_fill}
         }}",
        opacity.header, opacity.sidebar, opacity.feature
    )
}

/// Apply the live 0–100 transparency half of the glass control. The artwork's
/// real blur is regenerated off-thread by the app; this function handles only
/// cheap CSS and therefore follows the slider immediately.
pub fn set_glass_strength(strength: u8) {
    let strength = strength.min(100);
    GLASS_STRENGTH.with(|shown| shown.set(strength));
    MATERIAL.with(|provider| provider.load_from_string(&material_css(strength)));

    // The same cover, with a thinner or heavier legibility veil.
    repaint_shown_backdrop();
    refresh_adaptive_text();
}

thread_local! {
    /// The cover currently behind the window, so the next one has something to
    /// fade *from*.
    static SHOWN_ART: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
    /// Album colours actually painted in the current frame. Unlike the cover
    /// path this advances through every interpolated value, so rapidly skipping
    /// twice starts the second transition exactly where the first one stopped.
    static SHOWN_PALETTE: std::cell::Cell<Option<AlbumPalette>> =
        const { std::cell::Cell::new(None) };
    /// The coordinated cover-and-palette fade in flight, if any.
    static VISUAL_FADE: std::cell::RefCell<Option<gtk::glib::SourceId>> =
        const { std::cell::RefCell::new(None) };
}

/// How long a cover and its glass palette take to become the next track's, and
/// how often they repaint. Just under half a second reads as atmosphere
/// changing, without leaving the old album behind after a quick skip.
const FADE_MS: u64 = 480;
const FRAME_MS: u64 = 16;

fn cancel_visual_fade() {
    VISUAL_FADE.with(|fade| {
        if let Some(id) = fade.borrow_mut().take() {
            id.remove();
        }
    });
}

fn transition_image(
    from: &Option<std::path::PathBuf>,
    to: &Option<std::path::PathBuf>,
    fading: bool,
    percent: f32,
) -> Option<String> {
    if let (true, Some(from), Some(to)) = (fading, from.as_ref(), to.as_ref()) {
        Some(format!(
            "cross-fade({percent}% {}, {})",
            image_of(to),
            image_of(from)
        ))
    } else {
        to.as_deref().map(image_of)
    }
}

/// Put a cover behind the window and its colours through the glass surfaces,
/// or take both away.
///
/// Two layers, and the order matters: the artwork underneath, a scrim of the
/// window's own background over it. The scrim is why this is legible — every
/// label and icon on both surfaces has a colour chosen for contrast against
/// the theme, and a photograph behind them would be guessing. Taking the scrim
/// from `@window_bg_color` rather than from black is what makes the light
/// theme work too.
///
/// The two extracted colours do not replace the Jamkin accent: they tint only
/// the translucent material. Names and controls therefore keep their chosen
/// identity and contrast while the surrounding atmosphere follows the album.
pub fn set_track_visuals(path: Option<&std::path::Path>, palette: Option<AlbumPalette>) {
    // Whatever was in flight is now heading for the wrong cover and colours.
    cancel_visual_fade();

    let from = SHOWN_ART.with(|c| c.borrow().clone());
    let to = path.map(std::path::Path::to_path_buf);
    SHOWN_ART.with(|c| *c.borrow_mut() = to.clone());
    let from_palette = SHOWN_PALETTE.with(std::cell::Cell::get);

    let album_glass = ALBUM_GLASS_ON.with(std::cell::Cell::get);
    let fade_art = album_glass && matches!((&from, &to), (Some(from), Some(to)) if from != to);
    let fade_palette =
        album_glass && matches!((from_palette, palette), (Some(from), Some(to)) if from != to);
    if !fade_art && !fade_palette {
        // First cover, same cover, playback stopping, or no usable palette.
        // There is no pair to interpolate, so land on the honest state now.
        paint_backdrop(to.as_deref().map(image_of));
        paint_album_palette(palette);
        return;
    }
    if !fade_art {
        paint_backdrop(to.as_deref().map(image_of));
    }
    if !fade_palette {
        paint_album_palette(palette);
    }

    // One clock. The blurred cover and extracted colours are two readings of
    // the same sleeve; finishing apart makes the glass briefly look borrowed
    // from the wrong song.
    let start = std::time::Instant::now();
    let id = gtk::glib::timeout_add_local(std::time::Duration::from_millis(FRAME_MS), move || {
        let t = (start.elapsed().as_millis() as f32 / FADE_MS as f32).min(1.0);
        if t >= 1.0 {
            // Painted plainly at the end rather than as a 100% cross-fade, so
            // the settled state is one image and one url — and so a wrong guess
            // about which way `cross-fade` reads its percentage could only ever
            // be a fade in the wrong direction, never a wrong final frame.
            paint_backdrop(to.as_deref().map(image_of));
            paint_album_palette(palette);
            // Cleared here, not by the canceller: removing an already-finished
            // source logs a GLib critical.
            VISUAL_FADE.with(|f| *f.borrow_mut() = None);
            return gtk::glib::ControlFlow::Break;
        }
        let eased = ease(t);
        if fade_art {
            let pct = (eased * 100.0).round();
            paint_backdrop(transition_image(&from, &to, true, pct));
        }
        if fade_palette && let (Some(from), Some(to)) = (from_palette, palette) {
            paint_album_palette(Some(from.interpolate(to, eased)));
        }
        gtk::glib::ControlFlow::Continue
    });
    VISUAL_FADE.with(|f| *f.borrow_mut() = Some(id));
}

/// Replace only the blurred copy of the current cover. Used when the glass
/// slider crosses into a new blur radius; album-derived colours stay exactly
/// where their own transition left them.
pub fn set_backdrop_art(path: Option<&std::path::Path>) {
    cancel_visual_fade();
    let path = path.map(std::path::Path::to_path_buf);
    SHOWN_ART.with(|shown| *shown.borrow_mut() = path.clone());
    paint_backdrop(path.as_deref().map(image_of));
    refresh_adaptive_text();
}

/// Remember the current album palette, then paint either it or the selected
/// theme's own glass colours according to the single Album Liquid Glass switch.
fn paint_album_palette(palette: Option<AlbumPalette>) {
    SHOWN_PALETTE.with(|shown| shown.set(palette));
    refresh_album_palette();
    refresh_adaptive_text();
}

fn refresh_album_palette() {
    let album_glass = ALBUM_GLASS_ON.with(std::cell::Cell::get);
    let palette = SHOWN_PALETTE.with(std::cell::Cell::get);
    let theme = ACTIVE_THEME.with(std::cell::Cell::get);
    let css = glass_override_css(album_glass, palette, theme);
    GLASS.with(|provider| provider.load_from_string(&css));
}

fn glass_override_css(album_glass: bool, palette: Option<AlbumPalette>, theme: Theme) -> String {
    let colors = if album_glass {
        palette.map(|palette| (palette.primary.css(), palette.secondary.css()))
    } else {
        theme::glass_colors(theme)
            .map(|(primary, secondary)| (primary.to_owned(), secondary.to_owned()))
    };
    colors
        .map(|(primary, secondary)| {
            format!(
                ":root {{
                     --glass-primary-color: {primary};
                     --glass-secondary-color: {secondary};
                 }}"
            )
        })
        .unwrap_or_default()
}

fn adaptive_text_css(palette: AlbumPalette, blend: f32) -> String {
    let (foreground, shadow, drop_alpha, halo_alpha) = if palette.prefers_light_foreground() {
        ("#ffffff", "#08090b", 0.58, 0.26)
    } else {
        // A pale outline around dark type is especially visible on bright
        // covers. Keep only a quiet light bloom; the foreground already owns
        // the contrast on the artwork that selects this branch.
        ("#111216", "#ffffff", 0.28, 0.14)
    };
    let blend = blend.clamp(0.0, 1.0);
    format!(
        ":root {{
             --art-target-color: {foreground};
             --art-fg-color: mix(@window_fg_color, var(--art-target-color), {blend:.3});
             --art-shadow-color: {shadow};
         }}
         .art-foreground,
         .art-foreground headerbar {{
             color: var(--art-fg-color);
             text-shadow:
                 0 1px 3px alpha(var(--art-shadow-color), {:.3}),
                 0 0 12px alpha(var(--art-shadow-color), {:.3});
         }}
         .art-foreground headerbar button,
         .art-foreground headerbar button image {{
             color: var(--art-fg-color);
             -gtk-icon-shadow:
                 0 1px 2px alpha(var(--art-shadow-color), {:.3}),
                 0 0 7px alpha(var(--art-shadow-color), {:.3});
         }}
         .art-foreground .dim-label {{
             color: alpha(var(--art-fg-color), 0.78);
         }}
         /* The sidebar has its own selected-theme surface even when the main
            content is fully clear. Do not let the artwork halo inherit into
            that calm panel; it was the source of the crushed outlined labels. */
         .art-foreground .jam-glass-sidebar,
         .art-foreground .jam-glass-sidebar headerbar {{
             color: @window_fg_color;
             text-shadow: none;
         }}
         .art-foreground .jam-glass-sidebar headerbar button,
         .art-foreground .jam-glass-sidebar headerbar button image {{
             color: @window_fg_color;
             -gtk-icon-shadow: none;
         }}
         .art-foreground .jam-glass-sidebar .dim-label {{
             color: alpha(@window_fg_color, 0.73);
         }}",
        drop_alpha * blend,
        halo_alpha * blend,
        drop_alpha * 0.86 * blend,
        halo_alpha * 0.86 * blend,
    )
}

fn refresh_adaptive_text() {
    let strength = GLASS_STRENGTH.with(std::cell::Cell::get);
    let blend = adaptive_text_blend(strength);
    let clear_art = blend > 0.0
        && ALBUM_GLASS_ON.with(std::cell::Cell::get)
        && SHOWN_ART.with(|art| art.borrow().is_some());
    let css = clear_art
        .then(|| SHOWN_PALETTE.with(std::cell::Cell::get))
        .flatten()
        .map(|palette| adaptive_text_css(palette, blend))
        .unwrap_or_default();
    ADAPTIVE_TEXT.with(|provider| provider.load_from_string(&css));
}

fn adaptive_text_blend(strength: u8) -> f32 {
    if strength <= ADAPTIVE_TEXT_START {
        return 0.0;
    }
    if strength >= ADAPTIVE_TEXT_FULL {
        return 1.0;
    }
    let t = f32::from(strength - ADAPTIVE_TEXT_START)
        / f32::from(ADAPTIVE_TEXT_FULL - ADAPTIVE_TEXT_START);
    ease(t)
}

/// One cover as a CSS image.
fn image_of(path: &std::path::Path) -> String {
    format!("url(\"file://{}\")", path.display())
}

/// The backdrop rule. A CSS *image* in, CSS out — so the caller can hand over
/// one cover or a cross-fade of two and this does not care which.
///
/// The compact and expanded players are intentionally absent: both remain
/// stable selected-theme surfaces. How opaque the main-window veil is, top and
/// bottom, is still chosen per light or dark presentation.
///
/// One set of numbers cannot serve both, and that is not a matter of taste.
/// The veil is `@window_bg_color`, so at 0.86 the cover contributes the
/// remaining 14% either way; but 14% of a photograph over a *dark* window reads
/// as a coloured glow, and over a near-white one it is pastel mush. Rendered
/// side by side at the real 48px-upscaled blur, light needed roughly 0.60–0.70
/// where dark wants 0.86.
///
/// The floor is set by text, not by looks. Both surfaces carry labels in the
/// theme's own foreground colour — dark text in a light theme — so a veil thin
/// enough to show a sleeve's dark half is a veil thin enough to lose the words
/// on top of it. These are the strongest values that keep the type legible on
/// the covers this was checked against, not the prettiest ones available.
struct Veil {
    window: (f32, f32),
}

fn veil(dark: bool, strength: u8) -> Veil {
    if dark {
        Veil {
            window: (
                glass_opacity(0.82, 0.638, strength),
                glass_opacity(0.76, 0.592, strength),
            ),
        }
    } else {
        Veil {
            window: (
                glass_opacity(0.74, 0.60, strength),
                glass_opacity(0.68, 0.554, strength),
            ),
        }
    }
}

fn backdrop_css(image: Option<&str>, dark: bool, strength: u8) -> String {
    let Some(image) = image else {
        return String::new();
    };
    let veil = veil(dark, strength);
    let window = if strength >= 100 {
        format!(
            "background-image: {image};
             background-repeat: no-repeat;"
        )
    } else {
        format!(
            "background-image:
                 linear-gradient(
                     alpha(@window_bg_color, {}),
                     alpha(@window_bg_color, {})
                 ),
                 linear-gradient(
                     128deg,
                     alpha(var(--glass-primary-color), 0.13),
                     transparent 42%
                 ),
                 linear-gradient(
                     312deg,
                     alpha(var(--glass-secondary-color), 0.095),
                     transparent 46%
                 ),
                 {image};
             background-repeat: no-repeat;",
            veil.window.0, veil.window.1,
        )
    };
    format!(".jamelade-window {{ {window} }}")
}

/// Whether libadwaita is currently painting dark.
///
/// Asked at paint time rather than cached: the answer changes when the user
/// flips the preference *and* when the system does it under `ColorScheme::
/// Default`, and the second one never passes through our settings.
fn painting_dark() -> bool {
    adw::StyleManager::default().is_dark()
}

/// Let the current album own the cover and glass palette, or give both back to
/// the selected theme. Live from Preferences; the stored paths and palette mean
/// turning it back on needs no track change.
pub fn set_backdrop_enabled(on: bool) {
    ALBUM_GLASS_ON.with(|c| c.set(on));
    // Snapped, not faded. Fading a preference would say the app was thinking
    // about it; a switch should land the moment it is flipped.
    refresh_album_palette();
    repaint_shown_backdrop();
    refresh_adaptive_text();
}

fn repaint_shown_backdrop() {
    let image = SHOWN_ART.with(|shown| shown.borrow().clone());
    paint_backdrop(image.as_deref().map(image_of));
}

fn paint_backdrop(image: Option<String>) {
    // The one place every path funnels through — a track change, the fade's
    // per-frame repaint, the theme-flip handler — so the preference holds by
    // construction rather than at three call sites that could each forget it.
    let image = image.filter(|_| ALBUM_GLASS_ON.with(std::cell::Cell::get));
    let strength = GLASS_STRENGTH.with(std::cell::Cell::get);
    let css = backdrop_css(image.as_deref(), painting_dark(), strength);
    BACKDROP.with(|p| p.load_from_string(&css));
}

/// Ease in and out, so the fade does not start and stop abruptly.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_is_pinned_at_both_ends() {
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        // Out of range cannot overshoot: a late frame must not ask for a
        // cross-fade percentage outside the two covers it is between.
        assert_eq!(ease(-0.5), 0.0);
        assert_eq!(ease(2.0), 1.0);
    }

    #[test]
    fn the_drawer_is_always_a_normal_theme_surface() {
        assert!(PLAYER_SHEET_SURFACE.contains("background-color: @window_bg_color"));
        assert!(PLAYER_SHEET_SURFACE.contains("background-image: none"));
        assert!(!COVER_LAYOUT.contains(".np-sheet"));
        assert!(
            !adaptive_text_css(
                AlbumPalette {
                    primary: crate::palette::Rgb { r: 5, g: 6, b: 7 },
                    secondary: crate::palette::Rgb { r: 8, g: 9, b: 10 },
                },
                1.0,
            )
            .contains(".np-sheet")
        );
    }

    #[test]
    fn the_backdrop_is_static_and_centred() {
        // **The one that cost 20% of a core.** An `infinite` CSS animation
        // never lets GTK's frame clock stop — 119 fps on an idle window, #126.
        // The cross-fade between covers survives because it is a timer in
        // `set_track_visuals` that ends; nothing in the stylesheet may animate.
        //
        // And the position has to be stated, because it used to fall out of
        // that animation's keyframes: the CSS default is the top-left corner.
        let css = format!(
            "{COVER_LAYOUT}{}",
            backdrop_css(Some("url(\"file:///tmp/x.png\")"), true, 70)
        );
        assert!(
            !css.contains("animation") && !css.contains("keyframes"),
            "an animation here pins the frame clock open (#126): {css}"
        );
        assert!(css.contains("background-position"), "uncentred: {css}");
    }

    #[test]
    fn only_the_main_window_gets_the_cover() {
        let css = backdrop_css(Some("url(\"file:///tmp/x.png\")"), true, 70);
        assert!(
            css.contains(".jamelade-window"),
            "the main content was left out: {css}"
        );
        assert!(
            !css.contains(".np-bar"),
            "the compact player inherited album art: {css}"
        );
        assert!(
            !css.contains(".np-sheet"),
            "the drawer inherited album art: {css}"
        );
        assert_eq!(
            css.matches("url(\"file:///tmp/x.png\")").count(),
            1,
            "only the main window needs the image"
        );
        // The provider becomes empty so BASE's quiet Jamkin gradients become
        // visible again.
        let cleared = backdrop_css(None, true, 70);
        assert!(cleared.is_empty());
    }

    #[test]
    fn a_light_theme_gets_a_thinner_veil_than_a_dark_one() {
        // The bug this fixes: one set of numbers for both. The veil is
        // `@window_bg_color`, so 0.86 leaves the cover 14% either way — and 14%
        // of a photograph reads as a coloured glow over a dark window and as
        // pastel mush over a near-white one.
        let dark = backdrop_css(Some("url(\"a\")"), true, 70);
        let light = backdrop_css(Some("url(\"a\")"), false, 70);
        assert_ne!(dark, light, "both themes got the same veil");

        let dark = veil(true, 70);
        let light = veil(false, 70);
        let (dark_top, dark_bottom) = dark.window;
        let (light_top, light_bottom) = light.window;
        assert!(dark_top > dark_bottom, "the veil must thin downwards");
        assert!(light_top > light_bottom, "the veil must thin downwards");
        assert!(
            light.window.0 < dark.window.0,
            "light must let more of the cover through, not less"
        );
    }

    #[test]
    fn ordinary_glass_keeps_its_readability_floor() {
        // Up through the established default, system foreground colours still
        // sit on the same substantial veil. Beyond it the adaptive outlined
        // foreground takes over while the veil eases continuously to zero.
        for dark in [false, true] {
            for strength in [0, 70] {
                let veil = veil(dark, strength);
                let (top, bottom) = veil.window;
                assert!(bottom >= 0.5, "veil too thin for text: {bottom}");
                assert!(top <= 0.9, "veil so heavy the cover is invisible: {top}");
            }
        }
    }

    #[test]
    fn named_theme_owns_glass_and_header_when_album_glass_is_off() {
        let album = AlbumPalette {
            primary: crate::palette::Rgb { r: 1, g: 2, b: 3 },
            secondary: crate::palette::Rgb { r: 4, g: 5, b: 6 },
        };
        let on = glass_override_css(true, Some(album), Theme::Periwinkle);
        assert!(on.contains(&album.primary.css()));

        let off = glass_override_css(false, Some(album), Theme::Periwinkle);
        assert!(off.contains("#7c83c9"));
        assert!(off.contains("#ac8ad1"));
        assert!(!off.contains(&album.primary.css()));
        assert!(glass_override_css(false, Some(album), Theme::Light).is_empty());

        let material = material_css(70);
        assert!(material.contains("var(--jamelade-headerbar-color)"));
        assert!(
            !material
                .contains("headerbar {\n             background-color: alpha(@window_bg_color")
        );
    }

    #[test]
    fn the_clear_end_of_the_slider_has_no_opacity_cliff() {
        for dark in [false, true] {
            let almost = veil(dark, 99);
            let clear = veil(dark, 100);
            let (near_top, near_bottom) = almost.window;
            assert!(near_top > 0.0 && near_top < 0.01);
            assert!(near_bottom > 0.0 && near_bottom < 0.01);
            assert_eq!(clear.window, (0.0, 0.0));
        }
        let almost = material_opacity(99);
        assert!(almost.header > 0.0 && almost.header < 0.01);
        assert_eq!(material_opacity(100).header, 0.0);
        assert_eq!(adaptive_text_blend(77), 0.0);
        assert!(adaptive_text_blend(78) > 0.0);
    }

    #[test]
    fn the_explicit_full_setting_is_actually_clear() {
        let opacity = material_opacity(100);
        assert_eq!(opacity.header, 0.0);
        assert_eq!(opacity.sidebar, 0.0);
        assert_eq!(opacity.feature, 0.0);
        for dark in [false, true] {
            let veil = veil(dark, 100);
            assert_eq!(veil.window, (0.0, 0.0));
        }
        assert!(material_css(100).contains("background-image: none"));
        let backdrop = backdrop_css(Some("url(\"cover\")"), true, 100);
        assert_eq!(backdrop.matches("url(\"cover\")").count(), 1);
        assert!(!backdrop.contains("--glass-primary-color"));
        assert!(!backdrop.contains("--glass-secondary-color"));
    }

    #[test]
    fn the_compact_player_never_receives_a_cover_image() {
        let backdrop = backdrop_css(Some("url(\"clear-cover\")"), true, 100);
        assert_eq!(backdrop.matches("url(\"clear-cover\")").count(), 1);
        assert!(!backdrop.contains(".np-bar"));
        assert!(
            !adaptive_text_css(
                AlbumPalette {
                    primary: crate::palette::Rgb { r: 5, g: 6, b: 7 },
                    secondary: crate::palette::Rgb { r: 8, g: 9, b: 10 },
                },
                1.0,
            )
            .contains(".np-bar")
        );
    }

    #[test]
    fn adaptive_text_uses_opposing_foreground_and_a_soft_halo() {
        let dark = adaptive_text_css(
            AlbumPalette {
                primary: crate::palette::Rgb { r: 8, g: 9, b: 12 },
                secondary: crate::palette::Rgb {
                    r: 24,
                    g: 18,
                    b: 20,
                },
            },
            1.0,
        );
        assert!(dark.contains("--art-target-color: #ffffff"));
        assert!(dark.contains("--art-fg-color: mix("));
        assert!(dark.contains("--art-shadow-color: #08090b"));
        assert!(dark.contains("0 1px 3px"));
        assert!(dark.contains("0 0 12px"));
        assert!(!dark.contains("-1px -1px"));
        assert!(dark.contains(".art-foreground .jam-glass-sidebar"));
        assert!(dark.contains("text-shadow: none"));

        let light = adaptive_text_css(
            AlbumPalette {
                primary: crate::palette::Rgb {
                    r: 246,
                    g: 240,
                    b: 228,
                },
                secondary: crate::palette::Rgb {
                    r: 220,
                    g: 232,
                    b: 244,
                },
            },
            1.0,
        );
        assert!(light.contains("--art-target-color: #111216"));
        assert!(light.contains("--art-shadow-color: #ffffff"));
    }

    #[test]
    fn adaptive_text_blends_smoothly_before_becoming_opaque() {
        let samples = [77, 78, 80, 85, 90, 92].map(adaptive_text_blend);
        assert_eq!(samples[0], 0.0);
        assert_eq!(samples[5], 1.0);
        assert!(samples.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(samples[1] < 0.05, "the first visible step must be subtle");
        assert!((0.45..0.65).contains(&samples[3]));
    }

    #[test]
    fn a_stronger_setting_makes_the_material_clearer() {
        let subtle = material_opacity(0);
        let liquid = material_opacity(100);
        assert!(liquid.header < subtle.header);
        assert!(liquid.sidebar < subtle.sidebar);
        assert!(liquid.feature < subtle.feature);

        let subtle = veil(true, 0);
        let liquid = veil(true, 100);
        assert!(liquid.window.0 < subtle.window.0);
        assert!(liquid.window.1 < subtle.window.1);
    }

    #[test]
    fn sf_pro_is_the_preferred_ui_font_with_linux_fallbacks() {
        assert!(UI_FONT_STACK.starts_with("\"SF Pro Display\""));
        assert!(UI_FONT_STACK.contains("system-ui"));
        assert!(UI_FONT_STACK.ends_with("sans-serif"));
    }

    #[test]
    fn a_cover_is_a_file_url() {
        let css = image_of(std::path::Path::new("/tmp/a-b.backdrop.png"));
        assert_eq!(css, "url(\"file:///tmp/a-b.backdrop.png\")");
    }
}
