// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Explicitly opt-in ListenBrainz scrobbling.
//!
//! The token is encrypted with the per-application key supplied by the Secret
//! portal before it is written to Jamelade's private configuration directory.
//! A submission contains only the visible title, artist, album, duration and
//! start time: Apple identifiers, credentials, artwork and lyrics stay out.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ring::{aead, digest, rand};
use serde_json::json;
use zeroize::{Zeroize, Zeroizing};

use crate::player::protocol::Item;

const ENDPOINT: &str = "https://api.listenbrainz.org/1/submit-listens";
const MAGIC: &[u8; 4] = b"JLB1";
const NONCE_LEN: usize = 12;
const MAX_FILE_BYTES: usize = 512;
const MAX_TOKEN_BYTES: usize = 128;
const MIN_LISTEN_MS: u64 = 30_000;
const MAX_THRESHOLD_MS: u64 = 240_000;
const AAD: &[u8] = b"io.github.Jamelade.Jamelade/listenbrainz/v1";

/// A token whose debug representation and destructor never expose its bytes.
#[derive(Clone)]
pub struct Token(Zeroizing<String>);

impl Token {
    pub fn parse(value: impl Into<String>) -> Result<Self, &'static str> {
        let mut value = value.into();
        let mut trimmed = value.trim().to_owned();
        value.zeroize();
        if !(16..=MAX_TOKEN_BYTES).contains(&trimmed.len())
            || !trimmed
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            trimmed.zeroize();
            return Err("That does not look like a ListenBrainz token");
        }
        Ok(Self(Zeroizing::new(trimmed)))
    }

    fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token([redacted])")
    }
}

#[derive(Clone)]
pub struct Submission {
    key: String,
    token: Token,
    body: serde_json::Value,
}

impl Submission {
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl std::fmt::Debug for Submission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Submission")
            .field("key", &"[listening item]")
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
pub struct Scrobbler {
    token: Option<Token>,
    submitted_for: Option<String>,
    pending_for: Option<String>,
}

impl Scrobbler {
    pub fn set_token(&mut self, token: Token) {
        self.token = Some(token);
        self.submitted_for = None;
        self.pending_for = None;
    }

    pub fn disable(&mut self) {
        self.token = None;
        self.submitted_for = None;
        self.pending_for = None;
    }

    pub fn reset_track(&mut self) {
        self.submitted_for = None;
        self.pending_for = None;
    }

    /// Build at most one submission for a track after half its duration or
    /// four minutes, whichever comes first. Very short clips are ignored.
    pub fn prepare(&mut self, item: &Item, position_ms: u64) -> Option<Submission> {
        let token = self.token.clone()?;
        let duration_ms = item.duration_ms;
        if duration_ms < MIN_LISTEN_MS
            || item.title.trim().is_empty()
            || item.artist.trim().is_empty()
            || position_ms < (duration_ms / 2).min(MAX_THRESHOLD_MS)
        {
            return None;
        }
        let key = item
            .catalog_id
            .as_ref()
            .or(item.id.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("{}\u{1f}{}\u{1f}{}", item.artist, item.album, item.title));
        if self.submitted_for.as_deref() == Some(&key) || self.pending_for.as_deref() == Some(&key)
        {
            return None;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let started_at = now.saturating_sub(position_ms / 1_000);
        let body = json!({
            "listen_type": "single",
            "payload": [{
                "listened_at": started_at,
                "track_metadata": {
                    "artist_name": bounded_text(&item.artist),
                    "track_name": bounded_text(&item.title),
                    "release_name": bounded_text(&item.album),
                    "additional_info": {
                        "duration_ms": duration_ms,
                        "media_player": "Jamelade",
                        "submission_client": "Jamelade",
                        "submission_client_version": env!("CARGO_PKG_VERSION")
                    }
                }
            }]
        });
        self.pending_for = Some(key.clone());
        Some(Submission { key, token, body })
    }

    pub fn finish(&mut self, key: &str, succeeded: bool) {
        if self.pending_for.as_deref() == Some(key) {
            self.pending_for = None;
            // One attempt per track. A network outage must not turn the 250ms
            // UI clock into a retry loop; the next track gets a fresh attempt.
            let _ = succeeded;
            self.submitted_for = Some(key.to_owned());
        }
    }
}

fn bounded_text(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(512)
        .collect()
}

fn token_path() -> PathBuf {
    relm4::gtk::glib::user_config_dir()
        .join("jamelade")
        .join("listenbrainz-token.bin")
}

fn cipher(master: &[u8]) -> Result<aead::LessSafeKey, String> {
    if master.len() < 16 {
        return Err("the desktop secret was unavailable".into());
    }
    let mut material = Vec::with_capacity(AAD.len() + master.len());
    material.extend_from_slice(AAD);
    material.extend_from_slice(master);
    let hash = digest::digest(&digest::SHA256, &material);
    material.zeroize();
    let key = aead::UnboundKey::new(&aead::CHACHA20_POLY1305, hash.as_ref())
        .map_err(|_| "could not create an encryption key".to_owned())?;
    Ok(aead::LessSafeKey::new(key))
}

fn encrypt(token: &Token, master: &[u8]) -> Result<Vec<u8>, String> {
    let key = cipher(master)?;
    let rng = rand::SystemRandom::new();
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::SecureRandom::fill(&rng, &mut nonce_bytes)
        .map_err(|_| "could not create a secure nonce".to_owned())?;
    let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);
    let mut sealed = token.expose().as_bytes().to_vec();
    key.seal_in_place_append_tag(nonce, aead::Aad::from(AAD), &mut sealed)
        .map_err(|_| "could not protect the token".to_owned())?;
    let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + sealed.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&sealed);
    sealed.zeroize();
    Ok(out)
}

fn decrypt(stored: Vec<u8>, master: &[u8]) -> Result<Token, String> {
    // Keep the whole buffer in a wiping owner so every early error path is
    // covered, including authentication and UTF-8 failures.
    let mut stored = Zeroizing::new(stored);
    if stored.len() < MAGIC.len() + NONCE_LEN + aead::CHACHA20_POLY1305.tag_len()
        || stored.len() > MAX_FILE_BYTES
        || stored.get(..MAGIC.len()) != Some(MAGIC)
    {
        return Err("the stored ListenBrainz token is invalid".into());
    }
    let mut nonce_bytes = [0u8; NONCE_LEN];
    nonce_bytes.copy_from_slice(&stored[MAGIC.len()..MAGIC.len() + NONCE_LEN]);
    let start = MAGIC.len() + NONCE_LEN;
    let key = cipher(master)?;
    let plaintext = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce_bytes),
            aead::Aad::from(AAD),
            &mut stored[start..],
        )
        .map_err(|_| "the stored ListenBrainz token could not be unlocked".to_owned())?;
    let token_text = std::str::from_utf8(plaintext)
        .map_err(|_| "the stored ListenBrainz token is invalid".to_owned())?;
    Token::parse(token_text.to_owned()).map_err(str::to_owned)
}

async fn portal_secret() -> Result<Zeroizing<Vec<u8>>, String> {
    ashpd::desktop::secret::retrieve()
        .await
        .map(Zeroizing::new)
        .map_err(|_| "the desktop keyring portal is unavailable".to_owned())
}

pub async fn load_token() -> Result<Option<Token>, String> {
    let stored = match crate::private_storage::read_bytes(&token_path(), MAX_FILE_BYTES) {
        Ok(stored) => stored,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("the stored ListenBrainz token could not be read".into()),
    };
    let master = portal_secret().await?;
    decrypt(stored, &master).map(Some)
}

pub async fn store_token(token: &Token) -> Result<(), String> {
    let master = portal_secret().await?;
    let mut sealed = encrypt(token, &master)?;
    let result = crate::private_storage::write(&token_path(), &sealed)
        .map_err(|_| "the ListenBrainz token could not be saved".to_owned());
    sealed.zeroize();
    result
}

pub fn remove_token() -> Result<(), String> {
    match std::fs::remove_file(token_path()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("the stored ListenBrainz token could not be removed".into()),
    }
}

pub async fn submit(submission: Submission) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(8))
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(concat!("Jamelade/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "could not prepare ListenBrainz".to_owned())?;
    let response = client
        .post(ENDPOINT)
        .bearer_auth(submission.token.expose())
        .json(&submission.body)
        .send()
        .await
        .map_err(|_| "ListenBrainz could not be reached".to_owned())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err("ListenBrainz rejected the scrobble".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(duration_ms: u64) -> Item {
        Item {
            id: Some("synthetic-track".into()),
            catalog_id: Some("123456".into()),
            title: "Synthetic Song".into(),
            artist: "Example Artist".into(),
            album: "Example Album".into(),
            duration_ms,
            ..Item::default()
        }
    }

    #[test]
    fn token_encryption_round_trips_and_rejects_wrong_key() {
        let token = Token::parse("01234567-89ab-cdef-0123-456789abcdef").unwrap();
        let sealed = encrypt(&token, b"0123456789abcdef0123456789abcdef").unwrap();
        assert!(!sealed.windows(8).any(|part| part == b"01234567"));
        assert_eq!(
            decrypt(sealed.clone(), b"0123456789abcdef0123456789abcdef")
                .unwrap()
                .expose(),
            token.expose()
        );
        assert!(decrypt(sealed, b"fedcba9876543210fedcba9876543210").is_err());
    }

    #[test]
    fn a_track_scrobbles_once_after_its_threshold() {
        let mut scrobbler = Scrobbler::default();
        scrobbler.set_token(Token::parse("01234567-89ab-cdef-0123-456789abcdef").unwrap());
        assert!(scrobbler.prepare(&item(180_000), 89_999).is_none());
        let submission = scrobbler.prepare(&item(180_000), 90_000).unwrap();
        assert!(scrobbler.prepare(&item(180_000), 120_000).is_none());
        scrobbler.finish(submission.key(), true);
        assert!(scrobbler.prepare(&item(180_000), 170_000).is_none());
    }

    #[test]
    fn very_short_clips_do_not_scrobble() {
        let mut scrobbler = Scrobbler::default();
        scrobbler.set_token(Token::parse("01234567-89ab-cdef-0123-456789abcdef").unwrap());
        assert!(scrobbler.prepare(&item(29_999), 29_999).is_none());
    }
}
