// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! Lyrics from Apple Music and separately consented third-party providers.
//!
//! Apple is asked first through the already-authenticated Apple client: that
//! discloses no playback fact to a company that did not already receive it.
//! Every third-party request remains on the deliberately separate client below:
//! it carries no Apple headers or tokens, follows no redirects, and is called
//! only after that provider is enabled. Sources are tried one at a time and the
//! first useful answer wins, avoiding needless disclosure to every enabled
//! service.

use anyhow::{Context, Result};
use reqwest::{Client, StatusCode, header};
use serde::Deserialize;

mod apple_ttml;

pub(crate) use apple_ttml::parse as lyrics_from_apple_ttml;
#[cfg(test)]
use apple_ttml::parse_ttml_timestamp;

const LRCLIB_ENDPOINT: &str = "https://lrclib.net/api/get";
const LRCLIB_SEARCH_ENDPOINT: &str = "https://lrclib.net/api/search";
const LYRICS_OVH_ENDPOINT: &str = "https://api.lyrics.ovh/v1";
const BODY_MAX: usize = 256 * 1024;
const LINE_MAX: usize = 600;
const LINES_MAX: usize = 2_000;
const TRACK_FIELD_MAX: usize = 1_000;
const MAX_TIMESTAMP_MS: u64 = 24 * 60 * 60 * 1_000;
const SEARCH_DURATION_TOLERANCE_SECS: f64 = 3.0;

/// What a lyrics provider may need to match one recording. It intentionally
/// contains no Apple id, account identifier, cookie, or token.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Query {
    /// A numeric Apple catalog id, when MusicKit supplied one. It is part of
    /// the memory-cache identity and is the only value Apple's lyrics endpoint
    /// needs; library ids and malformed renderer input are dropped.
    pub catalog_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
}

impl Query {
    pub fn new(
        catalog_id: Option<&str>,
        title: &str,
        artist: &str,
        album: &str,
        duration_ms: u64,
    ) -> Option<Self> {
        let title = bounded(title);
        let artist = bounded(artist);
        if title.is_empty() || artist.is_empty() {
            return None;
        }
        Some(Self {
            catalog_id: catalog_id.and_then(valid_catalog_id).map(str::to_owned),
            title,
            artist,
            album: bounded(album),
            duration_ms,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// `None` for plain, unsynchronised lyrics.
    pub at_ms: Option<u64>,
    pub text: String,
}

/// The source that supplied a result. Kept with the in-memory cache entry so
/// the UI can always attribute lyrics without another network request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    AppleMusic,
    Lrclib,
    LyricsOvh,
}

impl Provider {
    pub const fn label(self) -> &'static str {
        match self {
            Self::AppleMusic => "Apple Music",
            Self::Lrclib => "LRCLIB",
            Self::LyricsOvh => "Lyrics.ovh",
        }
    }
}

/// One snapshot of the user's consent, captured before an asynchronous fetch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Providers {
    pub lrclib: bool,
    pub lyrics_ovh: bool,
}

impl Providers {
    pub const fn any(self) -> bool {
        self.lrclib || self.lyrics_ovh
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lyrics {
    pub lines: Vec<Line>,
    pub synced: bool,
    pub instrumental: bool,
    pub source: Option<Provider>,
    /// First-party alternatives embedded in Apple's bounded response. These
    /// are never synthesized or fetched from another provider.
    pub variants: Vec<LyricVariant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricVariantKind {
    Translation,
    Romanization,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricVariant {
    pub kind: LyricVariantKind,
    pub label: String,
    pub lines: Vec<Line>,
    pub synced: bool,
}

impl Lyrics {
    /// Index zero is always the original. Alternatives follow in the order
    /// Apple supplied them and inherit only the source attribution.
    pub fn selected(&self, index: usize) -> Self {
        let Some(variant) = index.checked_sub(1).and_then(|at| self.variants.get(at)) else {
            let mut original = self.clone();
            original.variants.clear();
            return original;
        };
        Self {
            lines: variant.lines.clone(),
            synced: variant.synced,
            instrumental: false,
            source: self.source,
            variants: Vec::new(),
        }
    }

    pub fn variant_labels(&self) -> Vec<String> {
        let mut labels = Vec::with_capacity(self.variants.len() + 1);
        labels.push(crate::i18n::tr("Original").to_owned());
        labels.extend(self.variants.iter().map(|variant| {
            let english = match variant.kind {
                LyricVariantKind::Translation => "Translation",
                LyricVariantKind::Romanization => "Romanized",
            };
            if let Some(detail) = variant.label.strip_prefix(english) {
                format!("{}{}", crate::i18n::tr(english), detail)
            } else {
                variant.label.clone()
            }
        }));
        labels
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireLyrics {
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    artist_name: String,
    #[serde(default)]
    album_name: String,
    #[serde(default)]
    duration: f64,
    #[serde(default)]
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LyricsOvhWire {
    #[serde(default)]
    lyrics: String,
}

/// Ask available providers for one track, in privacy-preserving priority order.
///
/// Apple Music goes first and reuses the existing authenticated client. LRCLIB
/// and Lyrics.ovh follow only when separately enabled and Apple had no usable
/// answer. A successful empty response beats a transport error from another
/// provider, so "not found" does not become an alarming failure page.
pub async fn fetch(
    query: &Query,
    providers: Providers,
    apple: Option<crate::music::client::Client>,
) -> Result<Lyrics> {
    if apple.is_none() && !providers.any() {
        anyhow::bail!("no lyrics provider is enabled");
    }

    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(12))
        .user_agent(concat!("Jamelade/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("the static rustls lyrics client configuration is valid");

    let mut had_response = false;
    let mut last_error = None;

    if let (Some(client), Some(catalog_id)) = (apple.as_ref(), query.catalog_id.as_deref()) {
        match client.lyrics(catalog_id).await {
            Ok(Some(lyrics)) => {
                had_response = true;
                if lyrics.instrumental || !lyrics.lines.is_empty() {
                    return Ok(lyrics);
                }
            }
            Ok(None) => had_response = true,
            Err(err) => last_error = Some(err),
        }
    } else if apple.is_some() {
        // Library uploads and delisted songs have no catalog id. Apple is still
        // an available source; "no match" is more honest than "no provider".
        had_response = true;
    }

    if providers.lrclib {
        match fetch_lrclib(&client, query).await {
            Ok(lyrics) => {
                had_response = true;
                if lyrics.instrumental || !lyrics.lines.is_empty() {
                    return Ok(lyrics);
                }
            }
            Err(err) => last_error = Some(err),
        }
    }

    if providers.lyrics_ovh {
        match fetch_lyrics_ovh(&client, query).await {
            Ok(lyrics) => {
                had_response = true;
                if !lyrics.lines.is_empty() {
                    return Ok(lyrics);
                }
            }
            Err(err) => last_error = Some(err),
        }
    }

    if had_response {
        Ok(Lyrics::default())
    } else {
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("lyrics providers could not be reached")))
    }
}

/// Ask LRCLIB for one track.
///
/// The response is streamed into a capped buffer rather than handed to
/// `Response::json`, which would accept an arbitrarily large body from a
/// third-party service. A 404 is the ordinary "nothing found" answer.
async fn fetch_lrclib(client: &Client, query: &Query) -> Result<Lyrics> {
    let mut url = format!(
        "{LRCLIB_ENDPOINT}?track_name={}&artist_name={}&album_name={}",
        urlencode(&query.title),
        urlencode(&query.artist),
        urlencode(&query.album),
    );
    let duration_secs = query.duration_ms.saturating_add(500) / 1_000;
    if (1..=3_600).contains(&duration_secs) {
        url.push_str(&format!("&duration={duration_secs}"));
    }

    let exact =
        get_json::<WireLyrics>(client, url, Provider::Lrclib, "decoding LRCLIB response").await?;
    if let Some(wire) = exact.as_ref() {
        let lyrics = lyrics_from_wire(wire);
        if lyrics.instrumental || lyrics.synced {
            return Ok(lyrics);
        }
    }

    // `/api/get` deliberately returns one exact record. Some releases have a
    // plain record there while a single, deluxe edition or re-upload of the
    // same recording has timestamps. Search at most once, then accept a
    // candidate only when title and artist normalize exactly and its duration
    // is within three seconds. A wrong synchronized lyric is worse than plain.
    match search_synced(client, query).await {
        Ok(Some(wire)) => return Ok(lyrics_from_wire(&wire)),
        Ok(None) => {}
        Err(err) if exact.is_none() => return Err(err),
        // The exact plain result remains useful when the optional fallback is
        // unavailable. Do not turn working lyrics into an error.
        Err(_) => {}
    }

    Ok(exact.as_ref().map(lyrics_from_wire).unwrap_or_default())
}

/// Ask Lyrics.ovh for a plain lyric. Its endpoint needs only artist and title;
/// album and duration are deliberately not appended to the path.
async fn fetch_lyrics_ovh(client: &Client, query: &Query) -> Result<Lyrics> {
    let url = format!(
        "{LYRICS_OVH_ENDPOINT}/{}/{}",
        urlencode(&query.artist),
        urlencode(&query.title),
    );
    let wire = get_json::<LyricsOvhWire>(
        client,
        url,
        Provider::LyricsOvh,
        "decoding Lyrics.ovh response",
    )
    .await?;
    Ok(lyrics_from_ovh_wire(wire.as_ref()))
}

async fn search_synced(client: &Client, query: &Query) -> Result<Option<WireLyrics>> {
    let url = format!(
        "{LRCLIB_SEARCH_ENDPOINT}?track_name={}&artist_name={}",
        urlencode(&query.title),
        urlencode(&query.artist),
    );
    let candidates =
        get_json::<Vec<WireLyrics>>(client, url, Provider::Lrclib, "decoding LRCLIB search")
            .await?
            .unwrap_or_default();
    Ok(choose_synced(query, candidates))
}

fn choose_synced(query: &Query, candidates: Vec<WireLyrics>) -> Option<WireLyrics> {
    let title = normalized(&query.title);
    let artist = normalized(&query.artist);
    let album = normalized(&query.album);
    let duration = query.duration_ms as f64 / 1_000.0;
    if title.is_empty() || artist.is_empty() || duration <= 0.0 {
        return None;
    }

    candidates
        .into_iter()
        .filter(|candidate| {
            !candidate.instrumental
                && normalized(&candidate.track_name) == title
                && normalized(&candidate.artist_name) == artist
                && candidate.duration.is_finite()
                && (candidate.duration - duration).abs() <= SEARCH_DURATION_TOLERANCE_SECS
                && candidate
                    .synced_lyrics
                    .as_deref()
                    .is_some_and(|raw| !parse_lrc(raw).is_empty())
        })
        .min_by(|left, right| {
            // Prefer the same album, then the closest duration. Stable input
            // order breaks the vanishingly unlikely exact tie.
            let left_album = normalized(&left.album_name) == album;
            let right_album = normalized(&right.album_name) == album;
            right_album.cmp(&left_album).then_with(|| {
                (left.duration - duration)
                    .abs()
                    .total_cmp(&(right.duration - duration).abs())
            })
        })
}

async fn get_json<T>(
    client: &Client,
    url: String,
    provider: Provider,
    decode_context: &'static str,
) -> Result<Option<T>>
where
    T: serde::de::DeserializeOwned,
{
    // `reqwest::Error` may include the complete request URL, which here holds
    // listening metadata in its query. Deliberately discard that source before
    // anything reaches the UI or logs.
    let service = provider.label();
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("could not reach {service}"))?;

    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        anyhow::bail!("{service} returned {}", response.status());
    }
    if response
        .content_length()
        .is_some_and(|len| len > BODY_MAX as u64)
    {
        anyhow::bail!("{service} response was too large");
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type
        .split(';')
        .next()
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
    {
        anyhow::bail!("{service} returned an unexpected content type");
    }

    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("{service} response was interrupted"))?
    {
        if body.len().saturating_add(chunk.len()) > BODY_MAX {
            anyhow::bail!("{service} response was too large");
        }
        body.extend_from_slice(&chunk);
    }
    let parsed = serde_json::from_slice(&body).context(decode_context)?;
    Ok(Some(parsed))
}

fn lyrics_from_wire(wire: &WireLyrics) -> Lyrics {
    if wire.instrumental {
        return Lyrics {
            instrumental: true,
            source: Some(Provider::Lrclib),
            ..Lyrics::default()
        };
    }

    if let Some(synced) = wire.synced_lyrics.as_deref() {
        let lines = parse_lrc(synced);
        if !lines.is_empty() {
            return Lyrics {
                lines,
                synced: true,
                instrumental: false,
                source: Some(Provider::Lrclib),
                ..Lyrics::default()
            };
        }
    }

    Lyrics {
        lines: wire
            .plain_lyrics
            .as_deref()
            .map(parse_plain)
            .unwrap_or_default(),
        synced: false,
        instrumental: false,
        source: Some(Provider::Lrclib),
        ..Lyrics::default()
    }
}

fn lyrics_from_ovh_wire(wire: Option<&LyricsOvhWire>) -> Lyrics {
    Lyrics {
        lines: wire
            .map(|wire| parse_plain(&wire.lyrics))
            .unwrap_or_default(),
        synced: false,
        instrumental: false,
        source: Some(Provider::LyricsOvh),
        ..Lyrics::default()
    }
}

fn bounded(value: &str) -> String {
    value.trim().chars().take(TRACK_FIELD_MAX).collect()
}

fn valid_catalog_id(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some(value)
}

fn clean_line(value: &str) -> String {
    value.trim().chars().take(LINE_MAX).collect()
}

fn normalized(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|ch| ch.is_alphanumeric())
        .collect()
}

fn parse_plain(raw: &str) -> Vec<Line> {
    raw.lines()
        .map(clean_line)
        .filter(|line| !line.is_empty())
        .take(LINES_MAX)
        .map(|text| Line { at_ms: None, text })
        .collect()
}

/// Parse the common enhanced-LRC shape, including more than one timestamp on
/// a line. Metadata tags (`[ar:…]`, `[offset:…]`) are ignored; malformed and
/// absurd timestamps are dropped rather than guessed.
fn parse_lrc(raw: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for raw_line in raw.lines() {
        let mut rest = raw_line.trim();
        let mut stamps = Vec::new();
        while let Some(after_open) = rest.strip_prefix('[') {
            let Some((tag, after)) = after_open.split_once(']') else {
                break;
            };
            let Some(at_ms) = parse_timestamp(tag) else {
                // A metadata tag at the front means this is not a lyric line.
                stamps.clear();
                break;
            };
            stamps.push(at_ms);
            rest = after;
        }
        if stamps.is_empty() {
            continue;
        }
        let text = clean_line(rest);
        if text.is_empty() {
            continue;
        }
        for at_ms in stamps {
            lines.push(Line {
                at_ms: Some(at_ms),
                text: text.clone(),
            });
            if lines.len() >= LINES_MAX {
                break;
            }
        }
        if lines.len() >= LINES_MAX {
            break;
        }
    }
    lines.sort_by_key(|line| line.at_ms);
    lines
}

fn parse_timestamp(tag: &str) -> Option<u64> {
    let (minutes, seconds) = tag.split_once(':')?;
    let minutes: u64 = minutes.parse().ok()?;
    let seconds: f64 = seconds.parse().ok()?;
    if !seconds.is_finite() || !(0.0..60.0).contains(&seconds) {
        return None;
    }
    let millis = minutes
        .checked_mul(60_000)?
        .checked_add((seconds * 1_000.0).round() as u64)?;
    (millis <= MAX_TIMESTAMP_MS).then_some(millis)
}

/// Percent-encode one query value according to RFC 3986. Keeping this tiny and
/// local avoids a URL dependency for four values and, crucially, never builds
/// a path from remote track metadata.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests;
