// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Persisted window themes and their presentation order.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
    Azure,
    Blossom,
    Ember,
    Forest,
    Marigold,
    Periwinkle,
    Tidepool,
    Vermilion,
}

impl Theme {
    /// The two neutral foundations lead, then named palettes alphabetically.
    /// Follow System remains at the end for existing users.
    pub const ALL: [Self; 11] = [
        Self::Light,
        Self::Dark,
        Self::Azure,
        Self::Blossom,
        Self::Ember,
        Self::Forest,
        Self::Marigold,
        Self::Periwinkle,
        Self::Tidepool,
        Self::Vermilion,
        Self::System,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::Azure => "Azure",
            Self::Blossom => "Blossom",
            Self::Ember => "Ember",
            Self::Forest => "Forest",
            Self::Marigold => "Marigold",
            Self::Periwinkle => "Periwinkle",
            Self::Tidepool => "Tidepool",
            Self::Vermilion => "Vermilion",
            Self::System => "Follow System",
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
            Self::Azure => "azure",
            Self::Blossom => "blossom",
            Self::Ember => "ember",
            Self::Forest => "forest",
            Self::Marigold => "marigold",
            Self::Periwinkle => "periwinkle",
            Self::Tidepool => "tidepool",
            Self::Vermilion => "vermilion",
        }
    }

    pub(super) fn parse(value: &str) -> Self {
        match value {
            "light" => Self::Light,
            "dark" => Self::Dark,
            "azure" => Self::Azure,
            "blossom" => Self::Blossom,
            "ember" => Self::Ember,
            "forest" => Self::Forest,
            "marigold" => Self::Marigold,
            "periwinkle" => Self::Periwinkle,
            "tidepool" => Self::Tidepool,
            "vermilion" => Self::Vermilion,
            _ => Self::System,
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

    pub(super) fn prefers_dark(self) -> Option<bool> {
        match self {
            Self::System => None,
            Self::Dark | Self::Ember | Self::Forest | Self::Periwinkle => Some(true),
            Self::Light
            | Self::Azure
            | Self::Blossom
            | Self::Marigold
            | Self::Tidepool
            | Self::Vermilion => Some(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_round_trip_through_storage_and_combo_indices() {
        for theme in Theme::ALL {
            assert_eq!(Theme::parse(theme.as_str()), theme);
            assert_eq!(Theme::from_index(theme.index()), theme);
        }
    }

    #[test]
    fn invalid_values_fall_back_to_follow_system() {
        assert_eq!(Theme::parse("solarized"), Theme::System);
        assert_eq!(Theme::parse(""), Theme::System);
        assert_eq!(Theme::from_index(99), Theme::System);
    }

    #[test]
    fn menu_starts_light_dark_then_named_palettes_alphabetically() {
        let labels: Vec<_> = Theme::ALL.iter().map(|theme| theme.label()).collect();
        assert_eq!(labels[0..2], ["Light", "Dark"]);
        assert_eq!(
            labels[2..10],
            [
                "Azure",
                "Blossom",
                "Ember",
                "Forest",
                "Marigold",
                "Periwinkle",
                "Tidepool",
                "Vermilion",
            ]
        );
        assert_eq!(labels[10], "Follow System");
    }
}
