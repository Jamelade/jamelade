// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! Artwork for user playlists in the library grid.
//!
//! Apple sends no `artwork` for playlists a person made, while its web player
//! quietly builds a collage from the tracks. Grid tiles do not otherwise know
//! those tracks, so they ask here. Requests are bounded, results are cached,
//! and only the first relationship page is read — enough to find four distinct
//! album covers without walking a thousand-track playlist.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

use relm4::ComponentSender;

use super::{AppModel, AppMsg, CommandMsg, TILE_ART};
use crate::components::artwork::{self, Decoded};
use crate::components::grid_item::{PlaylistArtRequest, playlist_art_key};
use crate::music::client::Client;
use crate::music::types::{Artwork, Playlist};

/// Four requests keep a large playlist library responsive without turning one
/// grid visit into an unbounded burst against Apple's API.
const CONCURRENCY: usize = 4;

pub(crate) struct Job {
    key: String,
    playlist_id: String,
    covers: Vec<Artwork>,
}

pub(crate) struct Finished {
    generation: u64,
    key: String,
    playlist_id: String,
    covers: Vec<Artwork>,
    path: Option<PathBuf>,
    cover: Option<Decoded>,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaylistArtJob")
            .field("key", &self.key)
            .field("covers", &self.covers.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Finished {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaylistArtFinished")
            .field("key", &self.key)
            .field("covers", &self.covers.len())
            .field("has_path", &self.path.is_some())
            .field("has_pixels", &self.cover.is_some())
            .finish_non_exhaustive()
    }
}

pub(super) struct State {
    request: PlaylistArtRequest,
    pending: HashSet<String>,
    queued: VecDeque<Queued>,
    active: usize,
    paths: HashMap<String, PathBuf>,
    library_dirty: bool,
    generation: u64,
}

struct Queued {
    generation: u64,
    job: Job,
}

impl State {
    pub fn new(sender: &ComponentSender<AppModel>) -> Self {
        let out = sender.clone();
        let request: PlaylistArtRequest = Rc::new(move |key, playlist_id, covers| {
            out.input(AppMsg::NeedPlaylistArt(Job {
                key,
                playlist_id,
                covers,
            }));
        });
        Self {
            request,
            pending: HashSet::new(),
            queued: VecDeque::new(),
            active: 0,
            paths: HashMap::new(),
            library_dirty: false,
            generation: 0,
        }
    }

    pub fn request(&self) -> PlaylistArtRequest {
        self.request.clone()
    }

    fn enqueue(&mut self, job: Job) {
        if self.pending.insert(job.key.clone()) {
            self.queued.push_back(Queued {
                generation: self.generation,
                job,
            });
        }
    }

    fn start(&mut self, client: Option<&Client>, sender: &ComponentSender<AppModel>) {
        while self.active < CONCURRENCY {
            let Some(queued) = self.queued.pop_front() else {
                break;
            };
            let cached = self.paths.get(&queued.job.key).cloned();
            // A cached library can bind before the sidecar has returned its
            // session. Keep the work queued; `wake` runs when it arrives.
            if cached.is_none() && queued.job.covers.is_empty() && client.is_none() {
                self.queued.push_front(queued);
                break;
            }
            let client = if cached.is_none() && queued.job.covers.is_empty() {
                client.cloned()
            } else {
                None
            };
            self.active += 1;
            sender.oneshot_command(async move {
                CommandMsg::PlaylistTileArt(run(queued, cached, client).await)
            });
        }
    }

    fn complete(&mut self, result: &Finished) -> bool {
        if result.generation != self.generation {
            return false;
        }
        self.active = self.active.saturating_sub(1);
        self.pending.remove(&result.key);
        if let Some(path) = &result.path {
            self.paths.insert(result.key.clone(), path.clone());
        } else {
            self.paths.remove(&result.key);
        }
        true
    }

    fn save_due(&mut self) -> bool {
        if self.active == 0 && self.queued.is_empty() && self.library_dirty {
            self.library_dirty = false;
            true
        } else {
            false
        }
    }

    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending.clear();
        self.queued.clear();
        self.active = 0;
        self.paths.clear();
        self.library_dirty = false;
    }
}

async fn run(queued: Queued, cached: Option<PathBuf>, client: Option<Client>) -> Finished {
    let generation = queued.generation;
    let Job {
        key,
        playlist_id,
        mut covers,
    } = queued.job;

    if let Some(path) = cached {
        let cover = artwork::decode(&path, TILE_ART as i32);
        return Finished {
            generation,
            key,
            playlist_id,
            covers,
            path: cover.is_some().then_some(path),
            cover,
        };
    }

    if covers.is_empty() {
        let Some(client) = client else {
            return Finished {
                generation,
                key,
                playlist_id,
                covers,
                path: None,
                cover: None,
            };
        };
        covers = match client.library_playlist_preview(&playlist_id).await {
            Ok(tracks) => super::pages::playlist_covers(&tracks),
            Err(_) => {
                // The API context contains the private playlist id. The hashed
                // key is enough to correlate retries without logging it.
                tracing::warn!("playlist mosaic tracks not fetched");
                Vec::new()
            }
        };
    }

    let (path, cover) = match covers.as_slice() {
        [] => (None, None),
        [only] => artwork::load_tile(only.clone(), TILE_ART, &key).await,
        _ => {
            let path = crate::components::mosaic::mosaic(covers.clone(), TILE_ART, TILE_ART).await;
            let cover = path
                .as_deref()
                .and_then(|path| artwork::decode(path, TILE_ART as i32));
            (path, cover)
        }
    };

    Finished {
        generation,
        key,
        playlist_id,
        covers,
        path,
        cover,
    }
}

/// Preserve a known collage when a refresh says the playlist itself did not
/// change. API resources never carry this local-only field.
pub(super) fn carry_cached_covers(old: &[Playlist], fresh: &mut [Playlist]) {
    let known: HashMap<(&str, &str), &[Artwork]> = old
        .iter()
        .filter(|playlist| {
            !playlist.last_modified.is_empty() && !playlist.mosaic_artwork.is_empty()
        })
        .map(|playlist| {
            (
                (playlist.id.as_str(), playlist.last_modified.as_str()),
                playlist.mosaic_artwork.as_slice(),
            )
        })
        .collect();
    for playlist in fresh {
        if playlist.artwork.is_none()
            && let Some(covers) =
                known.get(&(playlist.id.as_str(), playlist.last_modified.as_str()))
        {
            playlist.mosaic_artwork = covers.to_vec();
        }
    }
}

impl AppModel {
    fn playlist_art_client(&self) -> Option<Client> {
        if !self.apple_session.as_ref()?.has_user_token {
            return None;
        }
        self.client()
    }

    pub(super) fn need_playlist_art(&mut self, job: Job, sender: &ComponentSender<Self>) {
        self.playlist_art.enqueue(job);
        let client = self.playlist_art_client();
        self.playlist_art.start(client.as_ref(), sender);
    }

    pub(super) fn finish_playlist_art(&mut self, result: Finished, sender: &ComponentSender<Self>) {
        if !self.playlist_art.complete(&result) {
            // An explicit sign-out invalidates in-flight account artwork. A
            // task that finished just after the sweep must not put a private
            // playlist mosaic back on disk, including if another account has
            // already signed in by the time the result reaches the UI thread.
            if let Some(path) = result.path {
                let _ = std::fs::remove_file(path);
            }
            return;
        }

        if !result.covers.is_empty()
            && let Some(playlist) = self.playlists.iter_mut().find(|playlist| {
                playlist.id == result.playlist_id && playlist_art_key(playlist) == result.key
            })
            && playlist.mosaic_artwork != result.covers
        {
            playlist.mosaic_artwork = result.covers.clone();
            self.playlist_art.library_dirty = true;
        }

        if let Some(cover) = result.cover {
            let texture = cover.into_texture();
            for widget in self
                .playlist_art_widgets
                .borrow()
                .get(&result.key)
                .into_iter()
                .flatten()
            {
                widget.set_texture(&texture);
            }
        }

        let client = self.playlist_art_client();
        self.playlist_art.start(client.as_ref(), sender);
        if self.playlist_art.save_due() {
            self.save_cache();
        }
    }

    pub(super) fn wake_playlist_art(&mut self, sender: &ComponentSender<Self>) {
        let client = self.playlist_art_client();
        self.playlist_art.start(client.as_ref(), sender);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playlist(id: &str, modified: &str, covers: Vec<Artwork>) -> Playlist {
        Playlist {
            id: id.into(),
            date_added: String::new(),
            last_modified: modified.into(),
            name: id.into(),
            curator: String::new(),
            description: String::new(),
            artwork: None,
            mosaic_artwork: covers,
            share_url: None,
            library: true,
        }
    }

    #[test]
    fn refresh_keeps_covers_only_for_an_unchanged_playlist() {
        let cover = Artwork::new("https://is1.mzstatic.com/a/{w}x{h}bb.jpg");
        let old = [playlist("p.1", "before", vec![cover.clone()])];
        let mut same = [playlist("p.1", "before", Vec::new())];
        carry_cached_covers(&old, &mut same);
        assert_eq!(same[0].mosaic_artwork, vec![cover.clone()]);

        let mut changed = [playlist("p.1", "after", Vec::new())];
        carry_cached_covers(&old, &mut changed);
        assert!(changed[0].mosaic_artwork.is_empty());

        let undated_old = [playlist("p.1", "", vec![cover])];
        let mut undated_fresh = [playlist("p.1", "", Vec::new())];
        carry_cached_covers(&undated_old, &mut undated_fresh);
        assert!(undated_fresh[0].mosaic_artwork.is_empty());
    }

    #[test]
    fn apple_artwork_wins_over_a_local_collage() {
        let cover = Artwork::new("https://is1.mzstatic.com/a/{w}x{h}bb.jpg");
        let old = [playlist("p.1", "same", vec![cover.clone()])];
        let mut fresh = [playlist("p.1", "same", Vec::new())];
        fresh[0].artwork = Some(cover);
        carry_cached_covers(&old, &mut fresh);
        assert!(fresh[0].mosaic_artwork.is_empty());
    }

    #[test]
    fn debug_output_never_names_a_private_playlist() {
        let job = Job {
            key: "playlist-deadbeef".into(),
            playlist_id: "p.private-library-id".into(),
            covers: vec![Artwork::new("https://is1.mzstatic.com/private-cover.jpg")],
        };
        let shown = format!("{job:?}");

        assert!(shown.contains("playlist-deadbeef"));
        assert!(!shown.contains("private-library-id"));
        assert!(!shown.contains("private-cover"));
    }
}
