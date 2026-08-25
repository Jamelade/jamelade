// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! Typed native projection of Apple Music data obtained through the browser.
//!
//! MusicKit's authenticated in-page client owns the origin, headers, cookies,
//! developer token and Music User Token. Rust sends a bounded relative path
//! through the sidecar broker and receives only a status plus JSON body.
//!
//! M5 fills in the library and catalog calls. What exists now is the client
//! itself plus the error diagnosis, because "errors name the fix" is easier to
//! honour if it's there from the first request rather than retrofitted.

use anyhow::{Context, Result};
use reqwest::StatusCode;
use std::collections::HashMap;

use serde::Deserialize;

use crate::player::protocol::ApiMethod;
use crate::player::sidecar::{ApiReply, Broker, BrokerError};

use super::types::{
    Album, AlbumAttributes, AlbumResource, Artist, ArtistAttributes, ArtistPageData,
    ArtistResource, Explore, ExploreItem, ExploreSection, LibraryArtistResource, Playlist,
    PlaylistAttributes, RelationshipData, Resource, Response, SongAttributes, SongContainers,
    Station, StationAttributes, Track,
};

/// What a catalog search turned up. Apple returns each kind in its own array;
/// this keeps them apart rather than flattening, because the UI shows them as
/// different things.
#[derive(Debug, Default)]
pub struct SearchResults {
    pub songs: Vec<Track>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    pub playlists: Vec<Playlist>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongCredit {
    pub role: String,
    pub names: Vec<String>,
}

/// A playlist long enough to hit this is a playlist nobody scrolls. Bounded so
/// one enormous one cannot spin through fifty requests.
const PLAYLIST_MAX: usize = 1_000;
/// Enough tracks to find four distinct covers without reading a whole playlist.
const PLAYLIST_PREVIEW: usize = 40;

/// Apple's hard cap for a library page. Asking for more is silently clamped.
const LIBRARY_PAGE: usize = 100;
/// How many library pages to fetch at once. MusicKit's browser client can
/// produce transient gateway timeouts when six authenticated reads arrive at
/// once, especially just after login or resume. Two keeps useful overlap
/// without flooding the single browser-owned API transport.
const LIBRARY_CONCURRENCY: usize = 2;

/// Explore is intentionally small. These are shelves, not another infinite
/// results list, and bounding them also bounds the number of covers a visit can
/// make the app decode.
const EXPLORE_ITEMS: usize = 10;
/// Keep the complete useful set of Home-style groups Apple returns, rather
/// than the old arbitrary first three, while retaining a defensive widget
/// bound if the remote response ever grows unexpectedly. Apple's documented
/// default-recommendations endpoint has no pagination parameter of its own.
const RECOMMENDATION_GROUPS: usize = 24;
/// Remote JSON is trusted for identity, not for size. Apple normally returns
/// kilobytes; this leaves generous room for recommendations and large pages
/// while preventing a response (or intermediary) from allocating without a
/// ceiling before serde sees it.
const API_BODY_MAX: usize = 3 * 1024 * 1024;
/// Lyrics are one TTML document, not an API collection. A separate ceiling
/// keeps a compromised or changed endpoint from consuming the general 3 MiB
/// allowance merely because both responses happen to be JSON.
const LYRICS_BODY_MAX: usize = 2 * 1024 * 1024;
/// GETs are safe to repeat, and MusicKit can briefly report a gateway failure
/// while its authenticated API client starts or resumes. This bounded window
/// covers that hand-off and ordinary CDN blips without retrying writes or
/// turning a real Apple outage into an unbounded request loop.
const GET_RETRY_DELAYS: [std::time::Duration; 4] = [
    std::time::Duration::from_millis(350),
    std::time::Duration::from_millis(900),
    std::time::Duration::from_millis(1_800),
    std::time::Duration::from_millis(3_600),
];

fn transient_gateway(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT
    )
}

async fn bounded_json<T>(res: BrowserResponse, label: &'static str) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    bounded_json_with_limit(res, label, API_BODY_MAX).await
}

async fn bounded_json_with_limit<T>(
    res: BrowserResponse,
    label: &'static str,
    limit: usize,
) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    if res.body.len() > limit {
        anyhow::bail!("Apple Music response exceeded the safe size limit");
    }
    serde_json::from_str(&res.body).with_context(|| format!("decoding {label}"))
}

struct BrowserResponse {
    status: StatusCode,
    body: String,
}

impl BrowserResponse {
    fn status(&self) -> StatusCode {
        self.status
    }
}

/// Clone is cheap: the broker is an `Arc`-backed request router.
#[derive(Clone)]
pub struct Client {
    broker: Broker,
    storefront: String,
    authorized: bool,
}

/// Failures the UI has a distinct response to. Anything else is a toast.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not signed in to Apple Music")]
    Unauthorized,
    /// 401 while MusicKit reports an authorized session. The browser owns
    /// recovery; asking the person to sign in again here would be misleading.
    #[error("Apple Music rejected the request (401) despite a valid session")]
    Rejected,
    #[error("no active Apple Music subscription")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("Apple Music error ({0})")]
    Other(StatusCode),
}

impl Client {
    pub fn new(broker: Broker, storefront: String, authorized: bool) -> Self {
        Self {
            broker,
            storefront,
            authorized,
        }
    }

    pub fn storefront(&self) -> &str {
        &self.storefront
    }

    async fn request(&self, method: ApiMethod, path: &str) -> Result<BrowserResponse, ApiError> {
        let ApiReply { status, body } = self
            .broker
            .request(method, path.to_owned())
            .await
            .map_err(Self::transport_error)?;
        let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
        Ok(BrowserResponse { status, body })
    }

    async fn get(&self, path: &str) -> Result<BrowserResponse, ApiError> {
        let mut retry = 0;
        loop {
            let response = self.request(ApiMethod::Get, path).await?;
            if !transient_gateway(response.status()) || retry == GET_RETRY_DELAYS.len() {
                return Ok(response);
            }

            let delay = GET_RETRY_DELAYS[retry];
            retry += 1;
            tracing::debug!(
                status = %response.status(),
                attempt = retry,
                "retrying a transient Apple Music GET"
            );
            tokio::time::sleep(delay).await;
        }
    }

    async fn post(&self, path: &str) -> Result<BrowserResponse, ApiError> {
        self.request(ApiMethod::Post, path).await
    }

    /// Map a response status to something the UI can act on.
    ///
    /// `signed_in` matters: a 401 while holding a live user token is not a
    /// sign-in problem, it is a rejected *request*. Telling someone to sign in
    /// again when they already are sends them in circles — which is exactly
    /// what the first version of this did.
    fn diagnose(status: StatusCode, signed_in: bool) -> ApiError {
        match status {
            StatusCode::UNAUTHORIZED if signed_in => ApiError::Rejected,
            StatusCode::UNAUTHORIZED => ApiError::Unauthorized,
            StatusCode::FORBIDDEN => ApiError::Forbidden,
            StatusCode::NOT_FOUND => ApiError::NotFound,
            other => ApiError::Other(other),
        }
    }

    fn signed_in(&self) -> bool {
        self.authorized
    }

    /// Turn a failed response into an actionable status without retaining its
    /// body. Even a first-party error response is remote input and can echo
    /// request data; it does not belong in a desktop log or toast.
    async fn explain(&self, res: BrowserResponse) -> anyhow::Error {
        let status = res.status();
        let err = Self::diagnose(status, self.signed_in());
        tracing::warn!(%status, "apple music api error");
        err.into()
    }

    /// Apple Music's own lyrics for one numeric catalog song id.
    ///
    /// This is the privacy-preferred source: the request uses the same fixed
    /// Apple API origin and credentials as the catalog and carries only the id
    /// Apple already sees for playback. TTML is parsed into bounded native
    /// lines before it leaves the API boundary. The web MusicKit surface can
    /// change, so 404/empty answers remain ordinary fallback conditions.
    pub async fn lyrics(&self, catalog_id: &str) -> Result<Option<crate::lyrics::Lyrics>> {
        if catalog_id.is_empty()
            || catalog_id.len() > 32
            || !catalog_id.bytes().all(|byte| byte.is_ascii_digit())
        {
            anyhow::bail!("invalid Apple Music catalog id");
        }

        let (lyrics_locale, script_locale) = crate::i18n::apple_lyrics_localization();
        let res = self
            .get(&format!(
                "/catalog/{}/songs/{catalog_id}/syllable-lyrics?l%5Blyrics%5D={lyrics_locale}&l%5Bscript%5D={script_locale}&extend=ttmlLocalizations",
                self.storefront
            ))
            .await
            .context("requesting Apple Music lyrics")?;

        if res.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }
        let parsed: serde_json::Value =
            bounded_json_with_limit(res, "Apple Music lyrics", LYRICS_BODY_MAX).await?;
        apple_lyrics_from_value(&parsed)
    }

    /// Apple's credits relationship for the current recording. Remote strings
    /// are flattened into bounded plain text; the UI never renders Apple HTML
    /// or exposes the raw response.
    pub async fn song_credits(&self, catalog_id: &str) -> Result<Vec<SongCredit>> {
        if catalog_id.is_empty()
            || catalog_id.len() > 32
            || !catalog_id.bytes().all(|byte| byte.is_ascii_digit())
        {
            anyhow::bail!("invalid Apple Music catalog id");
        }
        let res = self
            .get(&format!(
                "/catalog/{}/songs/{catalog_id}?include=credits",
                self.storefront
            ))
            .await
            .context("requesting song credits")?;
        if res.status() == StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }
        let value: serde_json::Value = bounded_json(res, "song credits").await?;
        Ok(parse_song_credits(&value))
    }

    /// The user's whole saved-songs library.
    ///
    /// Apple caps a page at 100 and returns a `next` cursor, so this walks
    /// until the cursor runs out. `max` bounds it so a very large library
    /// cannot spin forever on a first run — the count is reported so the UI can
    /// say the list is partial rather than quietly truncating it.
    /// Fetches in **batches of concurrent pages** rather than one at a time.
    ///
    /// The pages are independent, so serial fetching spent the whole load
    /// waiting on round trips — six sequential requests for a 539-track
    /// library. Each round now issues `LIBRARY_CONCURRENCY` requests at once and
    /// stops as soon as a round comes back short, which is the same termination
    /// condition with far fewer waits.
    pub async fn all_library_songs(&self, max: usize) -> Result<Vec<Track>> {
        // `extend=inFavorites` is what puts the star on a row: it is listed in
        // `LibrarySongs.Attributes` but omitted from the response unless asked
        // for. Measured against a real library: 41 of 541 came back starred.
        //
        // `dateAdded` is **not** obtainable this way. It is not in that
        // dictionary and `extend` does not produce it — measured as 0 of 541 —
        // which is why there is no "Recently Added" sort. If a route to it
        // turns up, that is the sort to add back.
        let mut songs = self
            .all_library::<Resource<SongAttributes>, Track>("songs", "&extend=inFavorites", max)
            .await?;
        for song in &mut songs {
            song.in_library = true;
            // Its own id is the library id, so removal has what it needs
            // without a second request.
            song.library_id = Some(song.id.0.clone());
        }

        // Kept: this is how the `dateAdded` question got settled, and it is
        // how the next one will be.
        let starred = songs.iter().filter(|s| s.favorite).count();
        tracing::info!(total = songs.len(), starred, "library attributes present");

        Ok(songs)
    }

    /// Every album in the user's library.
    pub async fn all_library_albums(&self, max: usize) -> Result<Vec<Album>> {
        let mut albums = self
            .all_library::<Resource<AlbumAttributes>, Album>("albums", "", max)
            .await?;
        // Marked here rather than guessed from the id's shape later: these ids
        // only work against `/me/library/albums`, and the page that opens one
        // has to know which endpoint to ask.
        for album in &mut albums {
            album.library = true;
        }
        // The same counter that settled `dateAdded` for songs, asking it of
        // albums. Documented on `LibraryAlbums.Attributes` — and so was the
        // songs one, which arrived 0 times out of 541. A "Recently Added" sort
        // that silently orders by nothing is worse than not offering one.
        let dated = albums.iter().filter(|a| !a.date_added.is_empty()).count();
        let with_year = albums.iter().filter(|a| !a.year.is_empty()).count();
        tracing::info!(
            total = albums.len(),
            dated,
            with_year,
            "library album attributes present"
        );
        Ok(albums)
    }

    /// Every artist in the user's library.
    ///
    /// `include=catalog` is what gets the pictures. A library artist carries
    /// only a name — no artwork, no genres; the portrait the web player shows
    /// belongs to the *catalog* artist, and asking for it as a relationship
    /// costs no extra requests. If Apple ever stops honouring it the
    /// relationship comes back absent and the grid falls back to avatar
    /// placeholders, which is no worse than not having asked.
    pub async fn all_library_artists(&self, max: usize) -> Result<Vec<Artist>> {
        // `From<LibraryArtistResource>` already marks these as library-owned.
        self.all_library::<LibraryArtistResource, Artist>("artists", "&include=catalog", max)
            .await
    }

    /// Add catalog resources to the user's library.
    ///
    /// **202 Accepted with an empty body** is success, and Apple's own wording
    /// for it is "although the modification request was acceptable, it may not
    /// have completed". So this returning `Ok` means *accepted*, not *done* —
    /// nothing may call it and then claim the item is in the library.
    pub async fn add_song_to_library(&self, id: &str) -> Result<()> {
        let id = urlencode(id);
        self.accepted(format!("/me/library?ids[songs]={id}"), "adding to library")
            .await
    }

    /// Favourite a resource — the star, not the older love/dislike rating.
    pub async fn favorite_song(&self, id: &str) -> Result<()> {
        let id = urlencode(id);
        self.accepted(format!("/me/favorites?ids[songs]={id}"), "favouriting")
            .await
    }

    /// Create a user playlist through Apple's documented Music Library write.
    /// The browser broker accepts only this typed payload and never exposes its
    /// authentication material to Rust.
    pub async fn create_playlist(
        &self,
        name: String,
        description: String,
        songs: Vec<String>,
    ) -> Result<()> {
        let ApiReply { status, body } = self
            .broker
            .create_playlist(name, description, songs)
            .await
            .map_err(Self::transport_error)?;
        let response = BrowserResponse {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            body,
        };
        if !response.status().is_success() {
            return Err(self.explain(response).await);
        }
        Ok(())
    }

    /// Append catalog songs to a library playlist. Apple publishes no matching
    /// remove/reorder endpoint, so this client deliberately exposes neither.
    pub async fn add_playlist_tracks(&self, playlist_id: String, songs: Vec<String>) -> Result<()> {
        let ApiReply { status, body } = self
            .broker
            .add_playlist_tracks(playlist_id, songs)
            .await
            .map_err(Self::transport_error)?;
        let response = BrowserResponse {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            body,
        };
        if !response.status().is_success() {
            return Err(self.explain(response).await);
        }
        Ok(())
    }

    // There is deliberately no `remove_from_favorites`.
    //
    // Apple documents "Add resource to favorites" and publishes no counterpart.
    // `DELETE /v1/me/favorites?ids[songs]=…` — the obvious REST inverse — was
    // tried against a real account and answered:
    //
    //   400 Insufficient Permissions
    //   'Favorites:DELETE:IdsQuery' entities require permissions that are not
    //   in the request
    //
    // The browser broker exposes no arbitrary DELETE. Removal stays on the
    // dedicated, typed page command that uses MusicKit's verb helper, so this
    // client cannot accidentally widen its request capability.

    /// The shared POST-and-check for both. Neither returns a body worth
    /// parsing; what matters is that Apple accepted it.
    async fn accepted(&self, path: String, what: &'static str) -> Result<()> {
        let res = self.post(&path).await.context(what)?;

        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }
        Ok(())
    }

    /// Every playlist in the user's library.
    pub async fn all_library_playlists(&self, max: usize) -> Result<Vec<Playlist>> {
        let mut playlists = self
            .all_library::<Resource<PlaylistAttributes>, Playlist>("playlists", "", max)
            .await?;
        for playlist in &mut playlists {
            playlist.library = true;
        }
        let dated = playlists
            .iter()
            .filter(|p| !p.date_added.is_empty())
            .count();
        let modified = playlists
            .iter()
            .filter(|p| !p.last_modified.is_empty())
            .count();
        // Counts answer the implementation question without writing private
        // playlist names into journald or a copied diagnostic report.
        let with_art = playlists.iter().filter(|p| p.artwork.is_some()).count();
        let without_art = playlists.len().saturating_sub(with_art);
        tracing::info!(
            total = playlists.len(),
            dated,
            modified,
            with_art,
            without_art,
            "library playlist attributes present"
        );
        Ok(playlists)
    }

    /// One library playlist, with its tracks.
    ///
    /// Two requests, unlike [`Client::library_album`]: `include=tracks` caps at
    /// 100 and playlists routinely run longer, so the tracks come from the
    /// relationship endpoint through the ordinary paginator. Silently showing
    /// the first 100 of a 400-track playlist is the kind of wrong answer that
    /// looks right.
    pub async fn library_playlist(&self, id: &str) -> Result<(Playlist, Vec<Track>)> {
        // **Overlapped.** A small details request in front of the track walk is
        // still a round trip of nothing on screen, and joining costs no tokio
        // feature `all_pages` does not already use below.
        let details = self.clone();
        let for_details = id.to_owned();
        let spawned =
            tokio::spawn(async move { details.library_playlist_details(&for_details).await });
        let id = urlencode(id);
        let tracks = self
            .all_library::<Resource<SongAttributes>, Track>(
                &format!("playlists/{id}/tracks"),
                "&extend=inFavorites",
                PLAYLIST_MAX,
            )
            .await?;
        let playlist = spawned.await.context("playlist details task panicked")??;
        Ok((playlist, tracks))
    }

    /// The first page of tracks, enough to find four covers for a grid mosaic.
    pub async fn library_playlist_preview(&self, id: &str) -> Result<Vec<Track>> {
        let id = urlencode(id);
        self.page::<Resource<SongAttributes>, Track>(
            "/me/library",
            &format!("playlists/{id}/tracks"),
            "",
            PLAYLIST_PREVIEW,
            0,
        )
        .await
    }

    async fn library_playlist_details(&self, id: &str) -> Result<Playlist> {
        let id = urlencode(id);
        let mut playlist = self
            .playlist_details(&format!("/me/library/playlists/{id}"))
            .await?;
        playlist.library = true;
        Ok(playlist)
    }

    /// One playlist resource, from either id space — the caller supplies the
    /// path and owns the `library` flag, which is the rule everywhere else too.
    async fn playlist_details(&self, path: &str) -> Result<Playlist> {
        let res = self.get(path).await.context("requesting playlist")?;

        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }

        let parsed: Response<Resource<PlaylistAttributes>> = bounded_json(res, "playlist").await?;
        let mut playlist: Playlist = parsed
            .data
            .into_iter()
            .next()
            .context("playlist not found")?
            .into();
        playlist.library = true;
        Ok(playlist)
    }

    /// Walk a `/me/library/{kind}` collection to its end, or to `max`.
    ///
    /// `kind` is interpolated straight into the path, so it can be a collection
    /// (`songs`) or a relationship (`playlists/p.123/tracks`). That is what
    /// gives playlist tracks real pagination rather than the 100 that
    /// `include=tracks` caps out at.
    async fn all_library<R, T>(
        &self,
        kind: &str,
        extra_query: &'static str,
        max: usize,
    ) -> Result<Vec<T>>
    where
        R: serde::de::DeserializeOwned + Send + 'static,
        T: From<R> + Send + 'static,
    {
        self.all_pages("/me/library", kind, extra_query, max).await
    }

    /// The paginator, over any collection that takes `limit` and `offset`.
    ///
    /// Generic over Apple's wire shape `R` and our type `T` because songs,
    /// albums, artists and playlists paginate identically — four copies of this
    /// loop would drift, and the concurrency and short-page logic below is the
    /// fiddly part worth having once.
    ///
    /// `base` is the endpoint root: `/me/library` or `/catalog/{storefront}`.
    /// It is a parameter rather than a constant because those are the two id
    /// spaces (see ARCHITECTURE.md) and a path from one 404s against the other — so
    /// the caller, which knows which kind of id it holds, picks.
    async fn all_pages<R, T>(
        &self,
        base: &str,
        kind: &str,
        extra_query: &'static str,
        max: usize,
    ) -> Result<Vec<T>>
    where
        R: serde::de::DeserializeOwned + Send + 'static,
        T: From<R> + Send + 'static,
    {
        let mut all: Vec<T> = Vec::new();
        let mut offset = 0usize;

        while all.len() < max {
            let offsets: Vec<usize> = (0..LIBRARY_CONCURRENCY)
                .map(|i| offset + i * LIBRARY_PAGE)
                .collect();

            let mut tasks = tokio::task::JoinSet::new();
            for (slot, at) in offsets.iter().copied().enumerate() {
                let client = self.clone();
                // Cloned per task: `kind` may be a borrowed path built from an
                // id, and the tasks outlive this loop iteration.
                let kind = kind.to_owned();
                let base = base.to_owned();
                tasks.spawn(async move {
                    (
                        slot,
                        client
                            .page::<R, T>(&base, &kind, extra_query, LIBRARY_PAGE, at)
                            .await,
                    )
                });
            }

            // Collect by slot so the library keeps Apple's ordering regardless
            // of which request finishes first.
            let mut pages: Vec<Option<Vec<T>>> = (0..LIBRARY_CONCURRENCY).map(|_| None).collect();
            while let Some(joined) = tasks.join_next().await {
                let (slot, page) = joined.context("library page task panicked")?;
                pages[slot] = Some(page?);
            }

            let mut round = 0usize;
            let mut short = false;
            for page in pages.into_iter().flatten() {
                round += page.len();
                if page.len() < LIBRARY_PAGE {
                    short = true;
                }
                all.extend(page);
            }

            // A short page anywhere in the round means we have reached the end.
            if short || round == 0 {
                break;
            }
            offset += round;
        }

        all.truncate(max);
        Ok(all)
    }

    async fn page<R, T>(
        &self,
        base: &str,
        kind: &str,
        extra_query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<T>>
    where
        R: serde::de::DeserializeOwned,
        T: From<R>,
    {
        let res = self
            .get(&format!(
                "{base}/{kind}?limit={limit}&offset={offset}{extra_query}"
            ))
            .await
            .context("requesting library page")?;

        // A **relationship** past its end 404s with "No related resources"
        // rather than returning an empty page, unlike a collection, which
        // returns `{"data": []}`. Since a round fires LIBRARY_CONCURRENCY
        // offsets at once, every playlist shorter than
        // LIBRARY_PAGE * LIBRARY_CONCURRENCY can have later requests come back
        // 404 and take the whole page down with them.
        //
        // Treat it as the empty page it means. The caller stops on a short
        // round anyway, so this cannot mask a real gap: offset 0 returning
        // nothing for a resource that exists is indistinguishable from a
        // resource with no tracks, and both should render as "no tracks".
        if res.status() == StatusCode::NOT_FOUND {
            tracing::debug!(offset, "library page past the end");
            return Ok(Vec::new());
        }

        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }

        let parsed: Response<R> = bounded_json(res, "library page").await?;
        Ok(parsed.data.into_iter().map(T::from).collect())
    }

    /// Catalog search. Needs only the developer token — no user token, no
    /// subscription — which makes it the cheapest way to prove the harvested
    /// token actually works before any playback is involved.
    /// `offset` walks further into the results. Apple caps `limit` at 25 per
    /// request for search, so anything past the first 25 needs paging.
    ///
    /// `types` is a comma-separated list of kinds — see `CatalogFilter::types`.
    /// It is not merely a filter: **`offset` walks one cursor shared by every
    /// kind named**, so paging is only coherent when a single kind is asked
    /// for. A key is omitted entirely from the response when its kind matched
    /// nothing, so the caller cannot tell "no results" from "not requested"
    /// without knowing what it asked for.
    pub async fn search(
        &self,
        term: &str,
        types: &str,
        limit: u32,
        offset: usize,
    ) -> Result<SearchResults> {
        if !matches!(
            types,
            "songs" | "albums" | "artists" | "playlists" | "songs,albums,artists,playlists"
        ) {
            anyhow::bail!("invalid Apple Music search resource type");
        }
        let query = urlencode(term);
        let res = self
            .get(&format!(
                "/catalog/{}/search?types={types}&limit={limit}&offset={offset}&term={query}",
                self.storefront
            ))
            .await
            .context("searching the catalog")?;

        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }

        // Search nests its payload differently to every other endpoint:
        // results -> songs -> data, and `songs` is absent (not empty) when
        // nothing matched.
        let parsed: SearchResponse = bounded_json(res, "search results").await?;
        let mut songs: Vec<Track> = parsed
            .results
            .songs
            .map(|s| s.data.into_iter().map(Track::from).collect())
            .unwrap_or_default();

        // Ask, in one extra request, which of these are already saved — so the
        // row menu can offer "Remove from Library" instead of an "Add" that
        // would do nothing.
        let ids: Vec<String> = songs.iter().filter_map(|t| t.catalog_id.clone()).collect();
        let members = self.library_membership(&ids).await;
        if !members.is_empty() {
            for song in &mut songs {
                if let Some(catalog_id) = song.catalog_id.as_deref()
                    && let Some(library_id) = members.get(catalog_id)
                {
                    song.in_library = true;
                    song.library_id = Some(library_id.clone());
                }
            }
        }
        tracing::debug!(
            songs = songs.len(),
            in_library = members.len(),
            "search membership"
        );

        Ok(SearchResults {
            songs,
            albums: parsed
                .results
                .albums
                .map(|a| a.data.into_iter().map(Album::from).collect())
                .unwrap_or_default(),
            artists: parsed
                .results
                .artists
                .map(|a| a.data.into_iter().map(Artist::from).collect())
                .unwrap_or_default(),
            // `library` stays false, which is what the parse site owes every
            // Album/Artist/Playlist: these came from `/catalog`, so a
            // `/me/library` id would 404 against them.
            playlists: parsed
                .results
                .playlists
                .map(|p| p.data.into_iter().map(Playlist::from).collect())
                .unwrap_or_default(),
        })
    }

    /// Apple's native Explore equivalent: personal recommendations and
    /// listening history from `/me`, plus storefront charts from `/catalog`.
    ///
    /// These are public Apple Music API endpoints, not a scrape of the web
    /// player's private Browse feed. The four requests are independent and run
    /// together; a missing history permission or one changed response leaves
    /// the other shelves usable. Only when every endpoint fails is Explore a
    /// failed page.
    pub async fn explore(&self) -> Result<Explore> {
        let mut jobs = tokio::task::JoinSet::new();
        for part in ExplorePart::ALL {
            let client = self.clone();
            jobs.spawn(async move {
                let result = match part {
                    ExplorePart::Recommendations => client.recommendation_sections().await,
                    ExplorePart::Recent => {
                        client
                            .mixed_section(
                                "/me/recent/played?limit=10",
                                "Recently Played",
                                "Pick up where you left off",
                            )
                            .await
                    }
                    ExplorePart::HeavyRotation => {
                        client
                            .mixed_section(
                                "/me/history/heavy-rotation?limit=10",
                                "Heavy Rotation",
                                "The music you keep coming back to",
                            )
                            .await
                    }
                    ExplorePart::Charts => client.chart_sections().await,
                };
                (part, result)
            });
        }

        let mut landed: [Option<Vec<ExploreSection>>; 4] = std::array::from_fn(|_| None);
        let mut failures = 0usize;
        while let Some(joined) = jobs.join_next().await {
            match joined {
                Ok((part, Ok(sections))) => landed[part.index()] = Some(sections),
                Ok((part, Err(err))) => {
                    // No titles, artists, recommendation reasons, or response
                    // bodies in logs. A category and the already-redacted
                    // error chain are enough to diagnose the endpoint.
                    tracing::warn!(part = part.label(), error = %err, "explore source unavailable");
                    failures += 1;
                }
                Err(err) => {
                    tracing::warn!(error = %err, "explore task failed");
                    failures += 1;
                }
            }
        }

        let sections: Vec<ExploreSection> = landed
            .into_iter()
            .flatten()
            .flatten()
            .filter(|section| !section.items.is_empty())
            .collect();
        if sections.is_empty() && failures > 0 {
            anyhow::bail!("Apple Music Explore is temporarily unavailable");
        }
        Ok(Explore { sections })
    }

    async fn recommendation_sections(&self) -> Result<Vec<ExploreSection>> {
        let res = self
            .get("/me/recommendations")
            .await
            .context("requesting recommendations")?;
        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }

        let parsed: Response<RecommendationResource> = bounded_json(res, "recommendations").await?;
        Ok(parsed
            .data
            .into_iter()
            .filter_map(|recommendation| {
                let items = recommendation
                    .relationships
                    .and_then(|relationships| relationships.contents)
                    .map(|contents| mixed_items(contents.data))
                    .unwrap_or_default();
                if items.is_empty() {
                    return None;
                }
                let title = recommendation
                    .attributes
                    .title
                    .and_then(|title| nonempty(title.string_for_display))
                    .unwrap_or_else(|| "Made for You".into());
                let subtitle = recommendation
                    .attributes
                    .reason
                    .and_then(|reason| nonempty(reason.string_for_display))
                    .unwrap_or_default();
                Some(ExploreSection {
                    title,
                    subtitle,
                    items,
                })
            })
            .take(RECOMMENDATION_GROUPS)
            .collect())
    }

    async fn mixed_section(
        &self,
        path: &str,
        title: &'static str,
        subtitle: &'static str,
    ) -> Result<Vec<ExploreSection>> {
        let res = self
            .get(path)
            .await
            .with_context(|| format!("requesting {title}"))?;
        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }
        let parsed: Response<MixedResource> = bounded_json(res, "Explore section").await?;
        let items = mixed_items(parsed.data);
        Ok((!items.is_empty())
            .then(|| ExploreSection {
                title: title.into(),
                subtitle: subtitle.into(),
                items,
            })
            .into_iter()
            .collect())
    }

    async fn chart_sections(&self) -> Result<Vec<ExploreSection>> {
        let res = self
            .get(&format!(
                "/catalog/{}/charts?types=songs,albums,playlists&chart=most-played&limit=10",
                self.storefront
            ))
            .await
            .context("requesting charts")?;
        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }
        let parsed: ChartResponse = bounded_json(res, "charts").await?;
        let mut sections = Vec::new();

        if let Some(chart) = parsed.results.albums.into_iter().next() {
            let items = chart
                .data
                .into_iter()
                .map(Album::from)
                .map(ExploreItem::Album)
                .take(EXPLORE_ITEMS)
                .collect();
            sections.push(ExploreSection {
                title: nonempty(chart.name).unwrap_or_else(|| "Top Albums".into()),
                subtitle: "Popular in your Apple Music storefront".into(),
                items,
            });
        }
        if let Some(chart) = parsed.results.playlists.into_iter().next() {
            let items = chart
                .data
                .into_iter()
                .map(Playlist::from)
                .map(ExploreItem::Playlist)
                .take(EXPLORE_ITEMS)
                .collect();
            sections.push(ExploreSection {
                title: nonempty(chart.name).unwrap_or_else(|| "Popular Playlists".into()),
                subtitle: "Editorial picks people are playing now".into(),
                items,
            });
        }
        if let Some(chart) = parsed.results.songs.into_iter().next() {
            let items = chart
                .data
                .into_iter()
                .map(catalog_track)
                .map(ExploreItem::Track)
                .take(EXPLORE_ITEMS)
                .collect();
            sections.push(ExploreSection {
                title: nonempty(chart.name).unwrap_or_else(|| "Top Songs".into()),
                subtitle: "The storefront chart, refreshed by Apple".into(),
                items,
            });
        }
        Ok(sections)
    }

    /// Which of these catalog songs are already in the library, and under what
    /// library id.
    ///
    /// A second request, because **catalog search does not honour
    /// `include=library`** — measured: the relationship comes back absent on
    /// every result, while the same include on album and playlist track
    /// relationships works. So those get membership free and search pays for it.
    ///
    /// Best-effort by design. This decides whether a menu item is offered, and
    /// a search that failed because the *decoration* failed would be a bad
    /// trade — so every error yields an empty map and the rows simply behave as
    /// they did before.
    async fn library_membership(&self, ids: &[String]) -> HashMap<String, String> {
        let mut found = HashMap::new();
        if ids.is_empty() {
            return found;
        }
        // Well under the limit Apple accepts, and a search page is 25 anyway.
        for chunk in ids.chunks(100) {
            let joined = chunk
                .iter()
                .map(|id| urlencode(id))
                .collect::<Vec<_>>()
                .join(",");
            let res = self
                .get(&format!(
                    "/catalog/{}/songs?ids={joined}&include=library",
                    self.storefront
                ))
                .await;
            let Ok(res) = res else { continue };
            if !res.status().is_success() {
                tracing::debug!(status = %res.status(), "library membership lookup failed");
                continue;
            }
            let Ok(parsed) =
                bounded_json::<Response<Resource<SongAttributes>>>(res, "library membership").await
            else {
                continue;
            };
            for resource in parsed.data {
                if let Some(library_id) = resource.library_id() {
                    found.insert(resource.id, library_id);
                }
            }
        }
        found
    }

    /// A catalog playlist and its tracks.
    ///
    /// The tracks come from the relationship endpoint rather than
    /// `include=tracks`, for the same reason library playlists do: `include`
    /// caps at 100, and Apple's editorial playlists routinely run longer.
    /// Showing the first 100 of 400 is a wrong answer that looks right.
    pub async fn playlist(&self, id: &str) -> Result<(Playlist, Vec<Track>)> {
        let id = urlencode(id);
        let playlist = self
            .playlist_details(&format!("/catalog/{}/playlists/{id}", self.storefront))
            .await?;
        let tracks = self
            .all_pages::<Resource<SongAttributes>, Track>(
                &format!("/catalog/{}", self.storefront),
                &format!("playlists/{id}/tracks"),
                // Free membership: the tracks relationship honours it, so every
                // row knows whether it is already saved without a second call.
                "&include=library",
                PLAYLIST_MAX,
            )
            .await?;
        Ok((playlist, tracks))
    }

    /// The album and artist a **catalog** song belongs to.
    ///
    /// Exists because a queue item is not a `Track`: MusicKit reports a title,
    /// an artist name and an album name, and no ids for either. So walking from
    /// a queue row to its album page needs one lookup, which is why it happens
    /// on a menu click rather than for every row.
    ///
    /// Catalog only, and that is safe here rather than lucky: a queue is loaded
    /// by `setQueue` from catalog ids, so a queue item's id is always one.
    /// Either relationship may be absent, and `None` is a real answer — a
    /// single that belongs to no album is not an error.
    pub async fn song_containers(&self, id: &str) -> Result<(Option<String>, Option<String>)> {
        let id = urlencode(id);
        let res = self
            .get(&format!(
                "/catalog/{}/songs/{id}?include=albums,artists",
                self.storefront
            ))
            .await
            .context("requesting song")?;

        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }

        let parsed: Response<SongContainers> = bounded_json(res, "song").await?;
        let song = parsed.data.into_iter().next().context("song not found")?;
        let (albums, artists) = match song.relationships {
            Some(rel) => (rel.albums, rel.artists),
            None => (None, None),
        };
        let first = |data: Option<RelationshipData>| {
            data.and_then(|d| d.data.into_iter().next()).map(|r| r.id)
        };
        Ok((first(albums), first(artists)))
    }

    /// An album and its tracks, in one request.
    ///
    /// `include=tracks` saves a round trip: the relationship comes back inside
    /// the album resource rather than needing a second call.
    pub async fn album(&self, id: &str) -> Result<(Album, Vec<Track>)> {
        let id = urlencode(id);
        let resource = self
            .album_resource(&format!(
                "/catalog/{}/albums/{id}?include=tracks,library",
                self.storefront
            ))
            .await?;
        let tracks = album_tracks(&resource);
        Ok((Album::from(resource.into_album()), tracks))
    }

    /// Fetch one album resource, whichever collection it lives in. The catalog
    /// and library responses have the same shape; only the URL differs.
    async fn album_resource(&self, path: &str) -> Result<AlbumResource> {
        let res = self.get(path).await.context("requesting album")?;

        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }

        let parsed: Response<AlbumResource> = bounded_json(res, "album").await?;
        parsed.data.into_iter().next().context("album not found")
    }

    /// One album from the user's library, with the tracks they saved.
    ///
    /// Distinct from [`Client::album`] only in the URL: library ids 404 against
    /// `/catalog`. The response shape is identical, so the parsing is shared.
    pub async fn library_album(&self, id: &str) -> Result<(Album, Vec<Track>)> {
        let id = urlencode(id);
        let resource = self
            .album_resource(&format!("/me/library/albums/{id}?include=tracks"))
            .await?;
        let mut tracks = album_tracks(&resource);
        for track in &mut tracks {
            track.in_library = true;
            track.library_id = Some(track.id.0.clone());
        }
        let mut album = Album::from(resource.into_album());
        album.library = true;
        Ok((album, tracks))
    }

    /// One artist from the user's library, with the albums they saved.
    pub async fn library_artist_albums(&self, id: &str) -> Result<ArtistPageData> {
        let id = urlencode(id);
        // `catalog` alongside `albums`: the portrait belongs to the catalog
        // artist, exactly as in `all_library_artists`, so the page header
        // matches the tile that opened it.
        let resource = self
            .artist_resource(&format!(
                "/me/library/artists/{id}?include=albums,catalog&extend=editorialNotes"
            ))
            .await?;
        let portrait = resource
            .relationships
            .as_ref()
            .and_then(|r| r.catalog.as_ref())
            .and_then(|c| c.data.first())
            .cloned()
            .map(Artist::from);
        let catalog_id = resource
            .relationships
            .as_ref()
            .and_then(|r| r.catalog.as_ref())
            .and_then(|c| c.data.first())
            .map(|catalog| catalog.id.clone());

        let mut albums = artist_albums_of(&resource);

        // `include=` is documented for catalog resources and merely *works* for
        // library ones. If Apple ever stops honouring it the relationship comes
        // back absent, which would render as an artist who owns no albums —
        // a silent wrong answer. Ask the relationship endpoint directly instead.
        if albums.is_empty() {
            tracing::debug!("library artist had no included albums; asking directly");
            albums = self
                .library_pageless_albums(&format!("/me/library/artists/{id}/albums"))
                .await
                .unwrap_or_default();
        }

        for album in &mut albums {
            album.library = true;
        }

        let mut artist = Artist::from(resource.into_artist());
        artist.library = true;
        if let Some(portrait) = portrait {
            artist.artwork = portrait.artwork;
            artist.genres = portrait.genres;
            artist.biography = portrait.biography;
        }

        // Apple may omit extended attributes from an included catalog
        // relationship even when the top-level library request asked for
        // them. Fetch only the missing biography from the same catalog artist
        // as a best-effort fallback; the library page itself remains usable if
        // this optional request fails.
        if artist.biography.is_empty()
            && let Some(id) = catalog_id.as_deref()
        {
            match self
                .artist_resource(&catalog_artist_path(
                    &self.storefront,
                    &urlencode(id),
                    false,
                ))
                .await
            {
                Ok(resource) => {
                    artist.biography = Artist::from(resource.into_artist()).biography;
                }
                Err(_) => tracing::debug!("catalog artist biography was unavailable"),
            }
        }
        if let Some(id) = catalog_id.as_deref() {
            self.fill_missing_biography(&mut artist, &urlencode(id))
                .await;
        }
        let (top_songs, latest_release) = match catalog_id {
            Some(id) => self.artist_extras(&urlencode(&id)).await,
            None => (Vec::new(), albums.first().cloned()),
        };
        let latest_release = latest_release.or_else(|| albums.first().cloned());
        Ok(ArtistPageData {
            artist,
            top_songs,
            latest_release,
            albums,
        })
    }

    /// A one-shot relationship fetch — no paging. Used only as the fallback
    /// above, where a first page of albums is better than none.
    async fn library_pageless_albums(&self, path: &str) -> Result<Vec<Album>> {
        let res = self
            .get(path)
            .await
            .context("requesting library artist albums")?;

        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }

        let parsed: Response<Resource<AlbumAttributes>> =
            bounded_json(res, "library artist albums").await?;
        Ok(parsed.data.into_iter().map(Album::from).collect())
    }

    /// An artist's albums, newest first as Apple orders them.
    pub async fn artist_albums(&self, id: &str) -> Result<ArtistPageData> {
        let id = urlencode(id);
        let resource = self
            .artist_resource(&catalog_artist_path(&self.storefront, &id, true))
            .await?;
        let albums = artist_albums_of(&resource);
        let (top_songs, latest_release) = self.artist_extras(&id).await;
        let mut artist = Artist::from(resource.into_artist());
        self.fill_missing_biography(&mut artist, &id).await;
        Ok(ArtistPageData {
            artist,
            top_songs,
            latest_release: latest_release.or_else(|| albums.first().cloned()),
            albums,
        })
    }

    /// Fill editorial text without changing the page's active storefront.
    ///
    /// Apple does not localize every biography, and MusicKit can omit the
    /// field even when Apple's public artist page has it. Try the US catalog
    /// through the existing broker first, then make one credential-free,
    /// bounded request to Apple's public US page if the field is still empty.
    async fn fill_missing_biography(&self, artist: &mut Artist, encoded_id: &str) {
        if !artist.biography.is_empty() {
            return;
        }

        if !self.storefront.eq_ignore_ascii_case("us") {
            match self
                .artist_resource(&catalog_artist_path("us", encoded_id, false))
                .await
            {
                Ok(resource) => {
                    let english = Artist::from(resource.into_artist()).biography;
                    if !english.is_empty() {
                        artist.biography = english;
                    }
                }
                Err(_) => tracing::debug!("English catalog biography was unavailable"),
            }
        }
        if artist.biography.is_empty() {
            match super::biography::fetch_english(encoded_id).await {
                Ok(Some(english)) => artist.biography = english,
                Ok(None) => {}
                Err(_) => tracing::debug!("public Apple artist biography was unavailable"),
            }
        }
    }

    async fn artist_extras(&self, encoded_id: &str) -> (Vec<Track>, Option<Album>) {
        let songs = self.artist_top_songs(encoded_id).await.unwrap_or_else(|_| {
            tracing::warn!("artist top songs were unavailable");
            Vec::new()
        });
        let latest = self
            .artist_latest_release(encoded_id)
            .await
            .unwrap_or_else(|_| {
                tracing::warn!("artist latest release was unavailable");
                None
            });
        (songs, latest)
    }

    /// Apple's public top-songs view for one catalog artist.
    async fn artist_top_songs(&self, encoded_id: &str) -> Result<Vec<Track>> {
        let res = self
            .get(&format!(
                "/catalog/{}/artists/{encoded_id}/view/top-songs?limit=10",
                self.storefront
            ))
            .await
            .context("requesting artist top songs")?;

        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }
        let parsed: Response<Resource<SongAttributes>> =
            bounded_json(res, "artist top songs").await?;
        Ok(parsed.data.into_iter().map(Track::from).collect())
    }

    /// Apple's public latest-release view for one catalog artist.
    async fn artist_latest_release(&self, encoded_id: &str) -> Result<Option<Album>> {
        let res = self
            .get(&format!(
                "/catalog/{}/artists/{encoded_id}/view/latest-release?limit=1",
                self.storefront
            ))
            .await
            .context("requesting artist latest release")?;

        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }
        let parsed: Response<Resource<AlbumAttributes>> =
            bounded_json(res, "artist latest release").await?;
        Ok(parsed.data.into_iter().next().map(Album::from))
    }

    /// As [`Client::album_resource`], for artists.
    async fn artist_resource(&self, path: &str) -> Result<ArtistResource> {
        let res = self.get(path).await.context("requesting artist")?;

        if !res.status().is_success() {
            return Err(self.explain(res).await);
        }

        let parsed: Response<ArtistResource> = bounded_json(res, "artist").await?;
        parsed.data.into_iter().next().context("artist not found")
    }

    fn transport_error(err: BrokerError) -> ApiError {
        match err {
            BrokerError::InvalidRequest => ApiError::Other(StatusCode::BAD_REQUEST),
            BrokerError::Busy => ApiError::Other(StatusCode::TOO_MANY_REQUESTS),
            BrokerError::Unavailable => ApiError::Other(StatusCode::SERVICE_UNAVAILABLE),
            BrokerError::Timeout => ApiError::Other(StatusCode::GATEWAY_TIMEOUT),
        }
    }
}

fn bounded_credit_text(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| text.chars().take(160).collect())
}

fn credit_names(resource: &serde_json::Value) -> Vec<String> {
    let mut names = Vec::new();
    let mut add = |value: &serde_json::Value| {
        if let Some(name) = bounded_credit_text(value)
            && !names.contains(&name)
            && names.len() < 32
        {
            names.push(name);
        }
    };
    if let Some(attributes) = resource.get("attributes") {
        for key in ["names", "artists", "contributors"] {
            if let Some(values) = attributes.get(key).and_then(serde_json::Value::as_array) {
                for value in values {
                    if let Some(name) = value.get("name") {
                        add(name);
                    } else {
                        add(value);
                    }
                }
            }
        }
        if let Some(name) = attributes.get("name") {
            add(name);
        }
    }
    if let Some(relationships) = resource
        .get("relationships")
        .and_then(serde_json::Value::as_object)
    {
        for relationship in relationships.values() {
            if let Some(items) = relationship
                .get("data")
                .and_then(serde_json::Value::as_array)
            {
                for item in items {
                    if let Some(name) = item.get("attributes").and_then(|value| value.get("name")) {
                        add(name);
                    }
                }
            }
        }
    }
    names
}

fn parse_song_credits(value: &serde_json::Value) -> Vec<SongCredit> {
    let Some(data) = value.get("data").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    // Apple returns the role categories under the song's included `credits`
    // relationship. Accepting a direct array as well keeps the parser useful
    // for a bounded fixture or a future relationship-only response.
    let resources = data
        .first()
        .and_then(|song| song.get("relationships"))
        .and_then(|relationships| relationships.get("credits"))
        .and_then(|credits| credits.get("data"))
        .and_then(serde_json::Value::as_array)
        .unwrap_or(data);
    let mut groups = Vec::new();
    for resource in resources.iter().take(64) {
        let attributes = resource.get("attributes");
        let role = attributes
            .and_then(|attributes| {
                ["title", "roleName", "name"]
                    .into_iter()
                    .find_map(|key| attributes.get(key).and_then(bounded_credit_text))
                    .or_else(|| {
                        attributes
                            .get("roleNames")
                            .and_then(serde_json::Value::as_array)
                            .and_then(|roles| roles.first())
                            .and_then(bounded_credit_text)
                    })
            })
            .unwrap_or_else(|| "Credits".to_owned());
        let names = credit_names(resource);
        if !names.is_empty() {
            groups.push(SongCredit { role, names });
        }
    }
    groups
}

#[derive(Debug, Clone, Copy)]
enum ExplorePart {
    Recommendations,
    Recent,
    HeavyRotation,
    Charts,
}

impl ExplorePart {
    const ALL: [Self; 4] = [
        Self::Recommendations,
        Self::Recent,
        Self::HeavyRotation,
        Self::Charts,
    ];

    fn index(self) -> usize {
        match self {
            Self::Recommendations => 0,
            Self::Recent => 1,
            Self::HeavyRotation => 2,
            Self::Charts => 3,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Recommendations => "recommendations",
            Self::Recent => "recent",
            Self::HeavyRotation => "heavy-rotation",
            Self::Charts => "charts",
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct RecommendationResource {
    #[serde(default)]
    attributes: RecommendationAttributes,
    relationships: Option<RecommendationRelationships>,
}

#[derive(Debug, Default, Deserialize)]
struct RecommendationAttributes {
    title: Option<LocalizedText>,
    reason: Option<LocalizedText>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalizedText {
    #[serde(default)]
    string_for_display: String,
}

#[derive(Debug, Deserialize)]
struct RecommendationRelationships {
    contents: Option<MixedRelationship>,
}

#[derive(Debug, Deserialize)]
struct MixedRelationship {
    #[serde(default)]
    data: Vec<MixedResource>,
}

/// Apple's generic `Resource` from recommendation/history responses. Its
/// attributes depend on `type`, so the tag is inspected exactly once here and
/// converted immediately into one of Jamelade's own types.
#[derive(Debug, Deserialize)]
struct MixedResource {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    attributes: Option<serde_json::Value>,
}

impl MixedResource {
    fn into_item(self) -> Option<ExploreItem> {
        let attributes = self.attributes?;
        match self.kind.as_str() {
            "albums" => serde_json::from_value::<AlbumAttributes>(attributes)
                .ok()
                .map(|attributes| Resource {
                    id: self.id,
                    attributes: Some(attributes),
                    relationships: None,
                })
                .map(Album::from)
                .map(ExploreItem::Album),
            "playlists" => serde_json::from_value::<PlaylistAttributes>(attributes)
                .ok()
                .map(|attributes| Resource {
                    id: self.id,
                    attributes: Some(attributes),
                    relationships: None,
                })
                .map(Playlist::from)
                .map(ExploreItem::Playlist),
            "songs" => serde_json::from_value::<SongAttributes>(attributes)
                .ok()
                .map(|attributes| Resource {
                    id: self.id,
                    attributes: Some(attributes),
                    relationships: None,
                })
                .map(catalog_track)
                .map(ExploreItem::Track),
            "stations" => serde_json::from_value::<StationAttributes>(attributes)
                .ok()
                .map(|attributes| Resource {
                    id: self.id,
                    attributes: Some(attributes),
                    relationships: None,
                })
                .map(Station::from)
                .filter(|station| {
                    !station.id.is_empty()
                        && station.id.len() <= 512
                        && !station.name.trim().is_empty()
                })
                .map(ExploreItem::Station),
            // Music videos need a visible video surface Jamelade does not have.
            // Dropping them is safer than drawing dead cards.
            _ => None,
        }
    }
}

fn mixed_items(resources: Vec<MixedResource>) -> Vec<ExploreItem> {
    resources
        .into_iter()
        .filter_map(MixedResource::into_item)
        .take(EXPLORE_ITEMS)
        .collect()
}

/// A catalog song is playable by its resource id even if this particular
/// response omitted `playParams`.
fn catalog_track(resource: Resource<SongAttributes>) -> Track {
    let fallback = resource.id.clone();
    let mut track = Track::from(resource);
    if track.catalog_id.is_none() {
        track.catalog_id = Some(fallback);
    }
    track
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

#[derive(Debug, Default, Deserialize)]
struct ChartResponse {
    #[serde(default)]
    results: ChartResults,
}

#[derive(Debug, Default, Deserialize)]
struct ChartResults {
    #[serde(default)]
    albums: Vec<Chart<AlbumAttributes>>,
    #[serde(default)]
    playlists: Vec<Chart<PlaylistAttributes>>,
    #[serde(default)]
    songs: Vec<Chart<SongAttributes>>,
}

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "A: Deserialize<'de>"))]
struct Chart<A> {
    #[serde(default)]
    name: String,
    #[serde(default)]
    data: Vec<Resource<A>>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: SearchWire,
}

/// Apple omits a key entirely when a kind matched nothing, rather than sending
/// an empty array — so every field here is optional.
#[derive(Debug, Default, Deserialize)]
struct SearchWire {
    songs: Option<Response<Resource<SongAttributes>>>,
    albums: Option<Response<Resource<AlbumAttributes>>>,
    artists: Option<Response<Resource<ArtistAttributes>>>,
    playlists: Option<Response<Resource<PlaylistAttributes>>>,
}

/// Percent-encode a search term. The full `url` crate is a lot of dependency
/// for one query parameter; this covers everything not unreserved per RFC 3986.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// A catalog artist request with Apple's documented editorial-notes extension.
/// Keeping construction in one place prevents a page from silently regressing
/// to the biography-free default response.
fn catalog_artist_path(storefront: &str, encoded_id: &str, include_albums: bool) -> String {
    let include = if include_albums {
        "include=albums&"
    } else {
        ""
    };
    format!("/catalog/{storefront}/artists/{encoded_id}?{include}extend=editorialNotes")
}

/// The tracks Apple attached to an album via `include=tracks`, or none.
fn album_tracks(resource: &AlbumResource) -> Vec<Track> {
    resource
        .relationships
        .as_ref()
        .and_then(|r| r.tracks.as_ref())
        .map(|t| t.data.iter().cloned().map(Track::from).collect())
        .unwrap_or_default()
}

/// The albums Apple attached to an artist via `include=albums`, or none.
fn artist_albums_of(resource: &ArtistResource) -> Vec<Album> {
    resource
        .relationships
        .as_ref()
        .and_then(|r| r.albums.as_ref())
        .map(|a| a.data.iter().cloned().map(Album::from).collect())
        .unwrap_or_default()
}

/// Decode the normal TTML and any first-party translation/pronunciation TTML
/// Apple embedded beside it. The public API has changed the nesting of these
/// optional values between clients, so alternatives are discovered only below
/// semantically named keys rather than by accepting every remote XML string.
fn apple_lyrics_from_value(value: &serde_json::Value) -> Result<Option<crate::lyrics::Lyrics>> {
    let Some(ttml) = value
        .pointer("/data/0/attributes/ttmlLocalizations")
        .and_then(serde_json::Value::as_str)
        .filter(|ttml| !ttml.trim().is_empty())
        .or_else(|| {
            value
                .pointer("/data/0/attributes/ttml")
                .and_then(serde_json::Value::as_str)
                .filter(|ttml| !ttml.trim().is_empty())
        })
    else {
        return Ok(None);
    };
    let mut original = crate::lyrics::lyrics_from_apple_ttml(ttml)?;
    let mut variants = std::mem::take(&mut original.variants);
    collect_apple_lyric_variants(value, None, None, &original.lines, &mut variants, 0);
    original.variants = variants;
    Ok(Some(original))
}

fn variant_kind(key: &str) -> Option<crate::lyrics::LyricVariantKind> {
    let key = key.to_ascii_lowercase();
    if key.contains("translation") {
        Some(crate::lyrics::LyricVariantKind::Translation)
    } else if key.contains("transliteration")
        || key.contains("romanization")
        || key.contains("romanisation")
        || key.contains("pronunciation")
    {
        Some(crate::lyrics::LyricVariantKind::Romanization)
    } else {
        None
    }
}

fn variant_label(
    object: &serde_json::Map<String, serde_json::Value>,
    kind: crate::lyrics::LyricVariantKind,
) -> String {
    let detail = ["displayName", "languageName", "language", "locale", "name"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(serde_json::Value::as_str))
        .map(|value| {
            value
                .trim()
                .chars()
                .filter(|ch| !ch.is_control())
                .take(48)
                .collect::<String>()
        })
        .filter(|value| !value.is_empty());
    let base = match kind {
        crate::lyrics::LyricVariantKind::Translation => "Translation",
        crate::lyrics::LyricVariantKind::Romanization => "Romanized",
    };
    detail
        .map(|detail| format!("{base} · {detail}"))
        .unwrap_or_else(|| base.to_owned())
}

fn collect_apple_lyric_variants(
    value: &serde_json::Value,
    inherited_kind: Option<crate::lyrics::LyricVariantKind>,
    inherited_label: Option<&str>,
    original: &[crate::lyrics::Line],
    out: &mut Vec<crate::lyrics::LyricVariant>,
    depth: usize,
) {
    const MAX_VARIANTS: usize = 4;
    const MAX_VARIANT_DEPTH: usize = 16;
    if out.len() >= MAX_VARIANTS || depth > MAX_VARIANT_DEPTH {
        return;
    }
    match value {
        serde_json::Value::Object(object) => {
            let label = inherited_kind.map(|kind| variant_label(object, kind));
            for (key, child) in object {
                let kind = variant_kind(key).or(inherited_kind);
                let child_label = label.as_deref().or(inherited_label);
                if key.eq_ignore_ascii_case("ttml")
                    && let (Some(kind), Some(raw)) = (kind, child.as_str())
                    && let Ok(parsed) = crate::lyrics::lyrics_from_apple_ttml(raw)
                    && !parsed.lines.is_empty()
                    && parsed.lines != original
                    && !out.iter().any(|existing| existing.lines == parsed.lines)
                {
                    out.push(crate::lyrics::LyricVariant {
                        kind,
                        label: child_label
                            .map(str::to_owned)
                            .unwrap_or_else(|| variant_label(object, kind)),
                        lines: parsed.lines,
                        synced: parsed.synced,
                    });
                } else {
                    collect_apple_lyric_variants(
                        child,
                        kind,
                        child_label,
                        original,
                        out,
                        depth + 1,
                    );
                }
                if out.len() >= MAX_VARIANTS {
                    break;
                }
            }
        }
        serde_json::Value::Array(values) => {
            for child in values.iter().take(MAX_VARIANTS * 2) {
                collect_apple_lyric_variants(
                    child,
                    inherited_kind,
                    inherited_label,
                    original,
                    out,
                    depth + 1,
                );
                if out.len() >= MAX_VARIANTS {
                    break;
                }
            }
        }
        // A future API might put the TTML directly under `translation` rather
        // than inside an object. Keep the same semantic-key requirement.
        serde_json::Value::String(raw) if inherited_kind.is_some() => {
            if let Ok(parsed) = crate::lyrics::lyrics_from_apple_ttml(raw)
                && !parsed.lines.is_empty()
                && parsed.lines != original
                && !out.iter().any(|existing| existing.lines == parsed.lines)
            {
                let kind = inherited_kind.expect("guarded above");
                out.push(crate::lyrics::LyricVariant {
                    kind,
                    label: inherited_label
                        .map(str::to_owned)
                        .unwrap_or_else(|| match kind {
                            crate::lyrics::LyricVariantKind::Translation => "Translation".into(),
                            crate::lyrics::LyricVariantKind::Romanization => "Romanized".into(),
                        }),
                    lines: parsed.lines,
                    synced: parsed.synced,
                });
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_results_are_nested_differently_to_every_other_endpoint() {
        let raw = r#"{"results":{"songs":{"data":[
            {"id":"1440857781","attributes":{"name":"Roundabout","artistName":"Yes"}}]}}}"#;
        let parsed: SearchResponse = serde_json::from_str(raw).unwrap();
        let songs = parsed.results.songs.expect("songs present");
        assert_eq!(songs.data[0].id, "1440857781");
    }

    #[test]
    fn apple_supplied_translation_and_pronunciation_are_preserved() {
        let raw = serde_json::json!({
            "data": [{"attributes": {
                "ttmlLocalizations": null,
                "ttml": "<tt><body><p begin='1s'>Original</p></body></tt>",
                "translations": [{
                    "languageName": "English",
                    "ttml": "<tt><body><p begin='1s'>Translated</p></body></tt>"
                }],
                "pronunciations": [{
                    "locale": "ja-Latn",
                    "ttml": "<tt><body><p begin='1s'>Romanized</p></body></tt>"
                }]
            }}]
        });
        let lyrics = apple_lyrics_from_value(&raw).unwrap().unwrap();
        assert_eq!(lyrics.lines[0].text, "Original");
        assert_eq!(lyrics.variants.len(), 2);
        assert!(
            lyrics
                .variants
                .iter()
                .any(|variant| variant.label.contains("English"))
        );
        assert!(
            lyrics
                .variants
                .iter()
                .any(|variant| variant.label.contains("ja-Latn"))
        );
    }

    #[test]
    fn a_search_that_matches_nothing_omits_the_key_entirely() {
        // Apple drops `songs` rather than returning an empty array — treating
        // that as an error would make every no-results search look broken.
        let parsed: SearchResponse = serde_json::from_str(r#"{"results":{}}"#).unwrap();
        assert!(parsed.results.songs.is_none());
    }

    #[test]
    fn search_terms_are_percent_encoded() {
        assert_eq!(urlencode("Sigur Rós & co"), "Sigur%20R%C3%B3s%20%26%20co");
        assert_eq!(urlencode("plain-term_1.0~x"), "plain-term_1.0~x");
        assert_eq!(
            urlencode("../songs?include=library&token=x"),
            "..%2Fsongs%3Finclude%3Dlibrary%26token%3Dx"
        );
    }

    #[test]
    fn artist_page_requests_the_editorial_notes_extension() {
        assert_eq!(
            catalog_artist_path("de", "1234567890", true),
            "/catalog/de/artists/1234567890?include=albums&extend=editorialNotes"
        );
        assert_eq!(
            catalog_artist_path("de", "id%2Fwith%3Fpunctuation", false),
            "/catalog/de/artists/id%2Fwith%3Fpunctuation?extend=editorialNotes"
        );
        assert_eq!(
            catalog_artist_path("us", "1234567890", false),
            "/catalog/us/artists/1234567890?extend=editorialNotes"
        );
    }

    #[test]
    fn statuses_map_to_errors_that_name_the_fix() {
        assert!(matches!(
            Client::diagnose(StatusCode::UNAUTHORIZED, false),
            ApiError::Unauthorized
        ));
        assert!(
            Client::diagnose(StatusCode::FORBIDDEN, true)
                .to_string()
                .contains("subscription")
        );
    }

    #[test]
    fn only_idempotent_gateway_failures_are_retryable() {
        for status in [
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert!(transient_gateway(status));
        }
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert!(!transient_gateway(status));
        }
    }

    #[test]
    fn a_401_while_signed_in_does_not_tell_you_to_sign_in() {
        // The original bug report: signed in with a live user token, and the
        // app said "sign in again" — sending you round in a circle when the
        // real cause was a rejected request.
        let err = Client::diagnose(StatusCode::UNAUTHORIZED, true);
        assert!(matches!(err, ApiError::Rejected));
        let msg = err.to_string();
        assert!(!msg.contains("sign in"), "misleading message: {msg}");
        assert!(msg.contains("valid session"));
    }

    #[test]
    fn recommendation_resources_are_tagged_before_attributes_are_parsed() {
        let raw = r#"{"data":[{"type":"personal-recommendation","attributes":{
            "title":{"stringForDisplay":"Made for You"},
            "reason":{"stringForDisplay":"Because you listened to Yes"}},
            "relationships":{"contents":{"data":[
                {"id":"1","type":"albums","attributes":{
                    "name":"Fragile","artistName":"Yes"}},
                {"id":"radio","type":"stations","attributes":{"name":"Yes Station"}}
            ]}}}]}"#;
        let parsed: Response<RecommendationResource> = serde_json::from_str(raw).unwrap();
        let recommendation = parsed.data.into_iter().next().unwrap();
        assert_eq!(
            recommendation.attributes.title.unwrap().string_for_display,
            "Made for You"
        );
        let items = mixed_items(recommendation.relationships.unwrap().contents.unwrap().data);
        assert_eq!(
            items.len(),
            2,
            "a Home station must survive as something MusicKit can play"
        );
        assert!(matches!(&items[0], ExploreItem::Album(album) if album.name == "Fragile"));
        assert!(matches!(&items[1], ExploreItem::Station(station)
            if station.id == "radio" && station.name == "Yes Station"));

        let oversized = MixedResource {
            id: "x".repeat(513),
            kind: "stations".into(),
            attributes: Some(serde_json::json!({ "name": "Too Large" })),
        };
        let nameless = MixedResource {
            id: "radio".into(),
            kind: "stations".into(),
            attributes: Some(serde_json::json!({})),
        };
        assert!(oversized.into_item().is_none());
        assert!(nameless.into_item().is_none());
    }

    #[test]
    fn chart_response_keeps_each_resource_kind_separate() {
        let raw = r#"{"results":{
            "albums":[{"name":"Top Albums","data":[{"id":"a","attributes":{
                "name":"Blue","artistName":"Joni Mitchell"}}]}],
            "playlists":[{"name":"Top Playlists","data":[]}],
            "songs":[{"name":"Top Songs","data":[{"id":"s","attributes":{
                "name":"A Case of You","artistName":"Joni Mitchell"}}]}]
        }}"#;
        let parsed: ChartResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.results.albums[0].name, "Top Albums");
        let song = catalog_track(
            parsed
                .results
                .songs
                .into_iter()
                .next()
                .unwrap()
                .data
                .remove(0),
        );
        assert_eq!(song.catalog_id.as_deref(), Some("s"));
    }

    #[test]
    fn explore_labels_drop_only_empty_strings() {
        assert_eq!(nonempty("  ".into()), None);
        assert_eq!(nonempty("Top Songs".into()).as_deref(), Some("Top Songs"));
    }

    #[test]
    fn credits_are_flattened_to_bounded_plain_roles_and_names() {
        let response = serde_json::json!({
            "data": [{
                "type": "songs",
                "id": "1000000001",
                "attributes": { "name": "Synthetic song" },
                "relationships": { "credits": { "data": [{
                    "type": "role-categories",
                    "attributes": { "title": "Songwriting" },
                    "relationships": { "credit-artists": { "data": [
                        { "attributes": { "name": "Example Person" } }
                    ]}}
                }]}}
            }]
        });
        assert_eq!(
            parse_song_credits(&response),
            vec![SongCredit {
                role: "Songwriting".into(),
                names: vec!["Example Person".into()],
            }]
        );
    }
}
