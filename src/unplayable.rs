// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! Catalog ids MusicKit has refused, remembered between runs.
//!
//! `setQueue` is all-or-nothing, so a handful of delisted tracks in a library
//! reject the whole queue. We recover by reading the ids out of the error and
//! retrying — but that costs a failed round trip on the *first* play of every
//! session, which is a visible delay before the music starts.
//!
//! The ids do not change from one run to the next, so there is no reason to
//! rediscover them each time. This is a cache, not state: losing it costs one
//! slow play, so it is written best-effort and read tolerantly.

use std::collections::HashSet;
use std::path::PathBuf;

const CACHE_MAX_BYTES: usize = 2 * 1024 * 1024;
const CACHE_MAX_IDS: usize = 50_000;
const CACHE_MAX_ID_BYTES: usize = 512;

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= CACHE_MAX_ID_BYTES && id.bytes().all(|byte| byte.is_ascii_digit())
}

fn cache_file() -> Option<PathBuf> {
    crate::components::artwork::cache_dir()?
        .parent()
        .map(|dir| dir.join("unplayable.json"))
}

/// Read the remembered ids. Any problem yields an empty set — a cache that
/// cannot be read is not an error, it just means the first play is slow again.
pub fn load() -> HashSet<String> {
    let Some(path) = cache_file() else {
        return HashSet::new();
    };
    let Ok(text) = crate::private_storage::read_to_string(&path, CACHE_MAX_BYTES) else {
        return HashSet::new();
    };
    match serde_json::from_str::<Vec<String>>(&text) {
        Ok(ids) if ids.len() <= CACHE_MAX_IDS && ids.iter().all(|id| valid_id(id)) => {
            let set: HashSet<String> = ids.into_iter().collect();
            tracing::debug!(count = set.len(), "loaded remembered unplayable ids");
            set
        }
        Ok(_) => {
            tracing::debug!("unplayable cache exceeded the safe item limit; ignoring");
            HashSet::new()
        }
        Err(err) => {
            tracing::debug!(?err, "unplayable cache unreadable; ignoring");
            HashSet::new()
        }
    }
}

/// Persist the ids. Best-effort: a failure here must never interrupt playback.
pub fn save(ids: &HashSet<String>) {
    let Some(path) = cache_file() else {
        return;
    };
    let mut sorted: Vec<&String> = ids
        .iter()
        .filter(|id| valid_id(id))
        .take(CACHE_MAX_IDS)
        .collect();
    sorted.sort(); // stable on disk, so the file does not churn
    let Ok(json) = serde_json::to_string(&sorted) else {
        return;
    };
    if json.len() > CACHE_MAX_BYTES {
        tracing::debug!("unplayable cache exceeded the safe size limit; not saving");
        return;
    }
    if let Err(err) = crate::private_storage::write(&path, json) {
        tracing::debug!(?err, "could not save unplayable cache");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_strictly_bounded() {
        assert!(valid_id("1440857781"));
        assert!(!valid_id(""));
        assert!(!valid_id("not-an-id"));
        assert!(!valid_id(&"1".repeat(CACHE_MAX_ID_BYTES + 1)));
    }
}
