// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Per-song lyric timing corrections. The file contains only numeric catalog
//! IDs and millisecond offsets—never lyric text, titles, artists, or history.

use std::collections::BTreeMap;
use std::path::PathBuf;

const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_ENTRIES: usize = 4_096;
pub const MIN_OFFSET_MS: i32 = -10_000;
pub const MAX_OFFSET_MS: i32 = 10_000;
pub const STEP_MS: i32 = 250;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Offsets {
    #[serde(default)]
    values: BTreeMap<String, i32>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|byte| byte.is_ascii_digit())
}

fn path() -> PathBuf {
    relm4::gtk::glib::user_config_dir()
        .join("jamelade")
        .join("lyric-offsets.json")
}

impl Offsets {
    pub fn load() -> Self {
        let Ok(raw) = crate::private_storage::read_to_string(&path(), MAX_FILE_BYTES) else {
            return Self::default();
        };
        Self::from_json(&raw)
    }

    fn from_json(raw: &str) -> Self {
        let Ok(mut loaded) = serde_json::from_str::<Self>(raw) else {
            return Self::default();
        };
        loaded
            .values
            .retain(|id, value| valid_id(id) && (MIN_OFFSET_MS..=MAX_OFFSET_MS).contains(value));
        while loaded.values.len() > MAX_ENTRIES {
            let Some(first) = loaded.values.keys().next().cloned() else {
                break;
            };
            loaded.values.remove(&first);
        }
        loaded
    }

    pub fn get(&self, catalog_id: Option<&str>) -> i32 {
        catalog_id
            .filter(|id| valid_id(id))
            .and_then(|id| self.values.get(id).copied())
            .unwrap_or(0)
    }

    pub fn set(&mut self, catalog_id: &str, offset_ms: i32) -> bool {
        if !valid_id(catalog_id) {
            return false;
        }
        let offset_ms = offset_ms.clamp(MIN_OFFSET_MS, MAX_OFFSET_MS);
        if offset_ms == 0 {
            self.values.remove(catalog_id);
        } else {
            if !self.values.contains_key(catalog_id) && self.values.len() >= MAX_ENTRIES {
                let Some(oldest) = self.values.keys().next().cloned() else {
                    return false;
                };
                self.values.remove(&oldest);
            }
            self.values.insert(catalog_id.to_owned(), offset_ms);
        }
        self.save();
        true
    }

    fn save(&self) {
        let Ok(data) = serde_json::to_vec(self) else {
            return;
        };
        if data.len() <= MAX_FILE_BYTES
            && let Err(error) = crate::private_storage::write(&path(), data)
        {
            tracing::warn!(kind = ?error.kind(), "could not save lyric timing corrections");
        }
    }
}

/// Convert the real playback clock to the lyric file's clock. A positive
/// correction delays the words; a negative one brings them forward.
pub fn lyric_clock(position_ms: u64, offset_ms: i32) -> u64 {
    if offset_ms >= 0 {
        position_ms.saturating_sub(offset_ms as u64)
    } else {
        position_ms.saturating_add(offset_ms.unsigned_abs() as u64)
    }
}

/// Convert a lyric timestamp back to the seek position in the recording.
pub fn playback_clock(lyric_ms: u64, offset_ms: i32) -> u64 {
    if offset_ms >= 0 {
        lyric_ms.saturating_add(offset_ms as u64)
    } else {
        lyric_ms.saturating_sub(offset_ms.unsigned_abs() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrections_contain_only_numeric_ids_and_bounded_offsets() {
        let loaded = Offsets::from_json(
            r#"{"values":{"1000000001":500,"bad/title":200,"1000000002":99999}}"#,
        );
        assert_eq!(loaded.get(Some("1000000001")), 500);
        assert_eq!(loaded.get(Some("bad/title")), 0);
        assert_eq!(loaded.get(Some("1000000002")), 0);
    }

    #[test]
    fn positive_offsets_delay_and_negative_offsets_advance() {
        assert_eq!(lyric_clock(10_000, 500), 9_500);
        assert_eq!(playback_clock(9_500, 500), 10_000);
        assert_eq!(lyric_clock(10_000, -500), 10_500);
        assert_eq!(playback_clock(10_500, -500), 10_000);
    }
}
