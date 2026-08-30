// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! The navigation stack: pushing an album or artist page, and taking it away.
//!
//! Pages are addressed by **id**, never by their depth in the stack. By the
//! time a fetch lands or a click arrives the stack may have moved, and an index
//! that was right when the page was built is a wrong answer that looks like a
//! right one — the same lesson `queue.rs` is built around.

use relm4::ComponentSender;
use relm4::adw::prelude::NavigationPageExt;

use super::{
    ART_SIZE, AppModel, AppMsg, CommandMsg, DetailActions, DetailPage, PageKind, RowState, artwork,
};

/// How somebody reached a page, which is the only thing that distinguishes the
/// two once it is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Arrival {
    /// Drilled into from the list underneath — a grid tile, a queue row's menu.
    /// Going back means the thing you came from.
    PushedOnto,
    /// Chosen in the sidebar. There is nothing behind it, so no back button.
    FromTheSidebar,
}
use crate::music::types::{Artwork, Track};

/// The most covers a mosaic uses. Fewer is fine — they divide the square
/// between them rather than leaving holes; see `artwork::layout`.
pub(super) const MOSAIC_TILES: usize = 4;

/// The first `want` **distinct** covers, in order.
///
/// Distinct by cache key, and that is the whole subtlety: a playlist drawn from
/// two albums would otherwise tile the same sleeve twice over, which reads as a
/// rendering fault rather than as a playlist.
///
/// Takes an iterator rather than `&[Track]` so it can be tested without
/// building a `Track`, which has no `Default` and a dozen fields.
pub(super) fn distinct_covers<'a>(
    arts: impl Iterator<Item = &'a Artwork>,
    want: usize,
) -> Vec<Artwork> {
    let mut seen = std::collections::HashSet::new();
    let mut covers = Vec::with_capacity(want);
    for art in arts {
        if seen.insert(art.cache_key()) {
            covers.push(art.clone());
            if covers.len() == want {
                break;
            }
        }
    }
    covers
}

/// The covers a playlist's mosaic would be built from — up to
/// [`MOSAIC_TILES`], and any number below that is still a picture.
pub(super) fn playlist_covers(tracks: &[Track]) -> Vec<Artwork> {
    distinct_covers(
        tracks.iter().filter_map(|track| track.artwork.as_ref()),
        MOSAIC_TILES,
    )
}

impl AppModel {
    /// Push an album or artist page and ask Apple to fill it.
    ///
    /// The page appears immediately with a spinner rather than after the
    /// request lands: a click that does nothing for a second reads as a click
    /// that did not register, and the second click pushes it twice.
    pub(super) fn push_page(&mut self, kind: PageKind, sender: &ComponentSender<Self>) {
        self.open_page(kind, sender, Arrival::PushedOnto);
    }

    /// Open a page, saying how the person got to it.
    ///
    /// The two arrivals look identical once the page is up and must not: a
    /// drill-down has somewhere to go back *to*, and a sidebar destination does
    /// not — you did not navigate to it, you selected it.
    pub(super) fn open_page(
        &mut self,
        kind: PageKind,
        sender: &ComponentSender<Self>,
        arrival: Arrival,
    ) {
        let Some(client) = self.client() else {
            self.toast("Not connected yet");
            return;
        };

        let id = self.next_page_id;
        self.next_page_id += 1;
        let generation = self.account_generation;

        let activate = sender.clone();
        let play = sender.clone();
        let shuffle = sender.clone();
        let copy_link = sender.clone();
        let export_playlist = sender.clone();
        let album_artist = sender.clone();
        let artist_activate = sender.clone();
        let sidebar = sender.clone();
        let page = DetailPage::new(
            id,
            kind.clone(),
            RowState {
                overrides: self.row_overrides.clone(),
                current: self.current_track.clone(),
                dead: self.dead_rows.clone(),
            },
            DetailActions {
                activate: Box::new(move |row| {
                    activate.input(AppMsg::DetailActivated { page: id, row })
                }),
                play: Box::new(move || {
                    play.input(AppMsg::PlayPage {
                        page: id,
                        shuffle: false,
                    })
                }),
                shuffle: Box::new(move || {
                    shuffle.input(AppMsg::PlayPage {
                        page: id,
                        shuffle: true,
                    })
                }),
                copy_link: Box::new(move || copy_link.input(AppMsg::CopyPageLink { page: id })),
                export_playlist: Box::new(move |format| {
                    export_playlist.input(AppMsg::ExportPlaylist { page: id, format })
                }),
                album_artist: Box::new(move || {
                    album_artist.input(AppMsg::OpenAlbumArtist { page: id })
                }),
                request_art: self.tile_art_request.clone(),
                artist_activate: Box::new(move |target| {
                    artist_activate.input(AppMsg::ArtistActivatedOnPage { page: id, target })
                }),
                toggle_sidebar: Box::new(move || sidebar.input(AppMsg::ToggleSidebar)),
            },
        );
        page.set_end_controls(true);

        let pushed_onto = matches!(arrival, Arrival::PushedOnto);
        // **A sidebar destination has no back button.** `can_pop(false)` removes
        // it and disables the back gesture and Escape, while leaving
        // programmatic `pop` working — which `pop_to_results` still needs when a
        // section is chosen.
        page.widget().set_can_pop(pushed_onto);
        // **And it does not slide in.** A drill-down comes from the list you
        // were looking at, and the slide says so. A destination came from the
        // sidebar, the same as a section, and sections do not slide — an
        // animation that says "you went deeper" is the one thing still claiming
        // this is a stack.
        //
        // Suppressed around the push rather than turned off: pushes from a grid
        // tile still animate, because for those it is true.
        self.nav.set_animate_transitions(pushed_onto);
        self.nav.push(page.widget());
        self.nav.set_animate_transitions(true);
        // Keep the sidebar independently dismissible on every detail page.
        // A drill-down already owns a Back button, but Back and Sidebar answer
        // different questions: popping the album must never be the price of
        // hiding a pane that happens to be open beside it.
        page.show_sidebar_toggle(true);
        page.set_sidebar_shown(self.show_sidebar);
        self.pages.push(page);

        let catalog_id = kind.id().to_owned();
        tracing::info!(page = id, "opening page");
        match kind {
            PageKind::Album(_) => sender.oneshot_command(async move {
                CommandMsg::AlbumPage {
                    generation,
                    page: id,
                    result: client
                        .album(&catalog_id)
                        .await
                        .map_err(|err| format!("{err:#}")),
                }
            }),
            PageKind::LibraryAlbum(_) => sender.oneshot_command(async move {
                CommandMsg::AlbumPage {
                    generation,
                    page: id,
                    result: client
                        .library_album(&catalog_id)
                        .await
                        .map_err(|err| format!("{err:#}")),
                }
            }),
            PageKind::Artist(_) => sender.oneshot_command(async move {
                CommandMsg::ArtistPage {
                    generation,
                    page: id,
                    result: client
                        .artist_albums(&catalog_id)
                        .await
                        .map_err(|err| format!("{err:#}")),
                }
            }),
            PageKind::Playlist(_) => sender.oneshot_command(async move {
                CommandMsg::PlaylistPage {
                    generation,
                    page: id,
                    result: client
                        .playlist(&catalog_id)
                        .await
                        .map_err(|err| format!("{err:#}")),
                }
            }),
            PageKind::LibraryPlaylist(_) => sender.oneshot_command(async move {
                CommandMsg::PlaylistPage {
                    generation,
                    page: id,
                    result: client
                        .library_playlist(&catalog_id)
                        .await
                        .map_err(|err| format!("{err:#}")),
                }
            }),
            PageKind::LibraryArtist(_) => sender.oneshot_command(async move {
                CommandMsg::ArtistPage {
                    generation,
                    page: id,
                    result: client
                        .library_artist_albums(&catalog_id)
                        .await
                        .map_err(|err| format!("{err:#}")),
                }
            }),
        }
    }

    /// Re-read any open detail page for the playlist Apple just accepted a
    /// write to. The response body is intentionally not trusted as final
    /// state; fetching the exact resource also avoids making every open page
    /// flash after an unrelated playlist changes.
    pub(super) fn refresh_library_playlist_pages(
        &self,
        playlist_id: &str,
        expected_catalog_id: &str,
        sender: &ComponentSender<Self>,
    ) {
        let Some(client) = self.client() else {
            return;
        };
        let page_ids: Vec<u64> = self
            .pages
            .iter()
            .filter_map(|page| match page.kind() {
                PageKind::LibraryPlaylist(id) if id == playlist_id => Some(page.id),
                _ => None,
            })
            .collect();
        let generation = self.account_generation;

        for page in page_ids {
            let client = client.clone();
            let playlist_id = playlist_id.to_owned();
            let expected_catalog_id = expected_catalog_id.to_owned();
            sender.oneshot_command(async move {
                // Apple documents the append as 202 Accepted: the write can be
                // acknowledged before a read sees it. Retry only this bounded,
                // idempotent GET, and stop as soon as the expected song appears.
                let mut latest = client.library_playlist(&playlist_id).await;
                for delay_ms in [650, 1_500] {
                    if latest.as_ref().is_ok_and(|(_, tracks)| {
                        tracks.iter().any(|track| {
                            track.catalog_id.as_deref() == Some(expected_catalog_id.as_str())
                        })
                    }) {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    latest = client.library_playlist(&playlist_id).await;
                    if latest.as_ref().is_ok_and(|(_, tracks)| {
                        tracks.iter().any(|track| {
                            track.catalog_id.as_deref() == Some(expected_catalog_id.as_str())
                        })
                    }) {
                        break;
                    }
                }
                CommandMsg::PlaylistPage {
                    generation,
                    page,
                    result: latest.map_err(|err| format!("{err:#}")),
                }
            });
        }
    }

    /// Return to the results list, dropping whatever was pushed over it.
    ///
    ///
    /// Cheap when there is nothing to do, so callers do not have to check.
    pub(super) fn pop_to_results(&mut self) {
        if self.pages.is_empty() {
            return;
        }
        tracing::debug!(depth = self.pages.len(), "returning to results");

        // **A destination leaves the way it arrived: without a slide.** The
        // push is already silent, so animating only the way out made choosing a
        // section look like going back — which is the story a destination is
        // not telling. A drill-down still slides both ways, because for one of
        // those "back" is exactly what this is.
        //
        // `can_pop` is what says which: it is false only on a destination, set
        // where the page is opened.
        let leaving_destination = self.nav.visible_page().is_some_and(|page| !page.can_pop());
        self.nav.set_animate_transitions(!leaving_destination);
        // `connect_popped` fires per page and empties `self.pages` for us —
        // clearing it here as well would be a second source of truth for the
        // same thing.
        self.nav.pop_to_tag("results");
        self.nav.set_animate_transitions(true);
    }

    /// Keep the pushed pages' headers agreeing with the root about who owns the
    /// window controls. When the queue is open it is the rightmost pane, so the
    /// controls belong to its header — a page that still draws them puts a
    /// second close button in the middle of the window.
    pub(super) fn sync_page_controls(&self) {
        for page in &self.pages {
            page.set_end_controls(true);
        }
    }

    /// A playlist page's header picture: Apple's if there is one, ours if not.
    ///
    /// Only playlists reach the second branch. Albums and artists always have
    /// artwork of their own — an artist's comes from the catalog twin — but
    /// **Apple sends nothing for a playlist you made yourself**, which is why
    /// the app showed an empty sleeve where its own web player shows a mosaic.
    ///
    /// This is the one place Slipmat draws a picture Apple did not send, and it
    /// is deliberately the *cheap* one: a page has already loaded its tracks,
    /// so the covers are known and mostly cached. The grid cannot do this — a
    /// tile knows no tracks, so it would cost a request per artless playlist.
    pub(super) fn fetch_page_art_or_mosaic(
        &self,
        page: u64,
        art: Option<Artwork>,
        covers: Vec<Artwork>,
        sender: &ComponentSender<Self>,
    ) {
        if art.is_some() {
            self.fetch_page_art(page, art, sender);
            return;
        }
        // **One cover is not a mosaic, it is a cover.** A playlist drawn from a
        // single album gets that album's sleeve, fetched at page size through
        // the ordinary path — sharper than compositing one picture into a
        // square we would then have to invent a name for.
        if covers.len() == 1 {
            self.fetch_page_art(page, covers.into_iter().next(), sender);
            return;
        }
        if covers.is_empty() {
            tracing::warn!(page, "no track covers; keeping the empty sleeve");
            return;
        }
        let generation = self.account_generation;
        sender.oneshot_command(async move {
            CommandMsg::PageArtwork {
                generation,
                page,
                // `TILE_ART` is what the grids fetch, so a library playlist's
                // covers are usually on disk already and the mosaic costs no
                // download at all. Passed from here rather than derived inside,
                // so the coupling to the grids is visible at the call site.
                path: crate::components::mosaic::mosaic(covers, ART_SIZE, super::TILE_ART).await,
            }
        });
    }

    /// Fetch a page's header art, once we know what it is.
    pub(super) fn fetch_page_art(
        &self,
        page: u64,
        art: Option<Artwork>,
        sender: &ComponentSender<Self>,
    ) {
        // **The quietest of the three exits, and the one that matters.** A page
        // whose resource carries no artwork at all keeps its placeholder having
        // asked for nothing — indistinguishable, from the outside, from a fetch
        // that failed or one still in flight. Note this is the artwork on the
        // *details* response, which is a different request from the one the
        // grid's `with_art` counter measures: a playlist can carry a picture in
        // one and not the other.
        let Some(art) = art else {
            tracing::warn!(page, "page resource carries no artwork; nothing to fetch");
            return;
        };
        let generation = self.account_generation;
        sender.oneshot_command(async move {
            // A missing cover is cosmetic — `None` and the page keeps its
            // placeholder, exactly as the Now Playing bar does. **Cosmetic is
            // not the same as unexplained**, and this was `.ok()`: a header
            // that silently kept its placeholder was indistinguishable from a
            // playlist Apple has no picture for, which is exactly how a
            // playlist mosaic that used to render went missing without a word.
            // The error carries the URL it tried — see `artwork::fetch`.
            let path = match artwork::fetch(art, ART_SIZE).await {
                Ok(path) => Some(path),
                Err(_) => {
                    tracing::warn!(page, "page artwork not fetched");
                    None
                }
            };
            CommandMsg::PageArtwork {
                generation,
                page,
                path,
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn art(n: &str) -> Artwork {
        Artwork::new(format!("https://is1.mzstatic.com/{n}/{{w}}x{{h}}bb.jpg"))
    }

    #[test]
    fn a_mosaic_never_repeats_a_cover() {
        // The point of "distinct". A playlist drawn from two albums would
        // otherwise tile the same sleeve twice, which reads as a rendering
        // fault rather than as a playlist.
        let arts = [art("a"), art("a"), art("b"), art("a"), art("b")];
        let picked = distinct_covers(arts.iter(), MOSAIC_TILES);

        assert_eq!(picked.len(), 2, "two albums can only give two quadrants");
        assert_eq!(picked[0].cache_key(), art("a").cache_key());
        assert_eq!(picked[1].cache_key(), art("b").cache_key());
    }

    #[test]
    fn it_stops_at_four_however_long_the_playlist() {
        // 117 tracks was the real case; walking all of them to fill four slots
        // is work done for nothing.
        let arts: Vec<_> = (0..200).map(|i| art(&i.to_string())).collect();
        assert_eq!(
            distinct_covers(arts.iter(), MOSAIC_TILES).len(),
            MOSAIC_TILES
        );
    }

    #[test]
    fn order_follows_the_playlist() {
        // Top-left is the first track, so the mosaic matches what the list
        // underneath it shows.
        let arts = [art("z"), art("y"), art("x"), art("w")];
        let picked = distinct_covers(arts.iter(), MOSAIC_TILES);
        let keys: Vec<_> = picked.iter().map(Artwork::cache_key).collect();
        let want: Vec<_> = arts.iter().map(Artwork::cache_key).collect();
        assert_eq!(keys, want);
    }

    #[test]
    fn too_few_covers_yields_too_few() {
        // The caller keeps the empty sleeve rather than drawing a half grid.
        assert!(distinct_covers([art("a")].iter(), MOSAIC_TILES).len() < MOSAIC_TILES);
        assert!(distinct_covers([].iter(), MOSAIC_TILES).is_empty());
    }
}
