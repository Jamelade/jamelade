// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Live lyric colour and type-scale providers.
//!
//! They are deliberately independent of the large base stylesheet. Moving a
//! preference slider reparses only these selectors, not every surface rule.

use relm4::gtk::{self, gdk};

thread_local! {
    static ACCENT: gtk::CssProvider = gtk::CssProvider::new();
    static FONT: gtk::CssProvider = gtk::CssProvider::new();
}

pub(super) fn install(display: &gdk::Display, accent_strength: u8, font_scale: u8) {
    ACCENT.with(|provider| {
        gtk::style_context_add_provider_for_display(
            display,
            provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 4,
        );
    });
    FONT.with(|provider| {
        gtk::style_context_add_provider_for_display(
            display,
            provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 5,
        );
    });
    set_accent_strength(accent_strength);
    set_font_scale(font_scale);
}

/// Recolour only the lines immediately around the live timestamp.
///
/// Earlier lines stay neutral and faded; distant future lines stay neutral at
/// their existing opacity. The asymmetric factors make the next line easier
/// to anticipate than the line that has already passed.
pub fn set_accent_strength(strength: u8) {
    let (previous, next, following) = accent_factors(strength);
    // GTK's `mix(first, second, amount)` weights the second colour. Keeping the
    // neutral foreground first makes larger values mean more Jamkin colour.
    let css = format!(
        ".lyrics-previous {{ color: mix(@window_fg_color, var(--accent-color), {previous:.3}); }}
         .lyrics-next {{ color: mix(@window_fg_color, var(--accent-color), {next:.3}); }}
         .lyrics-following {{ color: mix(@window_fg_color, var(--accent-color), {following:.3}); }}"
    );
    ACCENT.with(|provider| provider.load_from_string(&css));
}

/// Scale only lyric copy. Navigation, player controls and Jamkin art keep
/// their native sizes, while both the full view and hover bubble stay aligned.
pub fn set_font_scale(scale: u8) {
    let scale = f64::from(scale.clamp(
        crate::settings::MIN_LYRICS_FONT_SCALE,
        crate::settings::MAX_LYRICS_FONT_SCALE,
    )) / 100.0;
    FONT.with(|provider| provider.load_from_string(&font_css(scale)));
}

fn accent_factors(strength: u8) -> (f64, f64, f64) {
    let strength = f64::from(strength.min(100)) / 100.0;
    (strength * 0.45, strength * 0.80, strength * 0.55)
}

fn font_css(scale: f64) -> String {
    format!(
        ".lyrics-line {{ font-size: {:.3}em; }}
         .lyrics-plain-line {{ font-size: {:.3}em; }}
         .jamkin-bubble-current {{ font-size: {:.3}em; }}
         .jamkin-bubble-next {{ font-size: {:.3}em; }}",
        1.42 * scale,
        1.18 * scale,
        1.08 * scale,
        0.90 * scale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearby_colour_is_asymmetric_and_bounded() {
        assert_eq!(accent_factors(0), (0.0, 0.0, 0.0));
        let (previous, next, following) = accent_factors(100);
        assert!(previous < following && following < next);
        assert_eq!((previous, next, following), (0.45, 0.80, 0.55));
        assert_eq!(accent_factors(u8::MAX), (previous, next, following));
    }

    #[test]
    fn normal_font_scale_preserves_the_designed_sizes() {
        let css = font_css(1.0);
        assert!(css.contains("1.420em"));
        assert!(css.contains("1.180em"));
        assert!(css.contains("1.080em"));
        assert!(css.contains("0.900em"));
    }

    #[test]
    fn font_scale_changes_every_lyric_surface_together() {
        let small = font_css(0.8);
        let large = font_css(1.6);
        for class in [
            ".lyrics-line",
            ".lyrics-plain-line",
            ".jamkin-bubble-current",
            ".jamkin-bubble-next",
        ] {
            assert!(small.contains(class));
            assert!(large.contains(class));
        }
        assert_ne!(small, large);
    }
}
