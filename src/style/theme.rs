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
            window: "#ffe3ee",
            view: "#fff1f7",
            surface: "#f4cbdc",
            card: "#fff4f8",
            glass_primary: "#d9447a",
            glass_secondary: "#f0749b",
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
            window: "#d5f3ed",
            view: "#e9faf6",
            surface: "#bfe8df",
            card: "#effbf8",
            glass_primary: "#158f83",
            glass_secondary: "#42b9aa",
            foreground: "#112d2a",
            shade: "rgba(20, 82, 75, 0.17)",
        },
        Theme::Vermilion => Palette {
            window: "#ffe1d8",
            view: "#fff0eb",
            surface: "#f5c3b5",
            card: "#fff3ee",
            glass_primary: "#dc3e27",
            glass_secondary: "#f0713c",
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
             --theme-glass-primary-color: {glass_primary};
             --theme-glass-secondary-color: {glass_secondary};
         }}
         .jamelade-window {{
             background-color: {window};
             color: {foreground};
         }}
         .jamelade-window:backdrop .jam-glass-sidebar,
         .jam-glass-sidebar:backdrop {{
             background-color: alpha({window}, 0.76);
             background-image:
                 radial-gradient(
                     circle at 18% 4%,
                     alpha({glass_primary}, 0.24),
                     transparent 43%
                 ),
                 linear-gradient(
                     158deg,
                     alpha(#ffffff, 0.14),
                     alpha({glass_primary}, 0.075) 48%,
                     alpha({glass_secondary}, 0.09)
                 );
             color: {foreground};
         }}
         .jamelade-window:backdrop .jam-glass-sidebar headerbar,
         .jam-glass-sidebar headerbar:backdrop {{
             background-color: alpha({surface}, 0.78);
             color: {foreground};
         }}
         .jamelade-window:backdrop .np-row,
         .np-row:backdrop {{
             background-color: {window};
             background-image: linear-gradient(
                 145deg,
                 alpha(#ffffff, 0.13),
                 transparent 42%,
                 alpha({glass_primary}, 0.09)
             );
             color: {foreground};
         }}",
        window = palette.window,
        view = palette.view,
        surface = palette.surface,
        card = palette.card,
        foreground = palette.foreground,
        shade = palette.shade,
        glass_primary = palette.glass_primary,
        glass_secondary = palette.glass_secondary,
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
            assert!(css.contains("--theme-glass-primary-color"));
            assert!(css.contains(".jamelade-window"));
            assert!(css.contains(".jam-glass-sidebar:backdrop"));
            assert!(css.contains(".np-row:backdrop"));
            assert!(glass_colors(theme).is_some());
        }
        for theme in [Theme::Light, Theme::Dark, Theme::System] {
            assert!(css(theme).is_empty());
            assert!(glass_colors(theme).is_none());
        }
    }
}
