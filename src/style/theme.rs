// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Named surface palettes, independent of the user-selected accent.

use crate::settings::Theme;

#[derive(Clone, Copy)]
struct Palette {
    window: &'static str,
    view: &'static str,
    surface: &'static str,
    card: &'static str,
    glass_primary: &'static str,
    glass_secondary: &'static str,
    foreground: &'static str,
    shade: &'static str,
}

fn palette(theme: Theme) -> Option<Palette> {
    Some(match theme {
        Theme::Azure => Palette {
            window: "#eaf6ff",
            view: "#f5fbff",
            surface: "#dceefa",
            card: "#f8fcff",
            glass_primary: "#59a7d8",
            glass_secondary: "#8dd8f0",
            foreground: "#132634",
            shade: "rgba(14, 54, 78, 0.18)",
        },
        Theme::Blossom => Palette {
            window: "#fff0f5",
            view: "#fff8fb",
            surface: "#f7e0e9",
            card: "#fff9fb",
            glass_primary: "#d98cab",
            glass_secondary: "#f0b6c8",
            foreground: "#37212a",
            shade: "rgba(91, 38, 61, 0.17)",
        },
        Theme::Ember => Palette {
            window: "#2a1d18",
            view: "#33231c",
            surface: "#3a271f",
            card: "#422c22",
            glass_primary: "#c26e3a",
            glass_secondary: "#e1a25c",
            foreground: "#fff2e8",
            shade: "rgba(0, 0, 0, 0.46)",
        },
        Theme::Forest => Palette {
            window: "#17251e",
            view: "#1d2d25",
            surface: "#21342a",
            card: "#273a30",
            glass_primary: "#3f8b68",
            glass_secondary: "#81a76b",
            foreground: "#edf7f1",
            shade: "rgba(0, 0, 0, 0.44)",
        },
        Theme::Marigold => Palette {
            window: "#fff5d8",
            view: "#fffaf0",
            surface: "#f3e5b9",
            card: "#fffaf0",
            glass_primary: "#d6a328",
            glass_secondary: "#f0c85a",
            foreground: "#332710",
            shade: "rgba(91, 66, 10, 0.18)",
        },
        Theme::Periwinkle => Palette {
            window: "#23243c",
            view: "#2b2c49",
            surface: "#303151",
            card: "#38395a",
            glass_primary: "#7c83c9",
            glass_secondary: "#ac8ad1",
            foreground: "#f2f1ff",
            shade: "rgba(0, 0, 0, 0.45)",
        },
        Theme::Tidepool => Palette {
            window: "#e5f7f3",
            view: "#f2fbf9",
            surface: "#d4eee8",
            card: "#f5fcfa",
            glass_primary: "#3fa89c",
            glass_secondary: "#76c9bc",
            foreground: "#112d2a",
            shade: "rgba(20, 82, 75, 0.17)",
        },
        Theme::Vermilion => Palette {
            window: "#fff0eb",
            view: "#fff8f5",
            surface: "#f8ded6",
            card: "#fff8f4",
            glass_primary: "#e15b45",
            glass_secondary: "#ef9a62",
            foreground: "#351b16",
            shade: "rgba(107, 43, 29, 0.18)",
        },
        Theme::System | Theme::Light | Theme::Dark => return None,
    })
}

pub(super) fn glass_colors(theme: Theme) -> Option<(&'static str, &'static str)> {
    palette(theme).map(|palette| (palette.glass_primary, palette.glass_secondary))
}

pub(super) fn css(theme: Theme) -> String {
    let Some(palette) = palette(theme) else {
        return String::new();
    };
    format!(
        "@define-color window_bg_color {window};
         @define-color window_fg_color {foreground};
         @define-color view_bg_color {view};
         @define-color view_fg_color {foreground};
         @define-color headerbar_bg_color {surface};
         @define-color headerbar_fg_color {foreground};
         @define-color sidebar_bg_color {surface};
         @define-color sidebar_fg_color {foreground};
         @define-color card_bg_color {card};
         @define-color card_fg_color {foreground};
         @define-color popover_bg_color {surface};
         @define-color popover_fg_color {foreground};
         @define-color dialog_bg_color {window};
         @define-color dialog_fg_color {foreground};
         @define-color shade_color {shade};
         :root {{
             --window-bg-color: {window};
             --window-fg-color: {foreground};
             --view-bg-color: {view};
             --view-fg-color: {foreground};
             --headerbar-bg-color: {surface};
             --headerbar-fg-color: {foreground};
             --sidebar-bg-color: {surface};
             --sidebar-fg-color: {foreground};
             --card-bg-color: {card};
             --card-fg-color: {foreground};
             --popover-bg-color: {surface};
             --popover-fg-color: {foreground};
             --dialog-bg-color: {window};
             --dialog-fg-color: {foreground};
             --shade-color: {shade};
             --jamelade-headerbar-color: {surface};
         }}
         .jamelade-window {{
             background-color: {window};
             color: {foreground};
         }}",
        window = palette.window,
        view = palette.view,
        surface = palette.surface,
        card = palette.card,
        foreground = palette.foreground,
        shade = palette.shade,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_themes_have_surface_palettes_and_neutral_themes_do_not() {
        for theme in [
            Theme::Azure,
            Theme::Blossom,
            Theme::Ember,
            Theme::Forest,
            Theme::Marigold,
            Theme::Periwinkle,
            Theme::Tidepool,
            Theme::Vermilion,
        ] {
            let css = css(theme);
            assert!(css.contains("@define-color window_bg_color"));
            assert!(css.contains("--window-bg-color"));
            assert!(css.contains("--jamelade-headerbar-color"));
            assert!(css.contains(".jamelade-window"));
            assert!(glass_colors(theme).is_some());
        }
        for theme in [Theme::Light, Theme::Dark, Theme::System] {
            assert!(css(theme).is_empty());
            assert!(glass_colors(theme).is_none());
        }
    }
}
