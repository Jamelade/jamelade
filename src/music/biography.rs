// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Credential-free fallback for biographies Apple publishes on artist pages.
//!
//! MusicKit's catalog response can omit editorial notes even when the public
//! Apple Music page has them. This client therefore sends only the numeric
//! artist id to a fixed Apple origin, carries no browser cookies or tokens,
//! validates Apple's one canonical redirect, and parses a bounded JSON block.

use anyhow::{Context, Result};
use reqwest::{StatusCode, header, redirect};
use serde_json::Value;

use super::types::editorial_plain_text;

const PAGE_BYTES_MAX: usize = 2 * 1024 * 1024;
const PAGE_URL_BYTES_MAX: usize = 2_048;
const JSON_NODES_MAX: usize = 50_000;
const SERIALIZED_DATA_ID: &str = "serialized-server-data";

/// Fetch the English biography Apple exposes on its public US artist page.
///
/// This is deliberately independent of the authenticated browser broker. The
/// request has no cookie jar, bearer token, account id, or listening metadata.
pub(crate) async fn fetch_english(catalog_id: &str) -> Result<Option<String>> {
    if !valid_catalog_id(catalog_id) {
        anyhow::bail!("invalid Apple Music artist id");
    }

    let redirect_id = catalog_id.to_owned();
    let client = reqwest::Client::builder()
        .redirect(redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > 1 || !trusted_artist_redirect(attempt.url(), &redirect_id)
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .timeout(std::time::Duration::from_secs(12))
        .user_agent(concat!("Jamelade/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("building Apple biography client")?;
    let url = format!("https://music.apple.com/us/artist/{catalog_id}");
    let mut response = client
        .get(url)
        .header(header::ACCEPT, "text/html")
        .send()
        .await
        // A reqwest error can retain its URL. Keep even a public artist id out
        // of diagnostics so this path follows the app's metadata-log policy.
        .map_err(|_| anyhow::anyhow!("Apple artist page could not be reached"))?;

    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        anyhow::bail!("Apple artist page returned an error");
    }
    if response
        .content_length()
        .is_some_and(|length| length > PAGE_BYTES_MAX as u64)
    {
        anyhow::bail!("Apple artist page was too large");
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type
        .split(';')
        .next()
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("text/html"))
    {
        anyhow::bail!("Apple artist page had an unexpected content type");
    }

    let mut body = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(PAGE_BYTES_MAX as u64) as usize,
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("Apple artist page was interrupted"))?
    {
        let Some(next) = body.len().checked_add(chunk.len()) else {
            anyhow::bail!("Apple artist page was too large");
        };
        if next > PAGE_BYTES_MAX {
            anyhow::bail!("Apple artist page was too large");
        }
        body.extend_from_slice(&chunk);
    }
    let html = std::str::from_utf8(&body).context("decoding Apple artist page")?;
    Ok(biography_from_page(html, catalog_id))
}

fn valid_catalog_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn trusted_artist_redirect(url: &reqwest::Url, catalog_id: &str) -> bool {
    if url.as_str().len() > PAGE_URL_BYTES_MAX
        || url.scheme() != "https"
        || url.host_str() != Some("music.apple.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let Some(parts) = url.path_segments() else {
        return false;
    };
    let parts: Vec<_> = parts.collect();
    parts.len() == 4
        && parts[0] == "us"
        && parts[1] == "artist"
        && !parts[2].is_empty()
        && parts[2].len() <= 256
        && parts[3] == catalog_id
}

fn biography_from_page(html: &str, catalog_id: &str) -> Option<String> {
    let raw = script_body_by_id(html, SERIALIZED_DATA_ID)?;
    let document: Value = serde_json::from_str(raw).ok()?;
    let target = format!("artist-bio - {catalog_id}");
    let mut stack = vec![&document];
    let mut visited = 0usize;

    while let Some(value) = stack.pop() {
        visited = visited.saturating_add(1);
        if visited > JSON_NODES_MAX {
            return None;
        }
        match value {
            Value::Object(object) => {
                if object.get("id").and_then(Value::as_str) == Some(target.as_str())
                    && let Some(raw) = object
                        .get("modalPresentationDescriptor")
                        .and_then(Value::as_object)
                        .and_then(|modal| modal.get("paragraphText"))
                        .and_then(Value::as_str)
                {
                    let biography = editorial_plain_text(raw);
                    if !biography.is_empty() {
                        return Some(biography);
                    }
                }
                stack.extend(object.values());
            }
            Value::Array(array) => stack.extend(array),
            _ => {}
        }
    }
    None
}

fn script_body_by_id<'a>(html: &'a str, wanted_id: &str) -> Option<&'a str> {
    let mut offset = 0usize;
    while let Some(relative_start) = html.get(offset..)?.find("<script") {
        let start = offset + relative_start;
        let open_end = start + html.get(start..)?.find('>')?;
        let opening = html.get(start..=open_end)?;
        let body_start = open_end + 1;
        let close = body_start + html.get(body_start..)?.find("</script>")?;
        if html_attribute(opening, "id") == Some(wanted_id) {
            return html.get(body_start..close);
        }
        offset = close + "</script>".len();
    }
    None
}

fn html_attribute<'a>(tag: &'a str, wanted: &str) -> Option<&'a str> {
    let bytes = tag.as_bytes();
    let mut index = "<script".len();
    while index < bytes.len() {
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b'/') {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] == b'>' {
            break;
        }
        let name_start = index;
        while index < bytes.len()
            && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'-' | b'_' | b':'))
        {
            index += 1;
        }
        if name_start == index {
            index += 1;
            continue;
        }
        let name = tag.get(name_start..index)?;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let quote = matches!(bytes[index], b'\'' | b'"').then_some(bytes[index]);
        if quote.is_some() {
            index += 1;
        }
        let value_start = index;
        while index < bytes.len()
            && match quote {
                Some(quote) => bytes[index] != quote,
                None => !bytes[index].is_ascii_whitespace() && bytes[index] != b'>',
            }
        {
            index += 1;
        }
        let value = tag.get(value_start..index)?;
        if quote.is_some() && index < bytes.len() {
            index += 1;
        }
        if name.eq_ignore_ascii_case(wanted) {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_expected_apple_canonical_redirect() {
        let good = reqwest::Url::parse("https://music.apple.com/us/artist/beach-bunny/1147783278")
            .unwrap();
        let wrong_id =
            reqwest::Url::parse("https://music.apple.com/us/artist/beach-bunny/9").unwrap();
        let wrong_host =
            reqwest::Url::parse("https://music.apple.com.example/us/artist/beach-bunny/1147783278")
                .unwrap();

        assert!(trusted_artist_redirect(&good, "1147783278"));
        assert!(!trusted_artist_redirect(&wrong_id, "1147783278"));
        assert!(!trusted_artist_redirect(&wrong_host, "1147783278"));
    }

    #[test]
    fn rejects_non_numeric_catalog_ids() {
        assert!(valid_catalog_id("1147783278"));
        assert!(!valid_catalog_id(""));
        assert!(!valid_catalog_id("r.library-id"));
        assert!(!valid_catalog_id("1/../../elsewhere"));
    }

    #[test]
    fn extracts_only_the_requested_explicit_artist_biography() {
        let html = r#"<html><head>
            <script id=schema:music-group type="application/ld+json">
              {"description":"Listen to music by Beach Bunny on Apple Music."}
            </script>
            <script type="application/json" id="serialized-server-data">
              {"data":{"items":[
                {"id":"artist-bio - 9","modalPresentationDescriptor":{"paragraphText":"Wrong artist."}},
                {"id":"artist-bio - 1147783278","modalPresentationDescriptor":{"paragraphText":"A <i>real</i> biography &amp; nothing else."}}
              ]}}
            </script></head></html>"#;

        assert_eq!(
            biography_from_page(html, "1147783278").as_deref(),
            Some("A real biography & nothing else.")
        );
        assert_eq!(biography_from_page(html, "8"), None);
    }

    #[test]
    fn missing_artist_bio_does_not_turn_page_metadata_into_a_biography() {
        let html = r#"<script id='serialized-server-data' type='application/json'>
            {"data":{"description":"Listen to music by Finishing Move Inc. on Apple Music."}}
            </script>"#;
        assert_eq!(biography_from_page(html, "1152686264"), None);
    }
}
