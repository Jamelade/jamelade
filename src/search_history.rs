// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded, private, device-local catalog search history.
//!
//! Recording consent is a preference; the queries themselves are state and
//! live separately under XDG state. Turning recording off intentionally leaves
//! this file alone. Only an explicit remove or Clear History changes it.

const MAX_ENTRIES: usize = 16;
const MAX_QUERY_CHARS: usize = 160;
const MAX_QUERY_BYTES: usize = 512;
const MAX_FILE_BYTES: usize = 16 * 1024;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct History {
    #[serde(default)]
    entries: Vec<String>,
}

fn normalize(query: &str) -> Option<String> {
    let mut normalized: String = query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_QUERY_CHARS)
        .collect();
    while normalized.len() > MAX_QUERY_BYTES {
        normalized.pop();
    }
    (!normalized.is_empty()).then_some(normalized)
}

fn path() -> std::path::PathBuf {
    relm4::gtk::glib::user_state_dir()
        .join("jamelade")
        .join("search-history.json")
}

impl History {
    pub fn load() -> Self {
        let Ok(raw) = crate::private_storage::read_to_string(&path(), MAX_FILE_BYTES) else {
            return Self::default();
        };
        Self::from_json(&raw)
    }

    fn from_json(raw: &str) -> Self {
        let Ok(stored) = serde_json::from_str::<Self>(raw) else {
            return Self::default();
        };
        let mut cleaned = Self::default();
        // Stored newest-first. Reinsert oldest-first so `add` preserves that
        // ordering while applying today's validation and deduplication rules.
        for query in stored.entries.iter().rev() {
            cleaned.add_inner(query);
        }
        cleaned
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn add(&mut self, query: &str) -> bool {
        if !self.add_inner(query) {
            return false;
        }
        self.save();
        true
    }

    fn add_inner(&mut self, query: &str) -> bool {
        let Some(query) = normalize(query) else {
            return false;
        };
        let folded = query.to_lowercase();
        self.entries.retain(|old| old.to_lowercase() != folded);
        self.entries.insert(0, query);
        self.entries.truncate(MAX_ENTRIES);
        true
    }

    pub fn remove(&mut self, query: &str) -> bool {
        let folded = query.to_lowercase();
        let before = self.entries.len();
        self.entries.retain(|old| old.to_lowercase() != folded);
        if self.entries.len() == before {
            return false;
        }
        self.save();
        true
    }

    pub fn clear(&mut self) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        self.entries.clear();
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
            tracing::warn!(kind = ?error.kind(), "could not save search history");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_searches_are_bounded_and_case_insensitively_deduplicated() {
        let mut history = History::default();
        for i in 0..20 {
            history.add_inner(&format!("query {i}"));
        }
        assert_eq!(history.entries.len(), MAX_ENTRIES);
        assert_eq!(history.entries[0], "query 19");
        assert!(!history.entries.iter().any(|entry| entry == "query 0"));

        history.add_inner("QUERY 19");
        assert_eq!(history.entries.len(), MAX_ENTRIES);
        assert_eq!(history.entries[0], "QUERY 19");
    }

    #[test]
    fn malformed_or_excessive_stored_queries_are_normalized() {
        let raw = format!(
            r#"{{"entries":["  mf   doom  ","{}","", "MF DOOM"]}}"#,
            "a".repeat(MAX_QUERY_CHARS + 40)
        );
        let history = History::from_json(&raw);
        assert_eq!(history.entries[0], "mf doom");
        assert_eq!(history.entries.len(), 2);
        assert!(history.entries[1].len() <= MAX_QUERY_BYTES);
    }
}
