// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A small, deterministic album-art palette for Jamelade's glass surfaces.
//!
//! Extraction runs beside artwork decoding, never on the GTK thread. The
//! result is cached next to the already-private cover and contains only two
//! RGB values — no title, account id, URL or Apple credential.

use std::path::{Path, PathBuf};

use relm4::gtk::gdk_pixbuf;

const QUANT: usize = 16;
const BUCKETS: usize = QUANT * QUANT * QUANT;
const CACHE_VERSION: &str = "v1";
const SAMPLE_PX: i32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub fn css(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let channel = |a: u8, b: u8| {
            (f32::from(a) + (f32::from(b) - f32::from(a)) * t)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Self {
            r: channel(self.r, other.r),
            g: channel(self.g, other.g),
            b: channel(self.b, other.b),
        }
    }

    fn luma(self) -> f32 {
        (0.2126 * f32::from(self.r) + 0.7152 * f32::from(self.g) + 0.0722 * f32::from(self.b))
            / 255.0
    }

    fn saturation(self) -> f32 {
        let high = self.r.max(self.g).max(self.b);
        let low = self.r.min(self.g).min(self.b);
        if high == 0 {
            0.0
        } else {
            f32::from(high - low) / f32::from(high)
        }
    }

    fn distance(self, other: Self) -> f32 {
        let dr = f32::from(self.r) - f32::from(other.r);
        let dg = f32::from(self.g) - f32::from(other.g);
        let db = f32::from(self.b) - f32::from(other.b);
        (dr.mul_add(dr, dg.mul_add(dg, db * db))).sqrt() / 441.67294
    }

    /// Extreme black contributes no tint in a dark window and near-white
    /// becomes glare in a light one. Pull only those extremes towards a useful
    /// glass tint; ordinary sleeve colours remain untouched.
    fn for_glass(self) -> Self {
        let luma = self.luma();
        if luma < 0.18 {
            self.mix(
                Self {
                    r: 255,
                    g: 255,
                    b: 255,
                },
                ((0.18 - luma) / (1.0 - luma)).min(0.38),
            )
        } else if luma > 0.84 {
            self.mix(Self { r: 0, g: 0, b: 0 }, ((luma - 0.84) / luma).min(0.32))
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlbumPalette {
    pub primary: Rgb,
    pub secondary: Rgb,
}

impl AlbumPalette {
    pub fn interpolate(self, other: Self, t: f32) -> Self {
        Self {
            primary: self.primary.mix(other.primary, t),
            secondary: self.secondary.mix(other.secondary, t),
        }
    }

    fn encode(self) -> String {
        format!(
            "{CACHE_VERSION} {:02x}{:02x}{:02x} {:02x}{:02x}{:02x}\n",
            self.primary.r,
            self.primary.g,
            self.primary.b,
            self.secondary.r,
            self.secondary.g,
            self.secondary.b,
        )
    }

    fn decode(raw: &str) -> Option<Self> {
        let mut fields = raw.split_whitespace();
        if fields.next()? != CACHE_VERSION {
            return None;
        }
        let primary = parse_rgb(fields.next()?)?;
        let secondary = parse_rgb(fields.next()?)?;
        if fields.next().is_some() {
            return None;
        }
        Some(Self { primary, secondary })
    }
}

#[derive(Clone, Copy, Default)]
struct Bucket {
    count: u32,
    r: u64,
    g: u64,
    b: u64,
}

impl Bucket {
    fn add(&mut self, r: u8, g: u8, b: u8) {
        self.count += 1;
        self.r += u64::from(r);
        self.g += u64::from(g);
        self.b += u64::from(b);
    }

    fn color(self) -> Option<Rgb> {
        let count = u64::from(self.count);
        (count > 0).then(|| Rgb {
            r: (self.r / count) as u8,
            g: (self.g / count) as u8,
            b: (self.b / count) as u8,
        })
    }
}

#[derive(Clone, Copy)]
struct Candidate {
    color: Rgb,
    score: f32,
}

/// Pick a vivid representative and a visibly distinct supporting colour.
///
/// Four-bit RGB buckets make the result stable under JPEG noise. Square-root
/// population keeps a huge black border from always beating a smaller, vivid
/// subject, while still respecting colours that genuinely dominate the art.
pub fn extract(
    pixels: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    stride: usize,
) -> Option<AlbumPalette> {
    if width == 0 || height == 0 || channels < 3 || stride < width.checked_mul(channels)? {
        return None;
    }
    let needed = (height - 1)
        .checked_mul(stride)?
        .checked_add(width.checked_mul(channels)?)?;
    if pixels.len() < needed {
        return None;
    }

    let mut buckets = vec![Bucket::default(); BUCKETS];
    for y in 0..height {
        for x in 0..width {
            let at = y * stride + x * channels;
            if channels >= 4 && pixels[at + 3] < 128 {
                continue;
            }
            let [r, g, b] = [pixels[at], pixels[at + 1], pixels[at + 2]];
            let bucket =
                (usize::from(r >> 4) << 8) | (usize::from(g >> 4) << 4) | usize::from(b >> 4);
            buckets[bucket].add(r, g, b);
        }
    }

    let candidates: Vec<Candidate> = buckets
        .into_iter()
        .filter_map(|bucket| {
            let color = bucket.color()?;
            let midtone = 1.0 - ((color.luma() - 0.5).abs() * 2.0).min(1.0);
            let score = (bucket.count as f32).sqrt()
                * (0.35 + 1.65 * color.saturation())
                * (0.72 + 0.28 * midtone);
            Some(Candidate { color, score })
        })
        .collect();

    let primary_raw = candidates
        .iter()
        .copied()
        .max_by(|a, b| a.score.total_cmp(&b.score))?
        .color;
    let primary = primary_raw.for_glass();
    let mut secondary = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.color != primary_raw)
        .max_by(|a, b| {
            let a_score = a.score * (0.30 + 1.70 * a.color.distance(primary_raw));
            let b_score = b.score * (0.30 + 1.70 * b.color.distance(primary_raw));
            a_score.total_cmp(&b_score)
        })
        .map(|candidate| candidate.color.for_glass())
        .unwrap_or(primary);

    if primary.distance(secondary) < 0.16 {
        let contrast = if primary.luma() < 0.52 {
            Rgb {
                r: 255,
                g: 255,
                b: 255,
            }
        } else {
            Rgb { r: 0, g: 0, b: 0 }
        };
        secondary = primary.mix(contrast, 0.30);
    }
    Some(AlbumPalette { primary, secondary })
}

/// Read or derive the palette for a trusted, already-downloaded cover.
/// Call off the GTK thread.
pub fn for_artwork(path: &Path) -> Option<AlbumPalette> {
    let cache = cache_path(path);
    if let Ok(raw) = crate::private_storage::read_to_string(&cache, 64)
        && let Some(palette) = AlbumPalette::decode(&raw)
    {
        return Some(palette);
    }

    let pixbuf = gdk_pixbuf::Pixbuf::from_file_at_scale(path, SAMPLE_PX, SAMPLE_PX, true).ok()?;
    let pixels = pixbuf.read_pixel_bytes();
    let palette = extract(
        pixels.as_ref(),
        pixbuf.width() as usize,
        pixbuf.height() as usize,
        pixbuf.n_channels() as usize,
        pixbuf.rowstride() as usize,
    )?;

    // A cache failure is cosmetic and must not throw away a palette we already
    // derived. The cover directory is private; the temporary file inherits the
    // same 0600 discipline and rename keeps partial data invisible.
    let tmp = cache.with_extension(format!("palette1.tmp{}", std::process::id()));
    if crate::private_storage::write(&tmp, palette.encode())
        .and_then(|()| std::fs::rename(&tmp, &cache))
        .is_err()
    {
        // A rename can lose a race with another request for the same cover.
        // Whichever one won wrote identical deterministic data; only make sure
        // the loser's private temporary name does not linger.
        let _ = std::fs::remove_file(&tmp);
    }
    Some(palette)
}

fn cache_path(path: &Path) -> PathBuf {
    path.with_extension("palette1")
}

fn parse_rgb(raw: &str) -> Option<Rgb> {
    if raw.len() != 6 || !raw.is_ascii() {
        return None;
    }
    Some(Rgb {
        r: u8::from_str_radix(&raw[0..2], 16).ok()?,
        g: u8::from_str_radix(&raw[2..4], 16).ok()?,
        b: u8::from_str_radix(&raw[4..6], 16).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Rgb, b: Rgb) -> bool {
        a.distance(b) < 0.08
    }

    #[test]
    fn a_two_colour_cover_keeps_both_colours() {
        let red = Rgb {
            r: 220,
            g: 25,
            b: 45,
        };
        let blue = Rgb {
            r: 25,
            g: 70,
            b: 220,
        };
        let mut pixels = Vec::new();
        for color in [red; 8].into_iter().chain([blue; 8]) {
            pixels.extend([color.r, color.g, color.b]);
        }
        let palette = extract(&pixels, 4, 4, 3, 12).unwrap();
        let picked = [palette.primary, palette.secondary];
        assert!(picked.iter().any(|color| close(*color, red)), "{picked:?}");
        assert!(picked.iter().any(|color| close(*color, blue)), "{picked:?}");
    }

    #[test]
    fn transparent_pixels_do_not_colour_the_glass() {
        let transparent_red = [255, 0, 0, 0];
        assert!(extract(&transparent_red, 1, 1, 4, 4).is_none());
    }

    #[test]
    fn a_flat_cover_still_gets_two_related_tints() {
        let pixels = [40, 110, 170].repeat(16);
        let palette = extract(&pixels, 4, 4, 3, 12).unwrap();
        assert_ne!(palette.primary, palette.secondary);
        assert!(palette.primary.distance(palette.secondary) >= 0.16);
    }

    #[test]
    fn extreme_colours_are_kept_visible_but_not_glaring() {
        for pixels in [[0, 0, 0], [255, 255, 255]] {
            let palette = extract(&pixels, 1, 1, 3, 3).unwrap();
            assert!((0.17..=0.85).contains(&palette.primary.luma()));
        }
    }

    #[test]
    fn cache_format_is_strict_and_round_trips() {
        let palette = AlbumPalette {
            primary: Rgb { r: 1, g: 2, b: 3 },
            secondary: Rgb { r: 4, g: 5, b: 6 },
        };
        assert_eq!(AlbumPalette::decode(&palette.encode()), Some(palette));
        assert!(AlbumPalette::decode("v2 010203 040506").is_none());
        assert!(AlbumPalette::decode("v1 010203 ../../etc/passwd").is_none());
        assert!(AlbumPalette::decode("v1 010203 040506 extra").is_none());
    }

    #[test]
    fn interpolation_is_pinned_at_both_ends() {
        let a = AlbumPalette {
            primary: Rgb {
                r: 10,
                g: 20,
                b: 30,
            },
            secondary: Rgb {
                r: 40,
                g: 50,
                b: 60,
            },
        };
        let b = AlbumPalette {
            primary: Rgb {
                r: 210,
                g: 220,
                b: 230,
            },
            secondary: Rgb {
                r: 140,
                g: 150,
                b: 160,
            },
        };
        assert_eq!(a.interpolate(b, 0.0), a);
        assert_eq!(a.interpolate(b, 1.0), b);
        assert_eq!(a.interpolate(b, -1.0), a);
        assert_eq!(a.interpolate(b, 2.0), b);
    }
}
