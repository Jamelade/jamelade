// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Empty-search dashboard state: private recent queries and Apple chart rows.

use relm4::ComponentSender;

use super::{AppModel, AppMsg, CommandMsg, Entry, SearchLandingAction, Stage, Start, View};

impl AppModel {
    pub(super) fn record_current_search(&mut self) {
        if self.view != View::Search || !self.settings.search_history_enabled {
            return;
        }
        if self.search_history.add(&self.catalog_query) {
            self.search_landing
                .set_history(self.search_history.entries());
        }
    }

    pub(super) fn handle_search_landing(
        &mut self,
        action: SearchLandingAction,
        sender: &ComponentSender<Self>,
        root: &relm4::adw::ApplicationWindow,
    ) {
        match action {
            SearchLandingAction::Search(query) => {
                self.searching = true;
                self.sync_entry = true;
                self.handle(AppMsg::SearchChanged(query), sender, root);
                self.record_current_search();
            }
            SearchLandingAction::Remove(query) => {
                if self.search_history.remove(&query) {
                    self.search_landing
                        .set_history(self.search_history.entries());
                }
            }
            SearchLandingAction::Clear => {
                if self.search_history.clear() {
                    self.search_landing.set_history(&[]);
                }
            }
            SearchLandingAction::PlayTracks { tracks, start } => {
                let entries: Vec<_> = tracks.into_iter().map(Entry::Song).collect();
                self.play_entries(&entries, start, Start::Clicked);
            }
        }
    }

    pub(super) fn load_search_trending(&mut self, sender: &ComponentSender<Self>) {
        if self.loading_search_trending || self.tried_search_trending {
            return;
        }
        let Some(client) = self.client() else {
            return;
        };
        self.tried_search_trending = true;
        self.loading_search_trending = true;
        self.search_trending_generation = self.search_trending_generation.wrapping_add(1);
        let generation = self.search_trending_generation;
        sender.oneshot_command(async move {
            CommandMsg::SearchTrending {
                generation,
                result: client
                    .trending_tracks()
                    .await
                    .map_err(|error| format!("{error:#}")),
            }
        });
    }

    pub(super) fn finish_search_trending(
        &mut self,
        generation: u64,
        result: Result<Vec<super::Track>, String>,
    ) {
        if generation != self.search_trending_generation || !matches!(self.stage, Stage::Ready) {
            return;
        }
        self.loading_search_trending = false;
        match result {
            Ok(tracks) => {
                tracing::debug!(count = tracks.len(), "search landing chart loaded");
                self.search_landing.set_trending(tracks);
            }
            Err(error) => {
                // Optional decoration. The useful local history and categories
                // remain on screen even if Apple's chart endpoint is transient.
                tracing::debug!(%error, "search landing chart unavailable");
                self.search_landing.set_trending(Vec::new());
            }
        }
    }

    pub(super) fn clear_search_landing_session(&mut self) {
        self.search_trending_generation = self.search_trending_generation.wrapping_add(1);
        self.loading_search_trending = false;
        self.tried_search_trending = false;
        self.search_landing.set_trending(Vec::new());
    }
}
