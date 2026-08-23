// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! Explore and optional lyrics.
//!
//! Both are deliberately kept out of the library loader: Explore and first-
//! party lyrics talk to Apple's authenticated API, while optional fallbacks use
//! an isolated, token-free client after explicit, per-provider privacy opt-ins.
//! Neither persists listening metadata to disk.

use relm4::ComponentSender;

use super::{AppModel, CommandMsg, View};
use crate::lyrics::{Lyrics, Providers, Query};

const LYRICS_CACHE_MAX: usize = 64;

impl AppModel {
    fn lyrics_wanted(&self) -> bool {
        self.view == View::Lyrics || self.settings.desktop_jamkin
    }

    fn lyrics_providers(&self) -> Providers {
        Providers {
            lrclib: self.settings.lyrics_enabled,
        }
    }

    /// Invalidate both an in-flight answer and the bounded memory cache after
    /// consent changes. The cache key does not contain provider preferences,
    /// so retaining it could show an answer from a provider just switched off
    /// or prevent a newly enabled fallback from filling an old empty result.
    pub(super) fn lyrics_provider_changed(&mut self, sender: &ComponentSender<Self>) {
        self.lyrics_generation = self.lyrics_generation.wrapping_add(1);
        self.lyrics_for = None;
        self.lyrics_loading = false;
        self.lyrics_cache.clear();
        if self.client().is_some() || self.lyrics_providers().any() {
            self.ensure_lyrics(sender);
        } else {
            self.lyrics_view.disabled();
            self.jamkin_mode.disabled();
        }
    }

    pub(super) fn load_explore(&mut self, sender: &ComponentSender<Self>) {
        if self.loading_explore || self.tried_explore {
            return;
        }
        let Some(client) = self.client() else {
            return;
        };

        self.tried_explore = true;
        self.loading_explore = true;
        self.explore_generation = self.explore_generation.wrapping_add(1);
        let generation = self.explore_generation;
        self.explore_view.loading();
        sender.oneshot_command(async move {
            CommandMsg::Explore {
                generation,
                result: client.explore().await.map_err(|err| format!("{err:#}")),
            }
        });
    }

    /// Ignore answers from a previous reload or signed-in account. Discovery
    /// data is account-derived even though it is not written to disk.
    pub(super) fn finish_explore(
        &mut self,
        generation: u64,
        result: Result<super::Explore, String>,
    ) {
        if generation != self.explore_generation || !matches!(self.stage, super::Stage::Ready) {
            return;
        }
        self.loading_explore = false;
        match result {
            Ok(explore) => {
                tracing::info!(sections = explore.sections.len(), "Explore loaded");
                self.explore_view.show(&explore);
            }
            Err(err) => {
                tracing::warn!(%err, "Explore failed");
                self.explore_view.fail(&err);
            }
        }
    }

    /// Bring both lyric surfaces into agreement with the privacy setting and
    /// the item MusicKit currently reports. The compact Jamkin consumes the
    /// same bounded in-memory answer; it never creates a second request.
    pub(super) fn ensure_lyrics(&mut self, sender: &ComponentSender<Self>) {
        if !self.lyrics_wanted() {
            return;
        }
        let providers = self.lyrics_providers();
        let apple = self.client();
        if apple.is_none() && !providers.any() {
            self.lyrics_loading = false;
            self.lyrics_for = None;
            self.lyrics_view.disabled();
            self.jamkin_mode.disabled();
            return;
        }

        let Some(item) = self.player.now_playing.as_ref() else {
            self.lyrics_loading = false;
            self.lyrics_for = None;
            self.lyrics_view.waiting();
            self.jamkin_mode.waiting();
            return;
        };
        let catalog_id = item.catalog_id.as_deref().or_else(|| {
            item.id
                .as_deref()
                .filter(|id| id.bytes().all(|byte| byte.is_ascii_digit()))
        });
        let Some(query) = Query::new(
            catalog_id,
            &item.title,
            &item.artist,
            &item.album,
            item.duration_ms,
        ) else {
            self.lyrics_loading = false;
            self.lyrics_for = None;
            self.lyrics_view.waiting();
            self.jamkin_mode.waiting();
            return;
        };

        if self.lyrics_for.as_ref() == Some(&query) {
            // Loading, successfully shown, or showing the error from the one
            // attempt: none should start a request loop. Leaving and returning
            // to the page clears `lyrics_for`, which is the explicit retry.
            self.sync_lyrics_position();
            return;
        }

        if let Some(lyrics) = self.lyrics_cache.get(&query).cloned() {
            self.lyrics_for = Some(query);
            self.lyrics_loading = false;
            self.lyrics_view.show(&lyrics);
            self.jamkin_mode.show(&lyrics);
            self.sync_lyrics_position();
            return;
        }

        self.lyrics_generation = self.lyrics_generation.wrapping_add(1);
        let generation = self.lyrics_generation;
        self.lyrics_for = Some(query.clone());
        self.lyrics_loading = true;
        self.lyrics_view.loading(&item.title);
        self.jamkin_mode.loading(&item.title);
        sender.oneshot_command(async move {
            let result = crate::lyrics::fetch(&query, providers, apple)
                .await
                .map_err(|err| format!("{err:#}"));
            CommandMsg::Lyrics {
                generation,
                query,
                result,
            }
        });
    }

    pub(super) fn sync_lyrics_position(&self) {
        if self.view == View::Lyrics {
            self.lyrics_view
                .sync_position(self.player.interpolated_position_ms());
        }
        if self.settings.desktop_jamkin {
            self.jamkin_mode
                .sync_position(self.player.interpolated_position_ms());
        }
    }

    pub(super) fn activate_discovery(&mut self, sender: &ComponentSender<Self>) {
        if !matches!(self.stage, super::Stage::Ready) {
            return;
        }
        if self.view == View::Explore {
            self.load_explore(sender);
        }
        // Jamkin Mode is independent of the navigation stack. In particular,
        // a track-change event must refresh its timeline while Songs, Explore,
        // or any detail page is visible, not only while Lyrics is selected.
        if self.lyrics_wanted() {
            self.ensure_lyrics(sender);
        }
    }

    pub(super) fn finish_lyrics(
        &mut self,
        generation: u64,
        query: Query,
        result: Result<Lyrics, String>,
    ) {
        if generation != self.lyrics_generation || self.lyrics_for.as_ref() != Some(&query) {
            return;
        }
        self.lyrics_loading = false;
        match result {
            Ok(lyrics) => {
                if self.lyrics_cache.len() >= LYRICS_CACHE_MAX
                    && !self.lyrics_cache.contains_key(&query)
                    && let Some(evicted) = self.lyrics_cache.keys().next().cloned()
                {
                    self.lyrics_cache.remove(&evicted);
                }
                self.lyrics_cache.insert(query, lyrics.clone());
                self.lyrics_view.show(&lyrics);
                self.jamkin_mode.show(&lyrics);
                self.sync_lyrics_position();
            }
            Err(err) => {
                // Deliberately no track fields in this log: the error describes
                // the provider/transport, and listening metadata stays out.
                tracing::warn!(%err, "lyrics request failed");
                self.lyrics_view.fail(&err);
                self.jamkin_mode.fail();
            }
        }
    }

    pub(super) fn clear_discovery_session(&mut self) {
        self.explore_generation = self.explore_generation.wrapping_add(1);
        self.loading_explore = false;
        self.tried_explore = false;
        self.explore_view.clear();

        self.lyrics_generation = self.lyrics_generation.wrapping_add(1);
        self.lyrics_for = None;
        self.lyrics_loading = false;
        self.lyrics_cache.clear();
        if self.client().is_some() || self.lyrics_providers().any() {
            self.lyrics_view.waiting();
            self.jamkin_mode.waiting();
        } else {
            self.lyrics_view.disabled();
            self.jamkin_mode.disabled();
        }
    }
}
