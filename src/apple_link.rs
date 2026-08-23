// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Public Apple Music links, kept separate from authenticated API traffic.
//!
//! Apple normally supplies canonical `attributes.url` values. Those are
//! accepted only when they are HTTPS links to the exact Apple Music host.
//! MusicKit queue items do not carry that attribute, so the currently playing
//! catalog song uses Apple's supported `song/-/{id}` redirect form instead.

const MAX_LINK_BYTES: usize = 2_048;
const MAX_CATALOG_ID_BYTES: usize = 32;

/// Keep a canonical Apple URL only when it cannot point somewhere else.
pub fn canonical(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.len() > MAX_LINK_BYTES || raw.chars().any(char::is_control) {
        return None;
    }
    let parsed = reqwest::Url::parse(raw).ok()?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("music.apple.com")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
    {
        return None;
    }
    Some(parsed.into())
}

/// A public link for a catalog song currently reported by MusicKit.
pub fn song(storefront: &str, catalog_id: &str) -> Option<String> {
    let storefront = storefront.trim().to_ascii_lowercase();
    if storefront.len() != 2 || !storefront.bytes().all(|byte| byte.is_ascii_lowercase()) {
        return None;
    }
    if catalog_id.is_empty()
        || catalog_id.len() > MAX_CATALOG_ID_BYTES
        || !catalog_id.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    canonical(&format!(
        "https://music.apple.com/{storefront}/song/-/{catalog_id}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_exact_apple_music_https_links() {
        let expected = "https://music.apple.com/de/album/example/123?i=456";
        assert_eq!(canonical(expected).as_deref(), Some(expected));
        assert!(canonical("http://music.apple.com/de/song/-/456").is_none());
        assert!(canonical("https://music.apple.com.evil.test/de/song/-/456").is_none());
        assert!(canonical("https://user@music.apple.com/de/song/-/456").is_none());
        assert!(canonical("https://music.apple.com:444/de/song/-/456").is_none());
    }

    #[test]
    fn queue_song_links_are_bounded_and_unambiguous() {
        assert_eq!(
            song("DE", "1049009209").as_deref(),
            Some("https://music.apple.com/de/song/-/1049009209")
        );
        assert!(song("../de", "1049009209").is_none());
        assert!(song("de", "i.private").is_none());
        assert!(song("de", "1?x=2").is_none());
    }
}
