// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The selectable Jamkin companion and its local-only artwork.

use std::path::PathBuf;

const ANIMATION_FRAMES: usize = 6;

/// The companion shown beside lyrics. Stored by id rather than display name so
/// a future copy edit does not reset somebody's choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[allow(clippy::enum_variant_names)] // The shared Jam prefix is the character-family name.
pub enum Companion {
    #[default]
    JamBun,
    JamPam,
    JamJoe,
}

/// Colours that tie the selected companion to the rest of the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub accent: &'static str,
    pub foreground: &'static str,
    pub secondary: &'static str,
}

impl Companion {
    pub const ALL: [Self; 3] = [Self::JamBun, Self::JamPam, Self::JamJoe];

    pub fn label(self) -> &'static str {
        match self {
            Self::JamBun => "JamBun",
            Self::JamPam => "JamPam",
            Self::JamJoe => "JamJoe",
        }
    }

    pub fn personality(self) -> &'static str {
        match self {
            Self::JamBun => "Cheerful and excitable",
            Self::JamPam => "Confident and elegant",
            Self::JamJoe => "Mellow and cozy",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::JamBun => "jambun",
            Self::JamPam => "jampam",
            Self::JamJoe => "jamjoe",
        }
    }

    pub fn window_icon_name(self) -> &'static str {
        #[cfg(not(feature = "broker-test"))]
        match self {
            Self::JamBun => "io.github.Jamelade.Jamelade.jambun",
            Self::JamPam => "io.github.Jamelade.Jamelade.jampam",
            Self::JamJoe => "io.github.Jamelade.Jamelade.jamjoe",
        }
        #[cfg(feature = "broker-test")]
        match self {
            Self::JamBun => "io.github.Jamelade.Jamelade.BrokerTest.jambun",
            Self::JamPam => "io.github.Jamelade.Jamelade.BrokerTest.jampam",
            Self::JamJoe => "io.github.Jamelade.Jamelade.BrokerTest.jamjoe",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            // Keep the old id as a one-way settings migration. The next save
            // writes `jampam`, so existing installs keep their chosen Jamkin
            // without preserving the retired name in new state.
            "jampam" | "jamila" => Self::JamPam,
            "jamjoe" => Self::JamJoe,
            _ => Self::JamBun,
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

    pub fn palette(self) -> Palette {
        match self {
            Self::JamBun => Palette {
                accent: "#a51d4d",
                foreground: "#ffffff",
                secondary: "#d99a28",
            },
            Self::JamPam => Palette {
                accent: "#744a9e",
                foreground: "#ffffff",
                secondary: "#b61f55",
            },
            Self::JamJoe => Palette {
                // Drawn from the saturated orange in his headphones rather
                // than the old muted brown, while retaining AA contrast both
                // as text on a light surface and behind white control glyphs.
                accent: "#b95710",
                foreground: "#ffffff",
                secondary: "#b51f52",
            },
        }
    }

    fn filename(self) -> &'static str {
        match self {
            Self::JamBun => "jambun.png",
            Self::JamPam => "jampam.png",
            Self::JamJoe => "jamjoe.png",
        }
    }

    #[cfg(debug_assertions)]
    fn local_data_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/companions")
    }

    fn installed_data_roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
            if !dir.is_empty() {
                roots.push(PathBuf::from(dir));
            }
        } else if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join(".local/share"));
        }
        match std::env::var_os("XDG_DATA_DIRS") {
            Some(dirs) if !dirs.is_empty() => roots.extend(std::env::split_paths(&dirs)),
            _ => roots.extend([
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]),
        }
        roots
    }

    fn data_roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Some(dir) = std::env::var_os("JAMELADE_COMPANION_DIR") {
            let root = PathBuf::from(dir);
            if root.is_dir() {
                roots.push(root);
            }
        }

        #[cfg(debug_assertions)]
        {
            let root = Self::local_data_root();
            if root.is_dir() {
                roots.push(root);
            }
        }

        roots.extend(
            Self::installed_data_roots()
                .into_iter()
                .map(|root| root.join("jamelade/companions"))
                .filter(|root| root.is_dir()),
        );
        roots
    }

    /// Locate artwork in a development tree or an installed XDG data root.
    /// The images ship with Jamelade; selecting a companion never reaches the
    /// network and never touches Apple or lyric credentials.
    pub fn image_path(self) -> Option<PathBuf> {
        Self::data_roots()
            .into_iter()
            .map(|root| root.join(self.filename()))
            .find(|path| path.is_file())
    }

    /// The rounded square used by the desktop's launcher portal. Separate from
    /// the transparent actor art so choosing a tile never boxes in the pet.
    pub fn launcher_icon_path(self) -> Option<PathBuf> {
        Self::data_roots()
            .into_iter()
            .map(|root| root.join("launcher").join(self.filename()))
            .find(|path| path.is_file())
    }

    /// The small, fixed-canvas animation shipped beside the static portrait.
    /// All frames must be present or the actor falls back to the portrait; a
    /// half-installed loop would visibly flash an empty frame.
    pub fn animation_frame_paths(self, high_resolution: bool) -> Option<Vec<PathBuf>> {
        Self::data_roots().into_iter().find_map(|root| {
            let set = if high_resolution {
                "animated-hq"
            } else {
                "animated"
            };
            let root = root.join(set).join(self.id());
            let frames: Vec<_> = (0..ANIMATION_FRAMES)
                .map(|index| root.join(format!("frame-{index:02}.png")))
                .collect();
            frames.iter().all(|path| path.is_file()).then_some(frames)
        })
    }

    pub const fn animation_interval_ms(self) -> u64 {
        match self {
            Self::JamBun => 125,
            Self::JamPam => 145,
            Self::JamJoe => 175,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn png_dimensions(path: &std::path::Path) -> (u32, u32) {
        let mut header = [0_u8; 24];
        std::fs::File::open(path)
            .and_then(|mut file| file.read_exact(&mut header))
            .expect("read bundled PNG header");
        assert_eq!(&header[..8], b"\x89PNG\r\n\x1a\n");
        (
            u32::from_be_bytes(header[16..20].try_into().unwrap()),
            u32::from_be_bytes(header[20..24].try_into().unwrap()),
        )
    }

    fn luminance(hex: &str) -> f64 {
        let channel = |at| {
            let value =
                u8::from_str_radix(&hex[at..at + 2], 16).expect("hex colour") as f64 / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
    }

    #[test]
    fn companion_ids_round_trip() {
        for companion in Companion::ALL {
            assert_eq!(Companion::parse(companion.id()), companion);
            assert_eq!(Companion::from_index(companion.index()), companion);
            assert!(companion.window_icon_name().starts_with(crate::APP_ID));
        }
    }

    #[test]
    fn the_retired_jamila_id_migrates_to_jampam() {
        assert_eq!(Companion::parse("jamila"), Companion::JamPam);
        assert_eq!(Companion::parse("jamila").id(), "jampam");
    }

    #[test]
    fn unknown_companions_fall_back_to_jambun() {
        assert_eq!(Companion::parse(""), Companion::JamBun);
        assert_eq!(Companion::parse("future-jamkin"), Companion::JamBun);
        assert_eq!(Companion::from_index(99), Companion::JamBun);
    }

    #[test]
    fn every_palette_has_an_accessible_dark_accent() {
        for companion in Companion::ALL {
            let palette = companion.palette();
            assert!(palette.accent.starts_with('#'));
            assert_eq!(palette.foreground, "#ffffff");
            assert!(palette.secondary.starts_with('#'));
            let contrast_with_white = 1.05 / (luminance(palette.accent) + 0.05);
            assert!(
                contrast_with_white >= 4.5,
                "{} has only {contrast_with_white:.2}:1 contrast",
                companion.label()
            );
        }
    }

    #[test]
    fn every_jamkin_has_its_own_accent() {
        let accents: std::collections::HashSet<_> = Companion::ALL
            .iter()
            .map(|companion| companion.palette().accent)
            .collect();
        assert_eq!(accents.len(), Companion::ALL.len());
        assert_eq!(Companion::JamJoe.palette().accent, "#b95710");
    }

    #[test]
    fn bundled_animation_sets_are_complete() {
        for companion in Companion::ALL {
            for (high_resolution, expected_size) in [(false, 320), (true, 1280)] {
                let frames = companion
                    .animation_frame_paths(high_resolution)
                    .unwrap_or_else(|| panic!("{} animation is incomplete", companion.label()));
                assert_eq!(frames.len(), ANIMATION_FRAMES);
                for frame in frames {
                    assert_eq!(png_dimensions(&frame), (expected_size, expected_size));
                }
            }
        }
    }

    #[test]
    fn every_companion_has_a_launcher_tile() {
        for companion in Companion::ALL {
            let path = companion
                .launcher_icon_path()
                .unwrap_or_else(|| panic!("{} has no launcher tile", companion.label()));
            assert_eq!(
                png_dimensions(&path),
                (512, 512),
                "{} launcher tile must satisfy the desktop portal's 512 px limit",
                companion.label()
            );
        }
    }
}
