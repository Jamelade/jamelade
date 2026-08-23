// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The local Jamkin asset-quality preference and its display-size policy.

/// Which bundled animation masters the Jamkin actor should decode. Automatic
/// keeps the original small set on ordinary displays, then opts into the
/// locally upscaled set only where those extra pixels can actually be seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JamkinQuality {
    #[default]
    Auto,
    High,
    Performance,
}

impl JamkinQuality {
    pub(super) const ALL: [Self; 3] = [Self::Auto, Self::High, Self::Performance];

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::High => "high",
            Self::Performance => "performance",
        }
    }

    pub(super) fn parse(value: &str) -> Self {
        match value {
            "high" => Self::High,
            "performance" => Self::Performance,
            _ => Self::Auto,
        }
    }

    pub fn index(self) -> u32 {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap_or_default() as u32
    }

    pub fn from_index(index: u32) -> Self {
        Self::ALL.get(index as usize).copied().unwrap_or_default()
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Auto => "High resolution on HiDPI displays or for large Jamkins",
            Self::High => "Always use the locally upscaled 1280 px animation frames",
            Self::Performance => "Use the original 320 px frames and the least memory",
        }
    }

    pub fn uses_high_resolution(self, size: i32, scale_factor: i32) -> bool {
        match self {
            Self::Auto => size > 320 || scale_factor > 1,
            Self::High => true,
            Self::Performance => false,
        }
    }
}
