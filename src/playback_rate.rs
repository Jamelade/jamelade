// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The small playback-speed vocabulary shared by the player and its UI.
//!
//! MusicKit accepts an arbitrary floating-point rate, but Jamelade deliberately
//! exposes only bounded tenths. That keeps the browser command predictable and
//! avoids sending values a media element may clamp differently by platform.

pub const DEFAULT: f64 = 1.0;
pub const MIN: f64 = 0.5;
pub const MAX: f64 = 2.0;
pub const STEP: f64 = 0.1;

pub fn normalize(rate: f64) -> Option<f64> {
    if !rate.is_finite() {
        return None;
    }
    let rounded = (rate * 10.0).round() / 10.0;
    ((MIN..=MAX).contains(&rounded) && (rate - rounded).abs() < 0.000_001).then_some(rounded)
}

pub fn label(rate: f64) -> String {
    let rate = normalize(rate).unwrap_or(DEFAULT);
    if rate == DEFAULT {
        "1×".into()
    } else if rate == MAX {
        "2×".into()
    } else {
        format!("{rate:.1}×")
    }
}

pub fn from_slider(rate: f64) -> f64 {
    ((rate.clamp(MIN, MAX) * 10.0).round() / 10.0).clamp(MIN, MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_bounded_tenths_are_accepted() {
        for tenth in 5..=20 {
            let rate = f64::from(tenth) / 10.0;
            assert_eq!(normalize(rate), Some(rate));
        }
        for rate in [f64::NAN, f64::INFINITY, 0.0, 0.95, 2.1, 4.0] {
            assert_eq!(normalize(rate), None);
        }
    }

    #[test]
    fn compact_labels_fit_the_transport_cell() {
        assert_eq!(label(0.5), "0.5×");
        assert_eq!(label(1.0), "1×");
        assert_eq!(label(1.7), "1.7×");
        assert_eq!(label(2.0), "2×");
    }

    #[test]
    fn slider_values_snap_to_tenths() {
        assert_eq!(from_slider(0.54), 0.5);
        assert_eq!(from_slider(1.26), 1.3);
        assert_eq!(from_slider(9.0), 2.0);
    }
}
