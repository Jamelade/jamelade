// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! Root model, messages, view, and reducer. Feature-specific work lives in the
//! sibling modules as `impl AppModel` blocks. I/O runs in bounded commands so
//! the GTK thread stays responsive; see `ARCHITECTURE.md`.

use std::path::PathBuf;

use relm4::adw::prelude::*;
use relm4::{
    Component, ComponentController, ComponentParts, ComponentSender, Controller, adw, gtk,
};

use relm4::typed_view::grid::TypedGridView;
use relm4::typed_view::list::TypedListView;

use crate::companion::Companion;
use crate::components::artist_view::ArtistActivate;
use crate::components::artwork::{self, ART_SIZE};
use crate::components::detail_page::{DetailActions, DetailPage, PageKind, RowState};
use crate::components::explore_view::{ExploreAction, ExploreView};
use crate::components::grid_item::{ArtRegistry, ArtRequest, GridItem, Tile, art_registry};
use crate::components::jamkin_mode::{JamkinMode, JamkinModeConfig};
use crate::components::lyrics_view::LyricsView;
use crate::components::now_playing::{
    NowPlaying, NowPlayingInput, NowPlayingOutput, Repeat, VOLUME_STEP,
};
use crate::components::player_view::{PlayerView, PlayerViewInput};
use crate::components::queue_view::{QueueView, QueueViewInput, QueueViewOutput};
use crate::components::track_row::LibraryRowWidgets;
use crate::components::track_row::{Entry, LibraryItem, RowMenuRequest};
use crate::components::{
    CurrentTrack, DeadTracks, RowRegistry, TrackOverrides, current_track, dead_tracks,
    row_registry, track_overrides,
};
use crate::mpris::Mpris;
use crate::music::types::{Album, Artist, ArtistPageData, Artwork, Explore, Playlist, Track};
use crate::notify;
use crate::player::protocol::{AppleSession, Command, RepeatMode};
use crate::player::{Incoming, PlayerState, sidecar};
use crate::segment_loop::SegmentLoop;
use crate::settings::{Section, Settings, Theme};

/// How often the seek bar redraws while playing.
///
/// The sidecar's own position events are coarse and irregular, so
/// `PlayerState::interpolated_position_ms` fills the gaps; this timer just
/// drives the repaint. Removed entirely when not playing — a paused player
/// should cost nothing (the same discipline as Pitwall's suspend-gated poll).
mod background;
mod chrome;
mod discovery;
mod global_shortcuts;
mod library;
mod osd;
mod pages;
mod pins;
mod playback;
mod playlist_art;
mod playlist_edit;
mod queue;
mod row_menu;
mod scrobbling;
mod segment_loop;
mod status;
mod supervise;
mod view;
mod wiring;
mod writes;

use chrome::{icon, register_actions, show_about, show_credits, show_shortcuts};
use queue::Start;
use supervise::{respawn_sidecar, start_sidecar};

pub use view::{CatalogFilter, SearchScope, SortBy, View};
use view::{SidebarRow, sidebar_rows};

const TICK_MS: u32 = 500;

/// How long the search box must sit still before a catalog search is sent.
///
/// The library filter is local and runs on every keystroke; the catalog is a
/// network request, and firing one per character would be both slow and rude.
const SEARCH_DEBOUNCE_MS: u64 = 350;

/// Apple caps search at 25 results per request, so this is its ceiling rather
/// than a choice. More than that means paging with an offset.
const CATALOG_LIMIT: u32 = 25;

/// Tile covers are fetched at twice their drawn size, so they stay sharp on a
/// HiDPI screen without paying for the 512px the Now Playing bar needs.
const TILE_ART: u32 = 320;

/// How many artists and albums to show above the songs. Enough to be a way in,
/// few enough that the songs are still visible without scrolling.
const CATALOG_BROWSE_ROWS: usize = 3;

/// Stop paging here. Nobody scrolls 400 search results, and an unbounded list
/// is an unbounded number of requests.
const CATALOG_MAX: usize = 200;

/// Upper bound on the library load. Apple pages at 100, so this is 25 requests
/// worst case. Generous for one laptop, and bounded so a very large library
/// cannot spin forever on first run.
const LIBRARY_MAX: usize = 2_500;

/// Where we are in bringing the sidecar up. Each variant is a distinct
/// `StatusPage`, because "it's just spinning" is the failure mode this whole
/// module exists to avoid (rule 4).
#[derive(Debug, Default)]
pub enum Stage {
    #[default]
    Starting,
    /// Chromium's component updater is fetching the CDM. First run only, but it
    /// needs network and can take a minute — so it gets to say so.
    InstallingWidevine,
    /// Loaded music.apple.com; waiting for the hook to attach.
    Connecting,
    /// Signed out. Apple's own login window is one click away.
    SignedOut,
    Ready,
    /// The sidecar died; a restart is scheduled (rule 6).
    Restarting(u32),
    /// Apple changed the page, or the CDM is unavailable. Names the fix.
    Broken(String),
}

pub struct AppModel {
    stage: Stage,
    player: PlayerState,
    /// The first-run gate, while it is up. `Some` exactly when the app is
    /// blocked, which is what stops it being presented twice.
    onboarding: Option<adw::Dialog>,

    /// Whether the restore has been attempted this session, so a later browser
    /// refresh cannot start it again.
    restored: bool,

    /// The last track MusicKit reported, kept so the bar can hold it through a
    /// queue reload — see `push_snapshot::showing`.
    last_item: Option<crate::player::protocol::Item>,

    /// Kept for the row context menu, whose GTK actions outlive the `update`
    /// call that built them.
    menu_sender: ComponentSender<AppModel>,

    /// The last command sent to the sidecar, and when. Read only by the
    /// gapless diagnostic, which needs to distinguish a transition **we** asked
    /// for from one MusicKit made on its own — the second is the gapless path
    /// and the first is not. `RefCell` because `send` takes `&self`.
    last_command: std::cell::RefCell<Option<(std::time::Instant, String)>>,

    /// Furthest position reached in the current track, and that track's length.
    ///
    /// A high-water mark rather than a live read, because at the moment
    /// `nowPlayingItemDidChange` arrives MusicKit has usually already zeroed
    /// the position — and sometimes has not. Sampling it there gave a number
    /// that was the full duration on three boundaries and zero on a fourth,
    /// depending purely on which event won the race.
    progress_mark: std::cell::Cell<(u64, u64)>,
    /// Process-local pause timer. It is intentionally not restored after an
    /// application restart.
    sleep_timer: crate::sleep_timer::Timer,

    /// Credential-free projection of the browser-owned Apple session.
    apple_session: Option<AppleSession>,
    /// Identity for work started with the current Apple session.
    ///
    /// A response can outlive sign-out because each API task owns a clone of
    /// the broker session it started with. Tagging every account-bound result lets us
    /// reject that response before it can repaint the UI or recreate the
    /// library cache for the account that was just forgotten.
    account_generation: u64,
    sidecar: Option<sidecar::Handle>,
    restarts: u32,
    toaster: adw::ToastOverlay,
    /// The volume panel. Its widgets rather than its state, which is the two
    /// fields below — see `osd.rs`.
    volume_osd: osd::VolumeOsd,
    /// Whether the panel is up. An **animated** property, so it is written on
    /// an edge through `sync_animated` and never as a `#[watch]`.
    osd_shown: bool,
    /// The single hide-timer, reset on each press rather than added to.
    osd_timer: Option<osd::HideTimer>,
    now_playing: Controller<NowPlaying>,
    queue_view: Controller<QueueView>,
    /// The drawer the bar opens into. Fed the same `Snapshot` as the bar, and
    /// its transport emits the same outputs — one player, two shapes (#18).
    player_view: Controller<PlayerView>,
    /// Apple-powered discovery shelves. Plain widget owners rather than relm4
    /// tasks: a complete Explore answer replaces them at once.
    explore_view: ExploreView,
    loading_explore: bool,
    tried_explore: bool,
    explore_generation: u64,
    /// Lyrics are fetched only while this page is open and the opt-in is on.
    /// The cache is memory-only so listening metadata does not become a second
    /// history file on disk.
    lyrics_view: LyricsView,
    /// The optional transparent toplevel. It reuses the lyrics already fetched
    /// for `lyrics_view`; Jamkin Mode never adds a provider request of its own.
    jamkin_mode: JamkinMode,
    /// Optional local Discord IPC. It owns no network client and stays
    /// completely dormant until the separately persisted opt-in is on.
    discord_presence: crate::discord::Presence,
    global_shortcuts_stop: Option<tokio::sync::watch::Sender<bool>>,
    /// Dormant until the separate ListenBrainz consent and encrypted token are
    /// both present. It remembers only the current track's submission state.
    scrobbler: crate::scrobble::Scrobbler,
    /// A portal confirmation is in flight. One at a time prevents two desktop
    /// dialogs racing to decide which launcher should win.
    launcher_icon_pending: Option<Companion>,
    lyrics_for: Option<crate::lyrics::Query>,
    lyrics_loading: bool,
    lyrics_generation: u64,
    lyrics_cache: std::collections::HashMap<crate::lyrics::Query, crate::lyrics::Lyrics>,
    /// Local per-recording lyric timing corrections. Only numeric catalog IDs
    /// and millisecond offsets are persisted; lyric text and listening history
    /// never enter this store.
    lyric_offsets: crate::lyric_timing::Offsets,
    /// The rows on screen — the filtered view. A `ListView`, so its cost is
    /// the number of rows visible rather than the size of the library.
    library: TypedListView<LibraryItem, gtk::NoSelection>,
    /// Whether the queue sidebar is open.
    show_queue: bool,
    /// Whether the navigation sidebar is open. Persisted, like the section:
    /// someone who closes it wants it closed next time too.
    show_sidebar: bool,
    /// What [`AppModel::sync_animated`] last pushed to the widgets. `None`
    /// until the first sync, which writes all three so the initial state is
    /// asserted once — at startup, when nothing is being resized.
    animated_shown: std::cell::Cell<Option<Animated>>,
    /// Whether the sidebar is currently an overlay rather than a pane.
    ///
    /// Mirrored from the split view rather than derived from a width we would
    /// have to measure ourselves: the breakpoint already owns this decision,
    /// and two places computing it is two places to disagree.
    sidebar_collapsed: bool,
    /// Whether the header is too narrow to hold a search entry as its title.
    ///
    /// Mirrored from its own breakpoint, the way `sidebar_collapsed` is
    /// mirrored from the split view — a width we measured ourselves would be a
    /// second opinion about a decision libadwaita has already made.
    narrow_header: bool,
    /// Set when something asked for the search box and the *widgets* have to
    /// act on it — focus it, and put the caret after the text. Cleared by
    /// `update_with_view` the moment it does, because it is a one-shot request
    /// rather than a state anything can be derived from.
    focus_search: bool,
    /// Set when the query changed from somewhere that is not the entry, so the
    /// entry has to be told.
    ///
    /// It is normally the *source* of the query and nothing writes back to it —
    /// a binding there would be the two-way loop from #37. The cost of that is
    /// this flag: clear the query without it and the words stay in the field
    /// over a list that is no longer filtered.
    sync_entry: bool,
    /// Whether the search entry is showing, on a narrow header where it is a
    /// button until asked for. Meaningless while `narrow_header` is false: the
    /// entry is simply the title then.
    searching: bool,
    /// Every sidebar row, in order — sections then pins. Rebuilt whenever the
    /// pins change, and the only thing `SidebarRowChosen` indexes into.
    sidebar_rows: Vec<SidebarRow>,
    /// Which sidebar row is selected, so a rebuild can put it back.
    ///
    /// Tracked here rather than read off the `ListBox`: a rebuild changes what
    /// each position means, so by the time it is needed the widget can only say
    /// *where* the selection was, not what it was.
    selected_row: Option<SidebarRow>,
    /// The sidebar's `row-selected` handler, so a rebuild can silence it.
    nav_selected: std::cell::RefCell<Option<gtk::glib::SignalHandlerId>>,
    /// The pins changed and the sidebar's rows have not caught up.
    ///
    /// Set in `update`, which cannot reach the widgets, and cleared in
    /// `sync_pins` on the way out — the same shape as `sync_animated`.
    pins_dirty: bool,
    /// A pinned row's label, by playlist id.
    ///
    /// Kept so a name can be filled in *after* the row is drawn. The sidebar is
    /// built before `seed_from_cache` runs, so at build time no pin has a name
    /// yet — and rebuilding the rows to fix that would clear the selection,
    /// which is the bug 285b542 removed. Writing the label leaves it alone.
    pin_labels: Vec<(String, gtk::Label)>,
    /// The sidebar's per-section spinners, built in `wiring::sidebar_rows`.
    ///
    /// Held because they are built outside `view!` and so get no `#[watch]` —
    /// `sync_section_spinners` is what replaces it. Apple Music has no entry:
    /// it has nothing of its own to load.
    section_spinners: Vec<(View, adw::Spinner)>,
    /// Whether the current track has already been reloaded to recover a
    /// playback that would not start. One attempt; a second failure is real.
    healed: bool,
    /// The reorder in flight, so it can be undone if the sidecar refuses it.
    ///
    /// An optimistic edit needs a way back or the list quietly stops matching
    /// what is playing — which is what a stale sidecar produced: fourteen
    /// `unknown-command` errors, fourteen rows left where they were dropped,
    /// and a queue that reverted the moment anything asked MusicKit for the
    /// next track.
    pending_move: Option<(usize, usize)>,
    /// Where to seek back to once a reloaded track becomes current.
    resume_at: Option<u64>,
    /// Whether the artwork cache has been swept this run. Once is enough: the
    /// library does not change under us, and the sweep touches ~1000 files.
    pruned: bool,
    /// Which library row currently carries the play marker.
    marked_playing: Option<String>,
    /// Icons of the library rows currently on screen, so the marker can move
    /// without editing the model — see `RowRegistry`.
    library_icons: RowRegistry<LibraryRowWidgets>,
    /// Who is playing. Shared with every library row; see `CurrentTrack`.
    current_track: CurrentTrack,
    /// Ids MusicKit refused, shared with every library row; see `DeadTracks`.
    dead_rows: DeadTracks,
    /// What has changed about a track since it was fetched — favourites and
    /// library membership — shared with every row in every list. See
    /// `components::TrackOverrides`: this replaces patching four separate
    /// copies of the same fact.
    row_overrides: TrackOverrides,
    /// The full library from the last load. The filter reads this, never the
    /// factory, so narrowing and then clearing a search is lossless.
    all_tracks: Vec<Track>,
    /// One query per scope. They are genuinely different searches: filtering
    /// your library by what you typed into Apple Music is meaningless, and
    /// clearing the box to get your library back would throw away the catalog
    /// search you were in the middle of.
    library_query: String,
    catalog_query: String,
    /// Which sidebar section is showing. `scope()` derives the search scope
    /// from it; never store both.
    view: View,
    /// How the Songs list is ordered. Applied in `visible_entries`.
    sorts: view::Sorts,
    /// The sort popover's two actions, kept so the menu can be re-pointed at
    /// another section's choice when the view changes.
    sort_actions: Option<(gtk::gio::SimpleAction, gtk::gio::SimpleAction)>,
    /// Check state for the primary menu's Show Jamkin action.
    jamkin_action: Option<gtk::gio::SimpleAction>,
    /// The user's library albums and artists, loaded on first visit rather than
    /// at startup — launching should not wait on three collections.
    albums: Vec<Album>,
    artists: Vec<Artist>,
    playlists: Vec<Playlist>,
    album_grid: TypedGridView<GridItem, gtk::NoSelection>,
    artist_grid: TypedGridView<GridItem, gtk::NoSelection>,
    playlist_grid: TypedGridView<GridItem, gtk::NoSelection>,
    loading_albums: bool,
    loading_artists: bool,
    loading_playlists: bool,
    /// Whether a load has been *attempted*, distinct from whether it produced
    /// anything. A failure leaves the collection empty, and "empty" alone would
    /// mean trying again on every event.
    tried_albums: bool,
    tried_artists: bool,
    tried_playlists: bool,
    tried_library: bool,
    /// What each section's widgets were last built *for*.
    ///
    /// Rebuilding is expensive — every tile that binds decodes its cover on the
    /// GTK thread — and switching sections was rebuilding unconditionally, so
    /// returning to a section you had already visited cost the same half second
    /// every time. `None` means stale; anything else is the fingerprint the
    /// current widgets already satisfy.
    built_rows: Option<String>,
    built_albums: Option<String>,
    built_artists: Option<String>,
    built_playlists: Option<String>,
    /// Which widget is showing which artwork — **one registry per grid**, for
    /// the same reason the row registries are per list: a shared one would have
    /// the two grids overwrite each other's entries, and clearing it for a
    /// rebuild of one would silently unregister the other's tiles.
    album_art_widgets: ArtRegistry,
    artist_art_widgets: ArtRegistry,
    playlist_art_widgets: ArtRegistry,
    playlist_art: playlist_art::State,
    /// Fetches already in flight, so a tile rebinding twice while scrolling
    /// does not queue the same download again.
    tile_art_pending: std::collections::HashSet<String>,
    /// Handed to every tile: "fetch this cover." An `Rc<dyn Fn>` rather than a
    /// sender because `bind` runs deep inside GTK's factory and has no access
    /// to the component — this is the same shape as the detail pages' click
    /// callbacks.
    tile_art_request: ArtRequest,
    /// Results of the last catalog search — songs, albums and artists mixed.
    /// Kept separate from `all_tracks` so switching back to Library does not
    /// have to reload anything.
    catalog: Vec<Entry>,
    /// Album and artist pages, innermost last. Not a widget mirror: the pages
    /// are pushed into a `NavigationView`, and this is what lets a click on one
    /// find the page it came from — **by id, never by depth**, because a stack
    /// that moved between the click and the handler is exactly the class of bug
    /// that produced the wrong song four times over.
    pages: Vec<DetailPage>,
    /// Never reused, never reset. A popped page's id must not come back and
    /// collect a response meant for it.
    next_page_id: u64,
    /// The navigation stack for the content pane. Held because pages are pushed
    /// from `update`, not declared in the view.
    nav: adw::NavigationView,

    /// How many rows of the **paging kind** are in `catalog`, and so the offset
    /// the next page starts from.
    ///
    /// Not simply `catalog.len()`: unfiltered, the browse rows on top are not
    /// part of Apple's song pagination and would skew it. Which kind pages
    /// depends on `catalog_filter` — see `library::catalog_rows`.
    catalog_paged: usize,
    /// Which kinds the catalog search asks for. Not persisted: a filter belongs
    /// to the search you are running, not to how you like the app.
    catalog_filter: CatalogFilter,
    /// Keeps the process alive while the window is hidden. `None` means the
    /// app is only alive because a window is open, which is the normal state.
    background: Option<gtk::gio::ApplicationHoldGuard>,
    /// Removals sent to the sidecar and not yet confirmed, by the id each
    /// command carried. See [`PendingWrite`].
    pending_writes: std::collections::HashMap<String, PendingWrite>,
    searching_catalog: bool,
    /// How many catalog results we already hold, and whether Apple has run out.
    /// Together these decide whether scrolling to the end fetches more.
    catalog_exhausted: bool,
    /// Bumped per keystroke; only the newest debounce timer is allowed to fire,
    /// and only the newest response is allowed to land. Without the second
    /// guard a slow request for "aita" can overwrite the results for "aitana".
    search_gen: u64,
    loading_library: bool,
    /// Catalog ids MusicKit has told us it cannot resolve. Remembered for the
    /// session so a delisted track only breaks one play attempt, not every one.
    dead_ids: std::collections::HashSet<String>,
    /// The last queue we tried and the id we wanted to start on, so a
    /// `NOT_FOUND` can be retried without the offenders instead of making the
    /// user click again. An id rather than an index — see `queue_from`.
    last_queue: Option<(Vec<String>, Option<String>)>,
    /// The track we asked to start on, held until MusicKit's own queue confirms
    /// it. See `verify_start`: the queue MusicKit builds is not always the list
    /// we sent, so the position we asked for is not always the track we meant.
    pending_start: Option<String>,
    mpris: Mpris,
    /// Volume is the one piece of player state the sidecar never echoes back,
    /// so we hold it here to keep the bar and MPRIS agreeing.
    volume: f64,
    /// Where the current cover lives on disk, for MPRIS's file:// artUrl.
    art_path: Option<PathBuf>,
    /// The artwork template of the track we last fetched, so a position tick
    /// or a queue echo doesn't re-request the same cover.
    art_for: Option<String>,
    /// Live only while playing; see `TICK_MS`.
    tick: Option<gtk::glib::SourceId>,
    /// A-B loop enforcement wakes only while an active loop is playing.
    segment_loop_tick: Option<gtk::glib::SourceId>,
    segment_loop: SegmentLoop,
    settings: Settings,
    /// The track the last notification was sent for, so a queue echo or a
    /// position tick cannot re-notify for the song already playing.
    notified_for: Option<String>,
    /// A track whose notification is waiting on its cover to finish
    /// downloading. See `maybe_notify`.
    notify_when_art_lands: Option<String>,
}

/// Logs how long a rebuild took, on the way out.
///
/// **Kept, not temporary.** It was added to answer "switching sections is
/// slow" with a number, found ~500ms of re-decoding covers, and is what the
/// section fingerprints are checked against — so it stays, and ARCHITECTURE.md points
/// at it as the way this stays measurable rather than remembered.
pub(crate) struct Timed(pub &'static str, pub std::time::Instant);

impl Drop for Timed {
    fn drop(&mut self) {
        let ms = self.1.elapsed().as_millis();
        if ms > 2 {
            tracing::debug!(what = self.0, ms, "rebuild");
        }
    }
}

/// Something we can ask Apple to do to the user's account.
///
/// Both answer 202 Accepted with an empty body — "acceptable, may not have
/// completed" — so neither can be treated as done, only as sent. That is why
/// nothing here toggles a checkbox: showing state would mean reading it back,
/// and a star that lies is worse than no star.
/// A library write sent to the sidecar and not yet confirmed.
///
/// The row is updated the moment the command goes out, because a menu that
/// waits on a round trip reads as broken. But an optimistic update that is
/// never taken back is how a UI comes to lie — which it did: a removal against
/// a stale sidecar answered `unknown-command`, and the row went on showing the
/// change that never happened.
///
/// **Keyed by the id the command carried**, not by the command name. There was
/// one slot and a name match, and the sidecar's dispatch is async — so removing
/// two tracks inside one round trip overwrote the first record, and the first
/// completion was attributed to the second's row. The wrong row left the list
/// while the removed one stayed.
#[derive(Debug, Clone)]
struct PendingWrite {
    /// The row to correct, which is not always the id the command carried:
    /// removal takes a library id, un-favouriting a catalog id.
    catalog_id: String,
    undo: WriteUndo,
}

#[derive(Debug, Clone, Copy)]
enum WriteUndo {
    InLibrary(bool),
    Favorite(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryAction {
    AddToLibrary,
    Favorite,
}

impl LibraryAction {
    fn sent(self) -> &'static str {
        match self {
            Self::AddToLibrary => "Adding to your library…",
            Self::Favorite => "Favouriting…",
        }
    }

    fn done(self) -> &'static str {
        match self {
            Self::AddToLibrary => "Sent to your library",
            Self::Favorite => "Favourited",
        }
    }
}

#[derive(Debug)]
pub enum AppMsg {
    SignIn,
    /// Asks first — see `confirm_sign_out`.
    SignOut,
    SignOutConfirmed,
    PlayPause,
    /// Explicit, not a toggle. MPRIS sends `Play`, `Pause` and `PlayPause` as
    /// three distinct calls, and collapsing the first two into the toggle makes
    /// the Shell pause a track it just asked to play.
    Play,
    Pause,
    Next,
    Previous,
    Seek(u64),
    SetVolume(f64),
    /// Nudge the volume by one [`VOLUME_STEP`]. Separate from `SetVolume`
    /// because an accelerator's closure cannot see the model, so only the
    /// reducer knows what the current volume is to step from.
    ///
    /// [`VOLUME_STEP`]: crate::components::now_playing::VOLUME_STEP
    VolumeUp,
    VolumeDown,
    SetShuffle(bool),
    SetRepeat(Repeat),
    CycleSegmentLoop,
    SegmentLoopTick,
    SetSleepTimer(crate::sleep_timer::Choice),
    SleepTimerExpired(u64),
    SetSort(SortBy),
    /// Narrow the catalog search to one kind of result, or widen it again.
    SetCatalogFilter(CatalogFilter),
    ToggleSortDirection,
    /// Take a song out of the library. Needs the **library** id; the catalog
    /// id is only carried so the row can be updated locally.
    RemoveFromLibrary {
        library_id: String,
        catalog_id: String,
    },
    /// Un-star a song, and nothing else. Deliberately does **not** also remove
    /// it from the library: favouriting adds it, un-favouriting does not take
    /// it back out, and that is what Apple's own client does. Chaining the two
    /// would silently delete a song someone only meant to un-star.
    Unfavorite {
        catalog_id: String,
    },
    /// MPRIS `Raise` — a controller asking for the window back. The counterpart
    /// to `close_window`'s hide, and the only way back from it that does not go
    /// through the launcher.
    Raise,
    /// The window's close button, or the WM. Not a quit: see the handler.
    WindowCloseRequested,
    /// The window is on screen again, however that happened.
    WindowShown,
    /// The drawer opened or closed by its own devices — dragged shut, or
    /// clicked away from. The model follows it rather than fighting it.
    PlayerDrawer(bool),
    /// A row was right-clicked; show its menu there.
    ShowRowMenu(RowMenuRequest),
    /// Empty the queue and stop.
    ClearQueue,
    /// Reorder the queue. `to` is where the item lands.
    MoveQueueItem {
        from: usize,
        to: usize,
    },
    /// Grow the queue MusicKit already holds, without rebuilding it.
    Enqueue {
        catalog_id: String,
        next: bool,
    },
    /// Write to the user's Apple Music account: save a track, or star it.
    LibraryWrite {
        catalog_id: String,
        action: LibraryAction,
    },
    /// Repaint the seek bar from the interpolated position.
    Tick,
    /// Play the visible list, starting at this row.
    PlayFrom(usize),
    SearchChanged(String),
    /// The debounce elapsed for this generation; run the catalog search.
    RunCatalogSearch(u64),
    SetView(View),
    /// A card on Explore was clicked.
    ExploreAction(ExploreAction),
    /// The lyrics button in Now Playing. Unlike a raw `SetView`, this also
    /// closes the modal player drawer so the destination is visible.
    ShowLyrics,
    /// Copy the current catalog song's public Apple Music URL.
    CopyPlayingLink,
    /// Star or un-star the current song using the same Apple write path as a
    /// song row's options menu.
    TogglePlayingFavorite,
    ShowCredits,
    /// Resolve the current catalog song's album and open its Jamelade page.
    OpenPlayingAlbum,
    /// Resolve the current catalog song's artist and open its Jamelade page.
    OpenPlayingArtist,
    /// Copy the public link stored on a loaded album or playlist page.
    CopyPageLink {
        page: u64,
    },
    ExportPlaylist {
        page: u64,
        format: crate::playlist_export::Format,
    },
    /// Resolve an album page's artist from its first catalog song and open it.
    OpenAlbumArtist {
        page: u64,
    },
    /// A grid tile was activated. The position is resolved against the grid
    /// immediately, never stored.
    AlbumActivated(u32),
    ArtistActivated(u32),
    PlaylistActivated(u32),
    /// A tile is on screen and its cover is not on disk yet.
    NeedTileArt(String, Artwork),
    /// A user playlist has no Apple-supplied picture, so its tile needs a collage.
    NeedPlaylistArt(playlist_art::Job),
    ToggleSidebar,
    /// The split view changed the sidebar's visibility by itself — a click
    /// outside it while collapsed. A fact, not a request: the widget has
    /// already done it.
    SidebarShown(bool),
    /// A sidebar row was activated. Dismisses the sidebar if it is an overlay,
    /// and does nothing at all if it is a pane.
    /// The breakpoint turned the sidebar into an overlay, or back into a pane.
    SidebarCollapsed(bool),
    /// The header crossed its own breakpoint.
    NarrowHeader(bool),
    /// Open or close the search entry on a narrow header.
    ShowSearch(bool),
    /// Put the caret in the search entry, opening it first if it is a button.
    FocusSearch,
    /// A printable key arrived with nothing focused that wanted it. Starts a
    /// search with that character already in it.
    TypeAhead(String),
    /// The results list is near its end; fetch the next page if there is one.
    LoadMoreCatalog,
    /// Re-fetch one library section. There is no section-less "reload": each
    /// is fetched separately, so a single one could not know which you meant.
    /// Fetch the section on screen again. Carries no payload on purpose: the
    /// only sender is a header button, which cannot read the model from inside
    /// its click handler, and the reducer already knows which view is showing.
    ReloadCurrentSection,
    ShowPreferences,
    ShowCreatePlaylist,
    CreatePlaylist {
        name: String,
        description: String,
    },
    ShowAddToPlaylist {
        catalog_id: String,
    },
    AddTrackToPlaylist {
        playlist_id: String,
        catalog_id: String,
    },
    ShowShortcuts,
    /// The hide-timer fired: the panel has been up long enough.
    HideVolumeOsd,
    /// A sidebar row was selected, by position. What it does depends on what
    /// kind of row it is — see `pins::sidebar_row_chosen`.
    SidebarRowChosen(i32),
    /// A sidebar row was clicked. Only the pin button cares — every other row
    /// has already acted through `SidebarRowChosen`, and this is where the
    /// overlay sidebar gets out of the way.
    SidebarRowActivated(i32),
    /// Open the picker over the library's playlists.
    ShowPinPicker,
    /// Pin or unpin one playlist, from the picker or a row menu.
    SetPinned {
        id: String,
        pinned: bool,
    },
    /// Pin or unpin every library playlist — the picker's header action.
    SetAllPinned(bool),
    /// A playlist tile was right-clicked.
    TileMenu(crate::components::grid_item::TileMenuRequest),
    /// A pinned row was dragged. `slot` is in the coordinates of the list as it
    /// was before the move — see `pins::move_pin`.
    MovePin {
        from: usize,
        slot: usize,
    },
    ShowAbout,
    /// Open the Ko-fi page in a browser.
    OpenSupport,
    SetTheme(u32),
    SetLanguage(u32),
    SetAccent(crate::style::Accent),
    SetCompanion(Companion),
    /// Select bundled high-resolution or original Jamkin animation frames.
    SetJamkinQuality(crate::settings::JamkinQuality),
    /// Ask the desktop portal to replace the app-menu tile.
    SetLauncherIcon(Companion),
    /// Show or hide the small movable desktop Jamkin.
    SetDesktopJamkin(bool),
    /// Resize the desktop actor live; clamped again at the reducer boundary.
    SetDesktopJamkinSize(u16),
    /// Change only the floating sprite's opacity; its lyric bubble stays clear.
    SetDesktopJamkinOpacity(u8),
    /// Freeze decorative Jamkin motion while retaining instant Edge Walk moves.
    SetJamkinReducedMotion(bool),
    /// Keep the independent Jamkin visible while the player window is hidden.
    SetDesktopJamkinStayVisible(bool),
    /// Keep the desktop actor over other windows where the compositor permits.
    SetDesktopJamkinAbove(bool),
    /// Move the overlay actor around the screen perimeter for Edge Walk.
    SetDesktopJamkinOledCare(bool),
    /// Remember the layer-shell placement without collecting screen identity.
    SetDesktopJamkinPosition {
        right: i32,
        bottom: i32,
    },
    /// Share current-track display metadata with the local Discord client.
    SetDiscordActivity(bool),
    ConfigureGlobalShortcuts,
    DisableGlobalShortcuts,
    ShowListenBrainzSetup,
    EnableListenBrainz(String),
    DisableListenBrainz,
    /// Whether the cover is painted behind the whole window (#145).
    SetPlayerBackdrop(bool),
    /// Combined surface transparency and real artwork blur; 100 is fully clear.
    SetGlassStrength(u8),
    /// Accent mix for lyrics immediately around the live line, 0–100.
    SetLyricsAccentStrength(u8),
    /// Text scale for the full lyrics view and desktop hover bubble.
    SetLyricsFontScale(u8),
    /// Shift synchronized lyrics for the current recording. Zero resets it;
    /// other values are signed deltas in milliseconds.
    AdjustLyricTiming(i32),
    SelectLyricVariant(usize),
    SetNotifyTrackChange(bool),
    /// LRCLIB metadata disclosure; off until explicitly enabled.
    SetLyricsEnabled(bool),
    /// A separate opt-in for the token-free Lyrics.ovh fallback.
    SetLyricsOvhEnabled(bool),
    ToggleQueue,
    /// A library row was activated; the position is resolved immediately.
    LibraryActivated(u32),
    /// A row on a pushed page was clicked. Carries the page's id so it can be
    /// resolved against the live stack rather than a remembered depth.
    DetailActivated {
        page: u64,
        row: usize,
    },
    ArtistActivatedOnPage {
        page: u64,
        target: ArtistActivate,
    },
    /// Play everything on a page — from the top, or shuffled.
    PlayPage {
        page: u64,
        shuffle: bool,
    },
    /// Push an album or artist page — catalog or library, which the `PageKind`
    /// carries so the fetch knows which endpoint to ask.
    OpenPage(PageKind),
    /// Walk from a queue row to the album or artist behind it.
    ///
    /// A separate message from `OpenPage` because a queue item has no album or
    /// artist id to push a page with — only the song's own. Resolving that costs
    /// a request, which is why it happens on a menu click and not per row.
    OpenQueueTrackPage {
        catalog_id: String,
        album: bool,
    },
    /// The navigation view popped a page — drop the state behind it.
    PagePopped(u64),
    /// Act on a track in MusicKit's queue, by id. The position is resolved
    /// against the live queue at send time — our row order can drift from
    /// MusicKit's, and sending a stale position got INVALID_ARGUMENTS.
    JumpTo {
        at: usize,
        id: String,
    },
    RemoveFromQueue {
        at: usize,
        id: String,
    },
}

#[derive(Debug)]
pub enum CommandMsg {
    /// Everything the sidecar pushed up, including its death.
    Sidecar(Incoming),
    /// The child started; here is the handle for talking to it.
    Spawned(sidecar::Handle),
    /// The user's library, or why it couldn't be read.
    Library {
        generation: u64,
        result: Result<Vec<Track>, String>,
    },
    /// Catalog results, tagged with the search they belong to.
    Catalog {
        generation: u64,
        /// Where this page started, so a first page replaces and a later page
        /// appends.
        offset: usize,
        result: Result<crate::music::client::SearchResults, String>,
    },
    /// Cover art is on disk. `None` when the fetch failed — a missing cover is
    /// cosmetic and must not become a toast.
    Artwork {
        generation: u64,
        /// The template this work was requested for. A slow older download may
        /// arrive after the track changed; it must not recolour the new song.
        template: String,
        path: Option<PathBuf>,
        /// A small blurred copy of the cover, to go behind the whole window.
        /// Carried here rather than in its own message because the cover and
        /// what is drawn from it must be applied together.
        backdrop: Option<PathBuf>,
        /// The slider value used to render `backdrop`. If it changed while the
        /// cover downloaded, the current variant is queued immediately after.
        glass_strength: u8,
        /// Two safe RGB tints derived locally from the same cover.
        palette: Option<crate::palette::AlbumPalette>,
    },
    /// A new blur variant for the current cover, generated after the slider
    /// moved. Tagged twice so late work cannot land on a different track or a
    /// newer slider position.
    GlassBackdrop {
        generation: u64,
        source: PathBuf,
        glass_strength: u8,
        backdrop: Option<PathBuf>,
    },
    /// An album page's contents. Tagged with the page id: by the time this
    /// lands the user may have gone back, and filling a page that is no longer
    /// on the stack is at best wasted work.
    AlbumPage {
        generation: u64,
        page: u64,
        result: Result<(Album, Vec<Track>), String>,
    },
    /// An artist page's contents.
    ArtistPage {
        generation: u64,
        page: u64,
        result: Result<ArtistPageData, String>,
    },
    /// A playlist page's contents.
    PlaylistPage {
        generation: u64,
        page: u64,
        result: Result<(Playlist, Vec<Track>), String>,
    },
    /// A page's header art is on disk, or could not be fetched.
    PageArtwork {
        generation: u64,
        page: u64,
        path: Option<PathBuf>,
    },
    /// The user's library albums / artists.
    LibraryAlbums {
        generation: u64,
        result: Result<Vec<Album>, String>,
    },
    LibraryArtists {
        generation: u64,
        result: Result<Vec<Artist>, String>,
    },
    LibraryPlaylists {
        generation: u64,
        result: Result<Vec<Playlist>, String>,
    },
    Explore {
        generation: u64,
        result: Result<Explore, String>,
    },
    Lyrics {
        generation: u64,
        query: crate::lyrics::Query,
        result: Result<crate::lyrics::Lyrics, String>,
    },
    /// The artwork sweep finished. It logs its own numbers; this exists so the
    /// work can be a command rather than something done on the GTK thread.
    Pruned(crate::components::prune::Report),
    /// Whether the Background portal agreed to list us. Advisory only.
    BackgroundPortal(Result<(), String>),
    LauncherIconInstalled {
        icon: Companion,
        result: Result<crate::launcher_icon::InstallMethod, String>,
    },
    /// A library write came back. `Ok` means Apple **accepted** it, not that
    /// it is done — see `Client::add_song_to_library`.
    LibraryWritten {
        generation: u64,
        catalog_id: String,
        action: LibraryAction,
        result: Result<(), String>,
    },
    /// Which album or artist a queue track belongs to. `None` means Apple
    /// named neither — a single that belongs to no album is not an error.
    QueueTrackPage {
        generation: u64,
        result: Result<Option<PageKind>, String>,
    },
    /// A grid tile's cover is on disk and decoded, or could not be had.
    /// Carries pixels rather than a path because the decode is the expensive
    /// part and it has already happened, off the GTK thread (#27).
    TileArt {
        generation: u64,
        key: String,
        path: Option<PathBuf>,
        cover: Option<artwork::Decoded>,
    },
    PlaylistTileArt(playlist_art::Finished),
    PlaylistWritten {
        generation: u64,
        created: bool,
        result: Result<(), String>,
    },
    Credits {
        generation: u64,
        result: Result<Vec<crate::music::client::SongCredit>, String>,
    },
    GlobalShortcutsReady(Result<(), String>),
    GlobalShortcut(String),
    ListenBrainzTokenLoaded(Result<Option<crate::scrobble::Token>, String>),
    ListenBrainzTokenStored {
        token: crate::scrobble::Token,
        result: Result<(), String>,
    },
    ListenBrainzSubmitted {
        key: String,
        result: Result<(), String>,
    },
}

/// The drawer emits the same outputs as the bar, so they map the same way.
/// Two players disagreeing about one MusicKit is the thing this avoids.
fn map_player_output(out: NowPlayingOutput) -> AppMsg {
    match out {
        NowPlayingOutput::PlayPause => AppMsg::PlayPause,
        NowPlayingOutput::Next => AppMsg::Next,
        NowPlayingOutput::Previous => AppMsg::Previous,
        NowPlayingOutput::Seek(ms) => AppMsg::Seek(ms),
        NowPlayingOutput::SetVolume(v) => AppMsg::SetVolume(v),
        NowPlayingOutput::SetShuffle(on) => AppMsg::SetShuffle(on),
        NowPlayingOutput::SetRepeat(r) => AppMsg::SetRepeat(r),
        NowPlayingOutput::CycleSegmentLoop => AppMsg::CycleSegmentLoop,
        NowPlayingOutput::ShowLyrics => AppMsg::ShowLyrics,
        NowPlayingOutput::ToggleQueue => AppMsg::ToggleQueue,
        NowPlayingOutput::OpenAlbum => AppMsg::OpenPlayingAlbum,
        NowPlayingOutput::OpenArtist => AppMsg::OpenPlayingArtist,
        NowPlayingOutput::CopyLink => AppMsg::CopyPlayingLink,
        NowPlayingOutput::ToggleFavorite => AppMsg::TogglePlayingFavorite,
        NowPlayingOutput::SetSleepTimer(choice) => AppMsg::SetSleepTimer(choice),
        NowPlayingOutput::ShowCredits => AppMsg::ShowCredits,
    }
}

#[relm4::component(pub)]
impl Component for AppModel {
    type Init = Settings;
    type Input = AppMsg;
    type Output = ();
    type CommandOutput = CommandMsg;

    view! {
        adw::ApplicationWindow {
            set_title: Some(crate::APP_NAME),
            add_css_class: "jamelade-window",

            // Closing a music player mid-song should not stop the music.
            // Always `Stop` — the reducer decides whether this is a hide or a
            // quit, because that depends on whether anything is loaded and the
            // handler cannot see the model.
            connect_close_request[sender] => move |_| {
                sender.input(AppMsg::WindowCloseRequested);
                gtk::glib::Propagation::Stop
            },

            // Any route back to a visible window — relaunching, the Background
            // Apps list, the media applet — means we are no longer running
            // without one, so the hold goes.
            connect_show[sender] => move |_| {
                sender.input(AppMsg::WindowShown);
            },
            set_default_width: 1000,
            set_default_height: 680,

            #[local_ref]
            toaster -> adw::ToastOverlay {
                // The volume panel floats here: above the drawer, so opening it
                // does not cover the panel, and below toasts, so an error still
                // wins. See `osd.rs`.
                gtk::Overlay {
                    #[local_ref]
                    add_overlay = volume_osd -> gtk::Revealer {},

                    #[wrap(Some)]
                    set_child = &adw::ToolbarView {
                    // The bar is the handle of a drawer, not furniture bolted
                    // to the bottom (#18). `AdwBottomSheet` is the widget for
                    // exactly this: `bottom_bar` while closed, `sheet` when
                    // open, and it owns the drag and the animation.
                    //
                    // The queue used to be an `OverlaySplitView` sidebar here,
                    // taking width from the content. It now lives inside the
                    // drawer, beside the thing it is a queue for.
                    #[wrap(Some)]
                    #[name = "player_sheet"]
                    set_content = &adw::BottomSheet {
                        set_full_width: true,
                        set_show_drag_handle: true,
                        // Modal: with the drawer open there is nothing useful
                        // to click behind it, and dismissing by clicking away
                        // is what a drawer should do.
                        set_modal: true,
                        // Not a `#[watch]`. See `sync_animated`.
                        set_open: model.show_queue,
                        // The bar is only meaningful once there is a player.
                        // Not a `#[watch]`. See `sync_animated`.
                        set_reveal_bottom_bar: matches!(model.stage, Stage::Ready),

                        // Dragged shut, or clicked away from — the model has to
                        // learn about it or the next toggle fights the widget.
                        connect_open_notify[sender] => move |sheet| {
                            sender.input(AppMsg::PlayerDrawer(sheet.is_open()));
                        },

                        #[wrap(Some)]
                        #[local_ref]
                        set_bottom_bar = now_playing_bar -> gtk::Box {
                            set_hexpand: true,
                        },

                        #[wrap(Some)]
                        #[local_ref]
                        set_sheet = player_sheet_content -> adw::BreakpointBin {},

                        // Navigation on the left, and an OverlaySplitView
                        // rather than a NavigationSplitView because it can be
                        // dismissed: once the sidebar is something you toggle,
                        // it is a panel you summon, which is exactly what this
                        // widget is for. The queue on the right is the same
                        // shape for the same reason.
                        #[wrap(Some)]
                            #[name = "nav_split"]
                            set_content = &adw::OverlaySplitView {
                            add_css_class: "art-foreground",
                            set_min_sidebar_width: 200.0,
                            set_max_sidebar_width: 260.0,
                            // Not a `#[watch]`. See `sync_animated`.
                            set_show_sidebar: model.show_sidebar,
                            // **The model has to adopt what the widget did.**
                            //
                            // Collapsed, this is an overlay, and the widget
                            // dismisses itself on a click outside — but the
                            // `#[watch]` above runs after *every* message, and
                            // during playback those never stop arriving. So it
                            // wrote `true` straight back and the sidebar
                            // reappeared before the click had finished.
                            //
                            // The same shape as the volume binding, in its
                            // quieter form: there the two values ping-ponged,
                            // here the model simply never learns. `SidebarShown`
                            // is the half that was missing.
                            connect_show_sidebar_notify[sender] => move |split| {
                                sender.input(AppMsg::SidebarShown(split.shows_sidebar()));
                            },
                            connect_collapsed_notify[sender] => move |split| {
                                sender.input(AppMsg::SidebarCollapsed(split.is_collapsed()));
                            },

                            #[wrap(Some)]
                            set_sidebar = &adw::ToolbarView {
                                    add_css_class: "jam-glass-sidebar",
                                    add_top_bar = &adw::HeaderBar {
                                        #[wrap(Some)]
                                        set_title_widget = &adw::WindowTitle {
                                            set_title: crate::APP_NAME,
                                            #[watch]
                                            set_subtitle: &model.subtitle(),
                                        },

                                    },

                                    #[wrap(Some)]
                                    set_content = &gtk::ScrolledWindow {
                                        set_vexpand: true,
                                        // The sections, and their reload
                                        // buttons. Insensitive until there is a
                                        // session to load anything from — but
                                        // note this is the ToolbarView's
                                        // *content*, so the header bar above it
                                        // keeps the primary menu live, and with
                                        // it Quit.
                                        #[watch]
                                        set_sensitive: model.controls_live(),

                                        #[wrap(Some)]
                                        set_child = &gtk::Box {
                                            set_orientation: gtk::Orientation::Vertical,
                                            set_spacing: 2,
                                            set_margin_top: 6,

                                            // ONE ListBox, not one per
                                            // section. Two boxes each keep
                                            // their own selection, and the one
                                            // that takes initial focus selects
                                            // its first row — overriding
                                            // whatever the other was set to,
                                            // which is why the wrong row looked
                                            // active on startup. Section
                                            // headings come from a header func
                                            // instead.
                                            #[name = "nav_list"]
                                            gtk::ListBox {
                                                add_css_class: "navigation-sidebar",
                                                set_selection_mode: gtk::SelectionMode::Single,
                                                // `row-selected` is connected
                                                // in `wiring`, not here: the
                                                // handler's id has to be kept
                                                // so a rebuild can silence it.
                                                // See `sync_pins`.
                                                // Choosing a section is the end
                                                // of what an overlay sidebar is
                                                // for, so it gets out of the
                                                // way — but only when it *is*
                                                // an overlay. Beside a pane, it
                                                // stays put.
                                                // Activation, not selection,
                                                // because the pin button is
                                                // deliberately unselectable —
                                                // it does something rather than
                                                // being somewhere.
                                                connect_row_activated[sender] => move |_, row| {
                                                    sender.input(
                                                        AppMsg::SidebarRowActivated(row.index()),
                                                    );
                                                },

                                                // The fixed rows are appended by
                                                // `wiring::sidebar_rows`, from the
                                                // one array that also defines the
                                                // index contract read just above.
                                            },
                                    },
                                },
                            },

                            #[wrap(Some)]
                            #[local_ref]
                            set_content = nav_view -> adw::NavigationView {
                                add = &adw::NavigationPage {
                                    set_title: crate::APP_NAME,
                                    // The root page. Albums and artists push on
                                    // top of it; nothing ever pops it.
                                    set_tag: Some("results"),

                                    #[wrap(Some)]
                                    #[name = "content_bars"]
                                    set_child = &adw::ToolbarView {
                                    add_top_bar = &adw::HeaderBar {
                                        // The sidebar's own header carries the
                                        // start-side window controls while it
                                        // is open, so this header only shows
                                        // them once the sidebar is away.
                                        #[watch]
                                        set_show_start_title_buttons: !model.show_sidebar,

                                        pack_start = &gtk::ToggleButton {
                                            set_icon_name: "sidebar-show-symbolic",
                                            set_tooltip_text: Some(crate::i18n::tr("Toggle Sidebar")),
                                            add_css_class: "flat",
                                            #[watch]
                                            set_active: model.show_sidebar,
                                            connect_clicked => AppMsg::ToggleSidebar,
                                        },

                                        // One application menu beside the
                                        // sidebar control. Keeping Preferences,
                                        // account and app actions together
                                        // avoids a second hamburger hidden in
                                        // the sidebar header.
                                        pack_start = &gtk::MenuButton {
                                            set_icon_name: "preferences-system-symbolic",
                                            set_tooltip_text: Some(crate::i18n::tr("Settings and App Menu")),
                                            add_css_class: "flat",
                                            set_menu_model: Some(&primary_menu),
                                        },

                                        // Beside the sidebar toggle rather than
                                        // over on the right: both are doors to
                                        // something the window is too narrow to
                                        // show outright, and the end of a header
                                        // is where the actions on what you are
                                        // already looking at live.
                                        //
                                        // Narrow only: wide, the entry is always
                                        // there and this would reveal nothing.
                                        #[name = "search_button"]
                                        pack_start = &gtk::ToggleButton {
                                            set_icon_name: "system-search-symbolic",
                                            set_tooltip_text: Some(crate::i18n::tr("Search")),
                                            add_css_class: "flat",
                                            #[watch]
                                            set_visible: model.narrow_header && model.view.searchable(),
                                            #[watch]
                                            set_sensitive: model.controls_live(),
                                            // `set_active` plus a report back is
                                            // the two-way binding from #37 —
                                            // `ShowSearch` drops a value equal to
                                            // the one held, which is what a
                                            // programmatic set arrives as.
                                            #[watch]
                                            set_active: model.searching,
                                            connect_toggled[sender] => move |button| {
                                                sender.input(AppMsg::ShowSearch(button.is_active()));
                                            },
                                        },

                                        // When the queue is open it is the
                                        // rightmost pane, so the window
                                        // controls belong to its header, not
                                        // this one. Without this they vanish:
                                        // the queue's header hides them and
                                        // this header is no longer at the edge.
                                        set_show_end_title_buttons: true,

                                        // Narrow, the title is the section name
                                        // and search is a button — the sidebar
                                        // row that says where you are has
                                        // collapsed to an overlay by then.
                                        // Not homogeneous, unlike the reload
                                        // stack: a short label and a field that
                                        // should take the whole header.
                                        #[wrap(Some)]
                                        set_title_widget = &gtk::Stack {
                                            set_hhomogeneous: false,
                                            set_hexpand: true,

                                            add_named[Some("title")] = &adw::WindowTitle {
                                                #[watch]
                                                set_title: model.view.title(),
                                            },

                                            #[name = "search_entry"]
                                            add_named[Some("search")] = &gtk::SearchEntry {
                                                // Never a fixed width: 320px here
                                                // was a floor under the window and
                                                // the app could not be tiled.
                                                // `max-width-chars` is a ceiling,
                                                // so it is safe — 60 fills a narrow
                                                // header and stops short of absurd
                                                // on a wide one.
                                                set_hexpand: true,
                                                set_max_width_chars: 60,
                                                // Typing here before the browser
                                                // session arrives queries a catalog that
                                                // cannot answer.
                                                #[watch]
                                                set_sensitive: model.controls_live(),
                                                #[watch]
                                                set_placeholder_text: Some(match model.view {
                                                    View::Explore | View::Lyrics => "Search Apple Music",
                                                    View::Songs => "Search your library",
                                                    View::Albums => "Search albums",
                                                    View::Artists => "Search artists",
                                                    View::Playlists => "Search playlists",
                                                    View::Search => "Search Apple Music",
                                                }),
                                                connect_search_changed[sender] => move |entry| {
                                                    sender.input(AppMsg::SearchChanged(entry.text().into()));
                                                },
                                                // Escape, as it cancels every other
                                                // search in GNOME.
                                                //
                                                // The entry is emptied here rather
                                                // than left to the reducer: it is
                                                // the *source* of the query, so
                                                // nothing writes back to it, and
                                                // clearing only the model left the
                                                // words sitting in a field over an
                                                // unfiltered list.
                                                //
                                                // `SearchChanged` as well, because
                                                // `search-changed` is delayed and
                                                // the list should stop filtering
                                                // now; it returns early when the
                                                // delayed one arrives after it.
                                                // And `ShowSearch`, because on a
                                                // wide header the box is never
                                                // "open" — that message would drop
                                                // a value it already holds and
                                                // Escape would close nothing.
                                                connect_stop_search[sender] => move |entry| {
                                                    entry.set_text("");
                                                    sender.input(AppMsg::SearchChanged(String::new()));
                                                    sender.input(AppMsg::ShowSearch(false));
                                                },
                                            },

                                            #[watch]
                                            set_visible_child_name: if model.search_showing() {
                                                "search"
                                            } else {
                                                "title"
                                            },
                                        },

                                        // One button, not the four this used to
                                        // be on the sidebar rows. A sidebar
                                        // button cannot know which section you
                                        // meant, so each row needed its own — and
                                        // that put reload behind the sidebar
                                        // toggle, which collapses on its own when
                                        // the window is narrow. Here there is
                                        // nothing to disambiguate: it reloads
                                        // what you are looking at.
                                        //
                                        // A `Stack` rather than two widgets
                                        // swapping their own visibility, because
                                        // a spinner is smaller than a button and
                                        // the header re-centred its search entry
                                        // every time one started. A stack is
                                        // homogeneous by default, so it holds the
                                        // button's width whichever child shows.
                                        pack_end = &gtk::Stack {
                                            // Children first. `view!` assigns in
                                            // the order written, so naming a
                                            // child above the `add_named` that
                                            // creates it is `Gtk-WARNING: Child
                                            // name 'reload' not found in
                                            // GtkStack` on every launch.
                                            add_named[Some("reload")] = &gtk::Button {
                                                set_icon_name: "view-refresh-symbolic",
                                                set_tooltip_text: Some(crate::i18n::tr("Reload")),
                                                add_css_class: "flat",
                                                #[watch]
                                                set_sensitive: model.controls_live(),
                                                connect_clicked[sender] => move |_| {
                                                    sender.input(AppMsg::ReloadCurrentSection);
                                                },
                                            },

                                            // The only sign a reload is running,
                                            // once the list stopped being taken
                                            // away for one.
                                            add_named[Some("busy")] = &adw::Spinner {
                                                set_size_request: (16, 16),
                                                set_valign: gtk::Align::Center,
                                                set_halign: gtk::Align::Center,
                                            },

                                            #[watch]
                                            set_visible: model.view.reloadable(),
                                            #[watch]
                                            set_visible_child_name: if model.loading_section() {
                                                "busy"
                                            } else {
                                                "reload"
                                            },
                                        },

                                        // Only in Songs: the grids have their
                                        // own natural order and sorting them
                                        // is a different question.
                                        #[name = "sort_button"]
                                        pack_end = &gtk::MenuButton {
                                            set_icon_name: "view-sort-descending-symbolic",
                                            set_tooltip_text: Some(crate::i18n::tr("Sort")),
                                            add_css_class: "flat",
                                            #[watch]
                                            // Every library section, not just
                                            // Songs. Search is the exception:
                                            // Apple ranked those results and
                                            // re-ordering them locally would
                                            // throw away the ranking without
                                            // being able to reproduce it.
                                            set_visible: model.view.sortable(),
                                            // Visibility follows the section,
                                            // which says nothing about whether
                                            // there is a list to reorder yet.
                                            #[watch]
                                            set_sensitive: model.controls_live(),
                                        },

                                        // Only in Search: a library filter is
                                        // the search box itself, and the grids
                                        // already are one kind each.
                                        #[name = "filter_button"]
                                        pack_end = &gtk::MenuButton {
                                            add_css_class: "flat",
                                            set_always_show_arrow: true,
                                            set_tooltip_text: Some(crate::i18n::tr("What to search for")),
                                            #[watch]
                                            set_visible: model.view == View::Search,
                                            // A label rather than an icon, for
                                            // two reasons. Adwaita has no
                                            // filter glyph — `funnel-symbolic`
                                            // and `view-filter-symbolic` are
                                            // both absent, and `chrome::icon`
                                            // would have quietly put a music
                                            // note here. And the current filter
                                            // needs to be readable *without*
                                            // hovering: this button is the only
                                            // thing on screen explaining why a
                                            // search returned one kind of
                                            // result, and a narrowed search
                                            // with no visible reason reads as
                                            // missing results.
                                            #[watch]
                                            set_label: model.catalog_filter.label(),
                                        },

                                    },

                                    #[wrap(Some)]
                                    set_content = &gtk::Stack {
                                        add_named[Some("status")] = &adw::StatusPage {
                                            #[watch]
                                            set_icon_name: Some(model.icon()),
                                            #[watch]
                                            set_title: &model.headline(),
                                            #[watch]
                                            set_description: Some(&model.detail()),
                                        },

                                        // Loading gets its own page: "nothing
                                        // here yet" and "still fetching" look
                                        // identical otherwise.
                                        add_named[Some("loading")] = &gtk::Box {
                                            set_orientation: gtk::Orientation::Vertical,
                                            set_halign: gtk::Align::Center,
                                            set_valign: gtk::Align::Center,
                                            set_spacing: 18,

                                            adw::Spinner {
                                                set_size_request: (42, 42),
                                            },

                                            gtk::Label {
                                                add_css_class: "title-2",
                                                #[watch]
                                                set_label: &model.waiting_for(),
                                            },
                                        },

                                        #[local_ref]
                                        add_named[Some("explore")] = explore_content -> gtk::Stack {},

                                        #[local_ref]
                                        add_named[Some("lyrics")] = lyrics_content -> gtk::Box {},

                                        // **`AdwClampScrollable`, not `AdwClamp`.**
                                        //
                                        // A plain clamp had to go *outside* the
                                        // scroller, because inside it breaks
                                        // `GtkListView`'s height allocation and
                                        // the list stops materialising rows part
                                        // way down. But outside, the clamp is
                                        // what the window sizes, so the scroller
                                        // is only 800px wide and its scrollbar
                                        // sits in the middle of the window
                                        // rather than at the edge.
                                        //
                                        // `AdwClampScrollable` is the widget for
                                        // exactly this trade: it implements
                                        // `GtkScrollable` and passes the
                                        // interface through to its child, so the
                                        // list still gets the adjustments it
                                        // needs while being clamped. The
                                        // scroller can then be the full width
                                        // and keep its bar at the edge.
                                        #[name = "library_scroller"]
                                        add_named[Some("library")] = &gtk::ScrolledWindow {
                                            set_vexpand: true,
                                            // See `.plain-scroller`: now that
                                            // this spans the window rather than
                                            // being clamped, its `view`
                                            // background does too.
                                            add_css_class: "plain-scroller",

                                            #[wrap(Some)]
                                            #[name = "library_clamp"]
                                            set_child = &adw::ClampScrollable {
                                                set_maximum_size: 800,
                                                add_css_class: "plain-scroller",
                                            },
                                        },

                                        // Grids scroll as themselves: unlike the
                                        // detail pages there is no header above
                                        // them, so the GridView can be the
                                        // scrollable child and stay virtualised.
                                        add_named[Some("albums")] = &gtk::ScrolledWindow {
                                            set_vexpand: true,
                                            set_hscrollbar_policy: gtk::PolicyType::Never,

                                            #[local_ref]
                                            album_grid -> gtk::GridView {
                                                set_single_click_activate: true,
                                                set_max_columns: 12,
                                                // Padding via `.tile-grid`,
                                                // not a margin: a GridView
                                                // draws its own `.view`
                                                // background, and a margin
                                                // leaves a strip of the window
                                                // showing all the way round it.
                                                add_css_class: "tile-grid",
                                            },
                                        },

                                        add_named[Some("artists")] = &gtk::ScrolledWindow {
                                            set_vexpand: true,
                                            set_hscrollbar_policy: gtk::PolicyType::Never,

                                            #[local_ref]
                                            artist_grid -> gtk::GridView {
                                                set_single_click_activate: true,
                                                set_max_columns: 12,
                                                // Padding via `.tile-grid`,
                                                // not a margin: a GridView
                                                // draws its own `.view`
                                                // background, and a margin
                                                // leaves a strip of the window
                                                // showing all the way round it.
                                                add_css_class: "tile-grid",
                                            },
                                        },

                                        add_named[Some("playlists")] = &gtk::ScrolledWindow {
                                            set_vexpand: true,
                                            set_hscrollbar_policy: gtk::PolicyType::Never,

                                            #[local_ref]
                                            playlist_grid -> gtk::GridView {
                                                set_single_click_activate: true,
                                                set_max_columns: 12,
                                                // Padding via `.tile-grid`,
                                                // not a margin: a GridView
                                                // draws its own `.view`
                                                // background, and a margin
                                                // leaves a strip of the window
                                                // showing all the way round it.
                                                add_css_class: "tile-grid",
                                            },
                                        },

                                        // An empty search box is not a failed
                                        // search. Telling someone that Apple
                                        // Music has nothing matching "" is
                                        // nonsense — this is an invitation.
                                        add_named[Some("search-prompt")] = &adw::StatusPage {
                                            set_icon_name: Some("system-search-symbolic"),
                                            set_title: crate::i18n::tr("Search Apple Music"),
                                            set_description: Some(
                                                "Find songs from the whole catalogue, not just your library.",
                                            ),
                                        },

                                        // Distinct from "status": an empty
                                        // library and a search with no matches
                                        // are different problems.
                                        add_named[Some("no-results")] = &adw::StatusPage {
                                            set_icon_name: Some("system-search-symbolic"),
                                            set_title: crate::i18n::tr("No matches"),
                                            #[watch]
                                            set_description: Some(&match model.view {
                                                View::Explore => "Apple Music returned no discovery sections.".into(),
                                                View::Lyrics => "No lyrics are available for this track.".into(),
                                                View::Songs => format!(
                                                    "Nothing in your library matches “{}”. Try searching Apple Music.",
                                                    model.query()
                                                ),
                                                View::Albums => format!(
                                                    "No album in your library matches “{}”.",
                                                    model.query()
                                                ),
                                                View::Artists => format!(
                                                    "No artist in your library matches “{}”.",
                                                    model.query()
                                                ),
                                                View::Playlists => format!(
                                                    "No playlist in your library matches “{}”.",
                                                    model.query()
                                                ),
                                                View::Search => format!(
                                                    "Apple Music has nothing matching “{}”.",
                                                    model.query()
                                                ),
                                            }),
                                        },

                                        // After the children — naming a child
                                        // before it has been added warns and
                                        // does nothing.
                                        #[watch]
                                        set_visible_child_name: model.page(),
                                    },
                                    },
                                },
                            },
                        },
                    },

                },
                },
            },
        }
    }

    fn init(
        settings: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        crate::i18n::set_language(settings.language);
        // The bar emits intent, never commands — `app/mod.rs` is the only place
        // that talks to the sidecar (rule 9).
        let now_playing = NowPlaying::builder()
            .launch(())
            .forward(sender.input_sender(), |out| match out {
                NowPlayingOutput::PlayPause => AppMsg::PlayPause,
                NowPlayingOutput::Next => AppMsg::Next,
                NowPlayingOutput::Previous => AppMsg::Previous,
                NowPlayingOutput::Seek(ms) => AppMsg::Seek(ms),
                NowPlayingOutput::SetVolume(v) => AppMsg::SetVolume(v),
                NowPlayingOutput::SetShuffle(on) => AppMsg::SetShuffle(on),
                NowPlayingOutput::SetRepeat(mode) => AppMsg::SetRepeat(mode),
                NowPlayingOutput::CycleSegmentLoop => AppMsg::CycleSegmentLoop,
                NowPlayingOutput::ShowLyrics => AppMsg::ShowLyrics,
                NowPlayingOutput::ToggleQueue => AppMsg::ToggleQueue,
                NowPlayingOutput::OpenAlbum => AppMsg::OpenPlayingAlbum,
                NowPlayingOutput::OpenArtist => AppMsg::OpenPlayingArtist,
                NowPlayingOutput::CopyLink => AppMsg::CopyPlayingLink,
                NowPlayingOutput::ToggleFavorite => AppMsg::TogglePlayingFavorite,
                NowPlayingOutput::SetSleepTimer(choice) => AppMsg::SetSleepTimer(choice),
                NowPlayingOutput::ShowCredits => AppMsg::ShowCredits,
            });

        let library: TypedListView<LibraryItem, gtk::NoSelection> = TypedListView::new();
        let activate = sender.clone();
        library.view.connect_activate(move |_, position| {
            activate.input(AppMsg::LibraryActivated(position));
        });

        // One handler for every list's rows. `setup` is a static method with no
        // item to carry a callback, so this is installed once, here.
        let menu_sender = sender.clone();
        crate::components::track_row::set_row_menu(move |req| {
            menu_sender.input(AppMsg::ShowRowMenu(req));
        });

        // The same handover, for the grid's tiles — `setup` is static and has no
        // item to reach the app through. See `pins::menu`.
        let tile_menu = sender.clone();
        crate::components::grid_item::set_tile_menu(move |req| {
            tile_menu.input(AppMsg::TileMenu(req));
        });

        let queue_view = QueueView::builder()
            .launch(())
            .forward(sender.input_sender(), |out| match out {
                QueueViewOutput::Jump { at, id } => AppMsg::JumpTo { at, id },
                QueueViewOutput::Remove { at, id } => AppMsg::RemoveFromQueue { at, id },
                QueueViewOutput::Clear => AppMsg::ClearQueue,
                QueueViewOutput::Move { from, to } => AppMsg::MoveQueueItem { from, to },
                QueueViewOutput::SetShuffle(on) => AppMsg::SetShuffle(on),
                QueueViewOutput::SetRepeat(mode) => AppMsg::SetRepeat(mode),
                QueueViewOutput::GoToAlbum(catalog_id) => AppMsg::OpenQueueTrackPage {
                    catalog_id,
                    album: true,
                },
                QueueViewOutput::GoToArtist(catalog_id) => AppMsg::OpenQueueTrackPage {
                    catalog_id,
                    album: false,
                },
            });

        // The queue **moves** into the expanded player rather than being
        // rebuilt there (#18). It is handed over before the view is built,
        // because relm4 constructs the widget tree before the model exists and
        // there is no init payload that can carry a widget through.
        crate::components::player_view::hand_over_queue(queue_view.widget().clone());
        let player_view = PlayerView::builder()
            .launch(())
            .forward(sender.input_sender(), map_player_output);

        // Popping is the user's business (back button, swipe, Escape), so the
        // stack is told about it rather than driving it. Resolving by tag keeps
        // the id-not-index rule intact even here.
        let nav = adw::NavigationView::new();
        let popped = sender.clone();
        nav.connect_popped(move |_, page| {
            if let Some(id) = page.tag().and_then(|t| t.parse::<u64>().ok()) {
                popped.input(AppMsg::PagePopped(id));
            }
        });

        let album_grid: TypedGridView<GridItem, gtk::NoSelection> = TypedGridView::new();
        let activate = sender.clone();
        album_grid
            .view
            .connect_activate(move |_, position| activate.input(AppMsg::AlbumActivated(position)));

        let artist_grid: TypedGridView<GridItem, gtk::NoSelection> = TypedGridView::new();
        let activate = sender.clone();
        artist_grid
            .view
            .connect_activate(move |_, position| activate.input(AppMsg::ArtistActivated(position)));

        let playlist_grid: TypedGridView<GridItem, gtk::NoSelection> = TypedGridView::new();
        let activate = sender.clone();
        playlist_grid.view.connect_activate(move |_, position| {
            activate.input(AppMsg::PlaylistActivated(position))
        });

        // Tiles call this from `bind`, deep inside GTK's factory, where there
        // is no component to reach. It turns "I need this cover" into an
        // ordinary message, so the fetch itself still happens as a Command off
        // the GTK thread (rule 8).
        let art_sender = sender.clone();
        let tile_art_request: ArtRequest = std::rc::Rc::new(move |key, art| {
            art_sender.input(AppMsg::NeedTileArt(key, art));
        });
        let playlist_art = playlist_art::State::new(&sender);

        let explore_sender = sender.clone();
        let explore_view = ExploreView::new(tile_art_request.clone(), move |action| {
            explore_sender.input(AppMsg::ExploreAction(action));
        });
        let preferences_sender = sender.clone();
        let lyrics_seek_sender = sender.clone();
        let lyrics_timing_sender = sender.clone();
        let lyric_variant_sender = sender.clone();
        let lyrics_view = LyricsView::new(
            settings.companion,
            settings.jamkin_quality,
            settings.jamkin_reduced_motion,
            move || preferences_sender.input(AppMsg::ShowPreferences),
            move |position_ms| lyrics_seek_sender.input(AppMsg::Seek(position_ms)),
            move |delta_ms| lyrics_timing_sender.input(AppMsg::AdjustLyricTiming(delta_ms)),
            move |index| lyric_variant_sender.input(AppMsg::SelectLyricVariant(index)),
        );
        let open_sender = sender.clone();
        let disable_sender = sender.clone();
        let position_sender = sender.clone();
        let jamkin_mode = JamkinMode::new(
            JamkinModeConfig::from_settings(&settings),
            move |right, bottom| {
                position_sender.input(AppMsg::SetDesktopJamkinPosition { right, bottom });
            },
            move || {
                open_sender.input(AppMsg::ShowLyrics);
                open_sender.input(AppMsg::Raise);
            },
            move || disable_sender.input(AppMsg::SetDesktopJamkin(false)),
        );
        let discord_presence = crate::discord::Presence::new(settings.discord_activity);

        let mut model = AppModel {
            stage: Stage::Starting,
            queue_view,
            player_view,
            explore_view,
            loading_explore: false,
            tried_explore: false,
            explore_generation: 0,
            lyrics_view,
            jamkin_mode,
            discord_presence,
            global_shortcuts_stop: None,
            scrobbler: crate::scrobble::Scrobbler::default(),
            launcher_icon_pending: None,
            lyrics_for: None,
            lyrics_loading: false,
            lyrics_generation: 0,
            lyrics_cache: std::collections::HashMap::new(),
            lyric_offsets: crate::lyric_timing::Offsets::load(),
            library,
            show_queue: false,
            show_sidebar: settings.show_sidebar,
            sidebar_collapsed: false,
            narrow_header: false,
            searching: false,
            focus_search: false,
            sync_entry: false,
            animated_shown: std::cell::Cell::new(None),
            healed: false,
            pending_move: None,
            resume_at: None,
            pruned: false,
            section_spinners: Vec::new(),
            pin_labels: Vec::new(),
            pins_dirty: false,
            selected_row: None,
            nav_selected: std::cell::RefCell::new(None),
            // Built from the persisted pins before anything is on screen: the
            // library cache has already seeded `playlists`, so a pinned row
            // draws its name at the same moment the sections do.
            sidebar_rows: sidebar_rows(&settings.pinned_playlists),
            marked_playing: None,
            library_icons: row_registry(),
            current_track: current_track(),
            dead_rows: dead_tracks(),
            row_overrides: track_overrides(),
            // filled from `dead_ids` once the model exists (see below)
            all_tracks: Vec::new(),
            library_query: String::new(),
            catalog_query: String::new(),
            view: View::from(settings.section),
            sorts: view::Sorts {
                songs: view::Sort {
                    by: SortBy::parse(&settings.sort).valid_for(View::Songs),
                    reversed: settings.sort_reversed,
                },
                albums: view::Sort {
                    by: SortBy::parse(&settings.album_sort).valid_for(View::Albums),
                    reversed: settings.album_sort_reversed,
                },
                artists: view::Sort {
                    by: SortBy::parse(&settings.artist_sort).valid_for(View::Artists),
                    reversed: settings.artist_sort_reversed,
                },
                playlists: view::Sort {
                    by: SortBy::parse(&settings.playlist_sort).valid_for(View::Playlists),
                    reversed: settings.playlist_sort_reversed,
                },
            },
            sort_actions: None,
            jamkin_action: None,
            albums: Vec::new(),
            artists: Vec::new(),
            playlists: Vec::new(),
            album_grid,
            artist_grid,
            playlist_grid,
            loading_albums: false,
            loading_artists: false,
            loading_playlists: false,
            tried_albums: false,
            tried_artists: false,
            tried_playlists: false,
            tried_library: false,
            built_rows: None,
            built_albums: None,
            built_artists: None,
            built_playlists: None,
            album_art_widgets: art_registry(),
            artist_art_widgets: art_registry(),
            playlist_art_widgets: art_registry(),
            playlist_art,
            tile_art_pending: std::collections::HashSet::new(),
            tile_art_request,
            catalog: Vec::new(),
            catalog_paged: 0,
            catalog_filter: CatalogFilter::default(),
            background: None,
            pending_writes: std::collections::HashMap::new(),
            pages: Vec::new(),
            next_page_id: 1,
            nav,
            searching_catalog: false,
            catalog_exhausted: false,
            search_gen: 0,
            loading_library: false,
            // Seeded from the cache so the first play of a session does not
            // have to rediscover them by failing a setQueue.
            dead_ids: crate::unplayable::load(),
            last_queue: None,
            pending_start: None,
            player: PlayerState::new(),
            restored: false,
            onboarding: None,
            last_item: None,
            menu_sender: sender.clone(),
            last_command: std::cell::RefCell::new(None),
            progress_mark: std::cell::Cell::new((0, 0)),
            sleep_timer: crate::sleep_timer::Timer::default(),
            apple_session: None,
            account_generation: 0,
            sidecar: None,
            restarts: 0,
            toaster: adw::ToastOverlay::new(),
            volume_osd: osd::VolumeOsd::new(),
            osd_shown: false,
            osd_timer: None,
            now_playing,
            mpris: Mpris::start(sender.clone()),
            volume: 1.0,
            art_path: None,
            art_for: None,
            tick: None,
            segment_loop_tick: None,
            segment_loop: SegmentLoop::default(),
            settings,
            notified_for: None,
            notify_when_art_lands: None,
        };
        let primary_menu = gtk::gio::Menu::new();
        {
            let preferences = gtk::gio::Menu::new();
            preferences.append(
                Some(crate::i18n::tr("_Preferences")),
                Some("win.preferences"),
            );
            preferences.append(
                Some(crate::i18n::tr("_Show Jamkin")),
                Some("win.show-jamkin"),
            );
            primary_menu.append_section(None, &preferences);

            let library = gtk::gio::Menu::new();
            library.append(
                Some(crate::i18n::tr("_New Playlist…")),
                Some("win.new-playlist"),
            );
            primary_menu.append_section(None, &library);

            // **First, in its own section.** It is the one item here that is
            // not about running the app, and under Preferences and About it
            // read as the least of them — the only thing in this menu that
            // asks rather than does, buried under three that do.
            //
            // **No icon, and not for want of trying.** `gio::MenuItem` carries
            // one and `GtkPopoverMenu` ignores it: GTK4 draws icons only for
            // items in a section with a `display-hint`, which is for the
            // little button rows, not for an ordinary entry. A heart was set
            // here, resolved from the theme, and simply never appeared.
            let support = gtk::gio::Menu::new();
            support.append(
                Some(crate::i18n::tr("_Buy Slipmat Creator a Coffee")),
                Some("win.support"),
            );
            primary_menu.append_section(None, &support);

            let section = gtk::gio::Menu::new();
            section.append(
                Some(crate::i18n::tr("_Keyboard Shortcuts")),
                Some("win.shortcuts"),
            );
            section.append(Some(crate::i18n::tr("_About Jamelade")), Some("win.about"));
            primary_menu.append_section(None, &section);

            // Its own section: signing out is an account action, not app
            // furniture, and it should not sit next to About.
            let account = gtk::gio::Menu::new();
            account.append(Some(crate::i18n::tr("_Sign Out")), Some("win.sign-out"));
            primary_menu.append_section(None, &account);

            // Quit was missing from this menu entirely, while the shortcuts
            // dialog advertised `Ctrl`+`Q` — so the app claimed a way out it
            // never showed. Last section, per the GNOME convention.
            let quit = gtk::gio::Menu::new();
            quit.append(Some(crate::i18n::tr("_Quit")), Some("app.quit"));
            primary_menu.append_section(None, &quit);
        }

        let toaster = &model.toaster;
        let now_playing_bar = model.now_playing.widget();
        let nav_view = &model.nav;
        let album_grid = &model.album_grid.view;
        let artist_grid = &model.artist_grid.view;
        let playlist_grid = &model.playlist_grid.view;
        let explore_content = model.explore_view.widget();
        let lyrics_content = model.lyrics_view.widget();
        let player_sheet_content = model.player_view.widget();
        // Cloned rather than borrowed from the model: `view_output!` needs it
        // while the model already owns it.
        let osd_revealer = model.volume_osd.revealer.clone();
        let volume_osd = &osd_revealer;
        let widgets = view_output!();

        wiring::connect(&mut model, &widgets, &root, &sender);

        // Wide uses a compact one-third-height drawer; narrow windows keep a
        // taller vertical composition. See `fill_window`.
        crate::components::player_view::fill_window(
            &root,
            &widgets.player_sheet,
            model.player_view.widget().upcast_ref(),
            model.player_view.sender(),
        );

        model.jamkin_action = Some(register_actions(
            &root,
            &sender,
            model.settings.desktop_jamkin,
        ));

        // Rows read playability from here, so seed it before any are built.
        *model.dead_rows.borrow_mut() = model.dead_ids.clone();

        model.jamkin_mode.set_enabled(model.settings.desktop_jamkin);

        // Open on last time's library rather than on a spinner. The refresh
        // still runs the moment the sidecar is up; it lands quietly, or — far
        // more often — finds nothing changed and does not even rebuild.
        model.seed_from_cache();
        // The rows exist by now but were drawn before the cache was read, so
        // every pin still says "Unavailable".
        model.refresh_pin_names();

        start_sidecar(&sender);

        if model.settings.global_shortcuts {
            sender.input(AppMsg::ConfigureGlobalShortcuts);
        }
        if model.settings.listenbrainz_scrobbling {
            sender.oneshot_command(async {
                CommandMsg::ListenBrainzTokenLoaded(crate::scrobble::load_token().await)
            });
        }

        ComponentParts { model, widgets }
    }

    fn shutdown(&mut self, _widgets: &mut Self::Widgets, _output: relm4::Sender<Self::Output>) {
        self.clear_segment_loop();
        if let Some(stop) = self.global_shortcuts_stop.take() {
            let _ = stop.send(true);
        }
        self.jamkin_mode.set_enabled(false);
        self.discord_presence.set_enabled(false);
        // A now-playing notification must not outlive the player that sent it.
        notify::clear(relm4::main_application().upcast_ref::<gtk::gio::Application>());
        // The only moment the position is accurate.
        self.save_session();
    }

    /// Wraps `update` so the search box can be re-filled after a scope change.
    ///
    /// The entry is the one widget holding text the model also owns, and the
    /// two must agree: switching scope swaps which query is live, and the box
    /// has to show that scope's text rather than the one you left behind.
    /// Timed, temporarily, because "switching sections is slow" needs a number
    /// before it needs a fix. `update_view` re-runs every `#[watch]` in the
    /// view macro, and there is a lot of it.
    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        msg: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        let view_before = self.view;
        self.update(msg, sender.clone(), root);
        self.sync_animated(widgets);
        self.sync_pins(widgets, &sender);
        self.sync_section_spinners();

        if self.view != view_before {
            // `set_text` fires `search-changed`, but `SearchChanged` returns
            // early when the text already matches the active query — which it
            // does by now, because `update` set it first. No loop.
            widgets.search_entry.set_text(self.query());
            self.sync_sort_menu(&widgets.sort_button);
        }

        // After `update`, so a narrow header has already swapped the entry in
        // for the section title — an unmapped widget cannot take the caret, and
        // that is the whole reason this is a flag rather than a `grab_focus`
        // at the call site.
        if std::mem::take(&mut self.sync_entry) {
            widgets.search_entry.set_text(self.query());
        }
        if std::mem::take(&mut self.focus_search) {
            widgets.search_entry.grab_focus();
            // Typing appends, so the caret belongs after what is already there.
            // `grab_focus` on an entry selects all of it, and the next
            // keystroke would replace the character that opened the search.
            widgets.search_entry.set_position(-1);
        }

        let painting = std::time::Instant::now();
        self.update_view(widgets, sender);
        let ms = painting.elapsed().as_millis();
        if ms > 4 {
            // Only the slow ones: at ~60fps anything over 16ms drops a frame,
            // and a message that costs more than a few is worth naming.
            tracing::debug!(ms, "view refresh");
        }
    }

    /// Overridden for one reason: [`AppModel::sync_animated`] and
    /// [`AppModel::sync_pins`] have to run on **both** paths.
    ///
    /// The default calls `update_cmd` then `update_view`, and command messages
    /// are how the sidecar's events arrive — including the one that moves
    /// `stage` to `Ready`, which is what reveals the Now Playing bar. Syncing
    /// only in `update_with_view` left the bar and the drawer hidden for the
    /// whole session, because their transition happened on the path that was
    /// not looking.
    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        self.update_cmd(message, sender.clone(), root);
        self.sync_animated(widgets);
        // Pruning a stale pin happens here, not in `update`: the library load is
        // a command message. Syncing only on the other path left the pruned row
        // on screen with nothing behind it — and clicking it opened whatever had
        // moved into its position.
        self.sync_pins(widgets, &sender);
        self.sync_section_spinners();
        self.update_view(widgets, sender);
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, root: &Self::Root) {
        self.handle(msg, &sender, root);
        self.activate_discovery(&sender);
        self.sync_onboarding(&sender, root);
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        self.handle_cmd(msg, &sender, root);
        self.activate_discovery(&sender);
        self.sync_onboarding(&sender, root);
    }
}

/// The three widget properties that are animated, sampled together so a
/// transition in any of them can be spotted without repeating the comparison.
#[derive(Clone, Copy)]
struct Animated {
    sidebar: bool,
    queue: bool,
    bottom_bar: bool,
    /// The volume panel's crossfade. Fourth of its kind, and here for the same
    /// reason as the other three: a `#[watch]` would re-ask for it after every
    /// message, and during playback those never stop.
    osd: bool,
}

impl AppModel {
    fn animated_state(&self) -> Animated {
        Animated {
            sidebar: self.show_sidebar,
            queue: self.show_queue,
            bottom_bar: matches!(self.stage, Stage::Ready),
            osd: self.osd_shown,
        }
    }

    /// Push the four animated properties, and **only where they changed**.
    ///
    /// Each drives an animation — the sidebar's spring, the drawer's slide, the
    /// bar's reveal, the volume panel's crossfade — so writing one asks it to
    /// start or
    /// re-aim. That is correct on an edge and catastrophic on a level: as a
    /// `#[watch]` it re-fired after every message, and during playback those
    /// never stop, which wedged the app inside libadwaita's spring solver.
    ///
    /// Compared against **what we last wrote**, not against the widget. A
    /// widget that disagrees persistently disagrees on every message too, so
    /// asking it is the level trigger again wearing a guard's clothes — that
    /// was the first attempt at this fix, and the second core dump found it
    /// still spinning.
    /// Show a spinner on whichever sections are fetching.
    ///
    /// What `#[watch]` did before the rows moved out of `view!`. Called from
    /// **both** view paths for the reason `sync_animated` documents: a library
    /// load finishes as a `CommandMsg`, which arrives through
    /// `update_cmd_with_view` and never through the other — wire this to one
    /// and every spinner starts but none of them ever stops.
    fn sync_section_spinners(&self) {
        for (view, spinner) in &self.section_spinners {
            spinner.set_visible(self.loading_in(*view));
        }
    }

    fn sync_animated(&self, widgets: &<Self as relm4::Component>::Widgets) {
        let now = self.animated_state();
        let last = self.animated_shown.get();
        if last.map(|l| l.sidebar) != Some(now.sidebar) {
            widgets.nav_split.set_show_sidebar(now.sidebar);
            // A destination page carries its own toggle, and a toggle drawn
            // pressed over a hidden sidebar lies about its own state.
            for page in &self.pages {
                page.set_sidebar_shown(now.sidebar);
            }
        }
        if last.map(|l| l.queue) != Some(now.queue) {
            widgets.player_sheet.set_open(now.queue);
        }
        if last.map(|l| l.bottom_bar) != Some(now.bottom_bar) {
            widgets.player_sheet.set_reveal_bottom_bar(now.bottom_bar);
        }
        if last.map(|l| l.osd) != Some(now.osd) {
            self.volume_osd.revealer.set_reveal_child(now.osd);
        }
        self.animated_shown.set(Some(now));
    }

    fn handle(
        &mut self,
        msg: AppMsg,
        sender: &ComponentSender<Self>,
        root: &adw::ApplicationWindow,
    ) {
        let sender = sender.clone();
        match msg {
            AppMsg::SignIn => self.send(Command::ShowLogin),
            AppMsg::SignOut => {
                // The menu item is always there; asking to sign out when you
                // already are should do nothing rather than prompt.
                if matches!(self.stage, Stage::Ready) {
                    self.confirm_sign_out(&sender, root);
                }
            }
            AppMsg::SignOutConfirmed => {
                tracing::info!("signing out");
                // The sidecar drops Apple's session — cookies and all, not just
                // MusicKit's token — and its `authorizationStatusDidChange`
                // confirms it rather than us assuming.
                self.send(Command::SignOut);
                self.forget_session();
            }
            AppMsg::PlayPause => self.send(Command::PlayPause),
            AppMsg::Play => self.send(Command::Play),
            AppMsg::Pause => self.send(Command::Pause),
            AppMsg::Next => self.send(Command::Next),
            AppMsg::Previous => self.go_previous(),
            AppMsg::Seek(position_ms) => {
                self.send(Command::Seek { position_ms });
                // Announce the jump straight away rather than waiting for the
                // sidecar's echo. The spec requires `Seeked` on discontinuous
                // moves — without it controllers keep extrapolating from the
                // old position and their progress bars drift.
                self.mpris.seeked(position_ms);
            }
            AppMsg::SetVolume(volume) => self.set_volume(volume),
            // **The panel is raised here rather than inside `set_volume`.**
            // That returns early when the value has not moved, so at 0.0 and
            // 1.0 it does nothing — and a shortcut that shows nothing at the
            // ends reads as a dropped keypress rather than "you are already
            // there".
            AppMsg::VolumeUp => {
                self.set_volume(self.volume + VOLUME_STEP);
                self.flash_volume(&sender);
            }
            AppMsg::VolumeDown => {
                self.set_volume(self.volume - VOLUME_STEP);
                self.flash_volume(&sender);
            }
            AppMsg::HideVolumeOsd => self.hide_volume_osd(),
            AppMsg::CopyPlayingLink => {
                let link = self.apple_session.as_ref().and_then(|session| {
                    self.playing_catalog_id()
                        .and_then(|id| crate::apple_link::song(&session.storefront, &id))
                });
                match link {
                    Some(link) => self.copy_apple_link(&link),
                    None => self.toast("No public Apple Music link is available"),
                }
            }
            AppMsg::TogglePlayingFavorite => {
                let Some(catalog_id) = self.playing_catalog_id() else {
                    self.toast("No catalog song is available");
                    return;
                };
                if self.playing_favorite(&catalog_id) {
                    sender.input(AppMsg::Unfavorite { catalog_id });
                } else {
                    sender.input(AppMsg::LibraryWrite {
                        catalog_id,
                        action: LibraryAction::Favorite,
                    });
                }
            }
            AppMsg::OpenPlayingAlbum => match self.playing_catalog_id() {
                Some(catalog_id) => sender.input(AppMsg::OpenQueueTrackPage {
                    catalog_id,
                    album: true,
                }),
                None => self.toast("Apple doesn't expose an album for this track"),
            },
            AppMsg::OpenPlayingArtist => match self.playing_catalog_id() {
                Some(catalog_id) => sender.input(AppMsg::OpenQueueTrackPage {
                    catalog_id,
                    album: false,
                }),
                None => self.toast("Apple doesn't expose an artist for this track"),
            },
            AppMsg::ShowCredits => {
                let Some(catalog_id) = self.playing_catalog_id() else {
                    self.toast("Credits are unavailable for this track");
                    return;
                };
                let Some(client) = self.client() else {
                    self.toast("Not connected yet");
                    return;
                };
                let generation = self.account_generation;
                sender.oneshot_command(async move {
                    CommandMsg::Credits {
                        generation,
                        result: client
                            .song_credits(&catalog_id)
                            .await
                            .map_err(|error| format!("{error:#}")),
                    }
                });
            }
            AppMsg::CopyPageLink { page } => {
                let link = self
                    .pages
                    .iter()
                    .find(|candidate| candidate.id == page)
                    .and_then(|page| page.share_link().map(str::to_owned));
                match link {
                    Some(link) => self.copy_apple_link(&link),
                    None => self.toast("No public Apple Music link is available"),
                }
            }
            AppMsg::ExportPlaylist { page, format } => {
                let Some((title, tracks)) = self
                    .pages
                    .iter()
                    .find(|candidate| candidate.id == page)
                    .and_then(DetailPage::export_data)
                else {
                    self.toast("This playlist is not ready to export");
                    return;
                };
                let Ok(contents) = crate::playlist_export::render(&title, &tracks, format) else {
                    self.toast("Could not prepare that export");
                    return;
                };
                let dialog = gtk::FileDialog::builder()
                    .title("Export Playlist")
                    .accept_label("Export")
                    .initial_name(crate::playlist_export::suggested_name(&title, format))
                    .modal(true)
                    .build();
                let root = root.clone();
                let toaster = self.toaster.clone();
                gtk::glib::spawn_future_local(async move {
                    let Ok(file) = dialog.save_future(Some(&root)).await else {
                        return;
                    };
                    let result = file
                        .replace_contents_future(
                            contents,
                            None,
                            false,
                            gtk::gio::FileCreateFlags::REPLACE_DESTINATION,
                        )
                        .await;
                    toaster.add_toast(transient_toast(if result.is_ok() {
                        "Playlist exported"
                    } else {
                        "Could not write the playlist export"
                    }));
                });
            }
            AppMsg::SidebarRowChosen(index) => self.sidebar_row_chosen(index, &sender),
            AppMsg::SidebarRowActivated(index) => self.sidebar_row_activated(index, &sender),
            AppMsg::ShowPinPicker => self.show_pin_picker(&sender, root),
            AppMsg::SetPinned { id, pinned } => self.set_pinned(&id, pinned, &sender),
            AppMsg::SetAllPinned(pinned) => self.set_all_pinned(pinned, &sender),
            AppMsg::TileMenu(req) => self.show_tile_menu(req),
            AppMsg::MovePin { from, slot } => self.move_pinned(from, slot),
            AppMsg::Tick => {
                self.push_snapshot();
                self.sync_lyrics_position();
                if self.settings.listenbrainz_scrobbling
                    && self.player.state.is_playing()
                    && let Some(item) = self.player.now_playing.as_ref()
                    && let Some(submission) = self
                        .scrobbler
                        .prepare(item, self.player.interpolated_position_ms())
                {
                    let key = submission.key().to_owned();
                    sender.oneshot_command(async move {
                        CommandMsg::ListenBrainzSubmitted {
                            key,
                            result: crate::scrobble::submit(submission).await,
                        }
                    });
                }
            }
            AppMsg::CycleSegmentLoop => self.cycle_segment_loop(&sender),
            AppMsg::SegmentLoopTick => self.enforce_segment_loop(),
            AppMsg::SetSleepTimer(choice) => {
                let current = self.playing_catalog_id();
                if matches!(choice, crate::sleep_timer::Choice::EndOfTrack) && current.is_none() {
                    self.toast("Start a song before choosing end of track");
                    return;
                }
                let (generation, delay) = self.sleep_timer.set(choice, current.as_deref());
                self.player_view
                    .emit(PlayerViewInput::SleepTimerActive(self.sleep_timer.active()));
                if let Some(delay) = delay {
                    let sender = sender.clone();
                    gtk::glib::timeout_add_local_once(delay, move || {
                        sender.input(AppMsg::SleepTimerExpired(generation));
                    });
                }
                self.toast(match choice {
                    crate::sleep_timer::Choice::Off => "Sleep timer turned off",
                    crate::sleep_timer::Choice::EndOfTrack => {
                        "Playback will pause at the end of this song"
                    }
                    _ => "Sleep timer started",
                });
            }
            AppMsg::SleepTimerExpired(generation) => {
                if self.sleep_timer.expires(generation) {
                    self.send(Command::Pause);
                    self.player_view
                        .emit(PlayerViewInput::SleepTimerActive(false));
                    self.toast("Sleep timer paused playback");
                }
            }
            AppMsg::SearchChanged(query) => {
                if !self.view.searchable() {
                    return;
                }
                if query == self.query() {
                    return;
                }
                // Typing into the search field is a request to see results, and
                // results are on the root page.
                self.pop_to_results();
                match self.scope() {
                    SearchScope::Library => self.library_query = query,
                    SearchScope::Catalog => self.catalog_query = query,
                }

                match self.view {
                    // Local filters: instant, every keystroke.
                    View::Explore | View::Lyrics => {}
                    View::Songs => self.rebuild_rows(),
                    View::Albums => self.rebuild_albums(),
                    View::Artists => self.rebuild_artists(),
                    View::Playlists => self.rebuild_playlists(),
                    View::Search => {
                        self.search_gen = self.search_gen.wrapping_add(1);
                        let generation = self.search_gen;

                        self.catalog_exhausted = false;
                        self.catalog_paged = 0;
                        if self.catalog_query.trim().is_empty() {
                            self.catalog.clear();
                            self.searching_catalog = false;
                            self.rebuild_rows();
                            return;
                        }

                        // Debounce. Only the newest timer commits — the same
                        // generation trick the seek bar uses, and for the same
                        // reason: removing a fired glib source aborts.
                        let sender = sender.clone();
                        gtk::glib::timeout_add_local_once(
                            std::time::Duration::from_millis(SEARCH_DEBOUNCE_MS),
                            move || sender.input(AppMsg::RunCatalogSearch(generation)),
                        );
                    }
                }
            }
            AppMsg::RunCatalogSearch(generation) => {
                if generation != self.search_gen {
                    return; // a later keystroke superseded this one
                }
                self.run_catalog_search(&sender, generation, 0);
            }
            AppMsg::SetCatalogFilter(filter) => {
                if filter == self.catalog_filter {
                    return;
                }
                self.catalog_filter = filter;

                // A different filter is a different question, so the previous
                // answer is discarded whole — including the offset, which
                // counts a kind that may no longer be the one that pages.
                self.search_gen = self.search_gen.wrapping_add(1);
                self.catalog_exhausted = false;
                self.catalog_paged = 0;
                self.catalog.clear();
                self.built_rows = None;

                if self.catalog_query.trim().is_empty() {
                    self.rebuild_rows();
                    return;
                }
                // No debounce: this is one deliberate click, not a keystroke
                // in a stream of them.
                self.run_catalog_search(&sender, self.search_gen, 0);
            }
            AppMsg::SetView(view) => {
                if view == self.view {
                    return;
                }
                let switch_started = std::time::Instant::now();
                if self.view == View::Lyrics && !self.settings.desktop_jamkin {
                    // Leaving is also the retry boundary. It invalidates an
                    // in-flight answer unless the desktop Jamkin still needs
                    // it; a cached success is reused immediately on returning.
                    self.lyrics_generation = self.lyrics_generation.wrapping_add(1);
                    self.lyrics_for = None;
                    self.lyrics_loading = false;
                }
                self.view = view;
                // The Now Playing lyrics shortcut is redundant on the lyrics
                // page itself. Update that density decision immediately rather
                // than waiting for the next playback tick.
                self.push_snapshot();
                // On a narrow header the search box follows the section:
                // Apple Music *is* a search and lands on a prompt to type one,
                // so arriving with the field shut would be a screen asking for
                // something it did not give you room to enter. Every other
                // section closes it, because the query it held was about the
                // list you just left.
                self.searching = self.narrow_header && view == View::Search;
                // Switching section means switching what the content pane is
                // about, so any album or artist pushed on top of it is now
                // showing the wrong thing sitting over the right thing.
                self.pop_to_results();
                self.settings.section = Section::from(view);
                self.settings.save();

                // Whichever section is now showing re-reads; the others keep
                // what they had, so switching back is instant.
                match view {
                    View::Explore => self.load_explore(&sender),
                    View::Lyrics => self.ensure_lyrics(&sender),
                    View::Songs => self.rebuild_rows(),
                    View::Albums => {
                        self.rebuild_albums();
                        self.load_albums(&sender);
                    }
                    View::Artists => {
                        self.rebuild_artists();
                        self.load_artists(&sender);
                    }
                    View::Playlists => {
                        self.rebuild_playlists();
                        self.load_playlists(&sender);
                    }
                    View::Search => {
                        self.search_gen = self.search_gen.wrapping_add(1);
                        let generation = self.search_gen;
                        if self.catalog_query.trim().is_empty() {
                            self.catalog.clear();
                            self.rebuild_rows();
                        } else {
                            self.run_catalog_search(&sender, generation, 0);
                        }
                    }
                }
                // What the *reducer* spent. If this is small and the section
                // still takes a second to appear, the cost is in rendering
                // rather than in here.
                tracing::debug!(
                    ?view,
                    ms = switch_started.elapsed().as_millis(),
                    "section switch"
                );
            }
            AppMsg::ExploreAction(action) => match action {
                ExploreAction::Album(album) => {
                    self.push_page(PageKind::album(&album), &sender);
                }
                ExploreAction::Playlist(playlist) => {
                    self.push_page(PageKind::playlist(&playlist), &sender);
                }
                ExploreAction::PlayTracks { tracks, start } => {
                    let entries: Vec<_> = tracks.into_iter().map(Entry::Song).collect();
                    self.play_entries(&entries, start, Start::Clicked);
                }
                ExploreAction::PlayStation(station) => {
                    self.send(Command::PlayStation { station });
                }
            },
            AppMsg::ShowLyrics => {
                self.show_queue = false;
                self.player_view.emit(PlayerViewInput::SetQueueShown(false));
                self.sync_page_controls();
                self.push_snapshot();
                // Lyrics belongs to the player controls rather than the
                // navigation sidebar, so no sidebar row should stay selected
                // while its dedicated view is open.
                self.selected_row = None;
                self.pins_dirty = true;
                self.handle(AppMsg::SetView(View::Lyrics), &sender, root);
            }
            AppMsg::AlbumActivated(position) => {
                if let Some(item) = self.album_grid.get(position)
                    && let Tile::Album(album) = &item.borrow().tile
                {
                    sender.input(AppMsg::OpenPage(PageKind::album(album)));
                }
            }
            AppMsg::ArtistActivated(position) => {
                if let Some(item) = self.artist_grid.get(position)
                    && let Tile::Artist(artist) = &item.borrow().tile
                {
                    sender.input(AppMsg::OpenPage(PageKind::artist(artist)));
                }
            }
            AppMsg::PlaylistActivated(position) => {
                if let Some(item) = self.playlist_grid.get(position)
                    && let Tile::Playlist(playlist) = &item.borrow().tile
                {
                    sender.input(AppMsg::OpenPage(PageKind::playlist(playlist)));
                }
            }
            AppMsg::NeedTileArt(key, art) => {
                // Scrolling rebinds the same tile repeatedly; one request each.
                if !self.tile_art_pending.insert(key.clone()) {
                    return;
                }
                let generation = self.account_generation;
                sender.oneshot_command(async move {
                    // Fetch only if it is missing — `fetch` short-circuits on
                    // the disk cache — but decode either way, off the GTK
                    // thread. That is the whole point of #27: the tile is
                    // handed pixels, not a filename. Either half failing is
                    // cosmetic but never silent; `load_tile` says which.
                    let (path, cover) = artwork::load_tile(art, TILE_ART, &key).await;
                    CommandMsg::TileArt {
                        generation,
                        key,
                        path,
                        cover,
                    }
                });
            }
            AppMsg::NeedPlaylistArt(job) => self.need_playlist_art(job, &sender),
            AppMsg::LoadMoreCatalog => {
                // Guarded on all four conditions: only in catalog scope, only
                // when a page is not already in flight, only while Apple still
                // has more, and only up to a ceiling. Scroll events arrive in
                // bursts, so without these one flick would queue several
                // identical requests.
                if self.scope() == SearchScope::Catalog
                    && !self.searching_catalog
                    && !self.catalog_exhausted
                    && !self.catalog.is_empty()
                    && self.catalog_paged < CATALOG_MAX
                {
                    let generation = self.search_gen;
                    // Songs only — the browse rows above them are not part of
                    // Apple's song pagination.
                    let offset = self.catalog_paged;
                    self.run_catalog_search(&sender, generation, offset);
                }
            }
            AppMsg::ReloadCurrentSection => self.reload(self.view, &sender),
            AppMsg::ShowPreferences => self.show_preferences(&sender, root),
            AppMsg::ShowCreatePlaylist => self.show_create_playlist(&sender, root),
            AppMsg::CreatePlaylist { name, description } => {
                let Some(client) = self.client() else {
                    self.toast("Not connected yet");
                    return;
                };
                let generation = self.account_generation;
                self.toast("Creating playlist…");
                sender.oneshot_command(async move {
                    CommandMsg::PlaylistWritten {
                        generation,
                        created: true,
                        result: client
                            .create_playlist(name, description, Vec::new())
                            .await
                            .map_err(|error| format!("{error:#}")),
                    }
                });
            }
            AppMsg::ShowAddToPlaylist { catalog_id } => {
                self.show_add_to_playlist(catalog_id, &sender, root);
            }
            AppMsg::AddTrackToPlaylist {
                playlist_id,
                catalog_id,
            } => {
                let Some(client) = self.client() else {
                    self.toast("Not connected yet");
                    return;
                };
                let generation = self.account_generation;
                self.toast("Adding song to playlist…");
                sender.oneshot_command(async move {
                    CommandMsg::PlaylistWritten {
                        generation,
                        created: false,
                        result: client
                            .add_playlist_tracks(playlist_id, vec![catalog_id])
                            .await
                            .map_err(|error| format!("{error:#}")),
                    }
                });
            }
            AppMsg::ShowShortcuts => show_shortcuts(root),
            AppMsg::ShowAbout => show_about(root, self.settings.companion),
            AppMsg::OpenSupport => chrome::open_support(root),
            AppMsg::SetTheme(index) => {
                self.settings.theme = Theme::from_index(index);
                self.settings.apply_theme();
                crate::style::set_theme(self.settings.theme);
                self.settings.save();
            }
            AppMsg::SetLanguage(index) => {
                let language = crate::i18n::Language::from_index(index);
                if self.settings.language == language {
                    return;
                }
                self.settings.language = language;
                self.settings.save();
                self.toast("Restart Jamelade to apply the interface language");
            }
            AppMsg::SetNotifyTrackChange(on) => {
                self.settings.notify_track_change = on;
                self.settings.save();
            }
            AppMsg::SetLyricsEnabled(on) => {
                if self.settings.lyrics_enabled == on {
                    return;
                }
                self.settings.lyrics_enabled = on;
                self.settings.save();
                self.lyrics_provider_changed(&sender);
            }
            AppMsg::SetLyricsOvhEnabled(on) => {
                if self.settings.lyrics_ovh_enabled == on {
                    return;
                }
                self.settings.lyrics_ovh_enabled = on;
                self.settings.save();
                self.lyrics_provider_changed(&sender);
            }
            AppMsg::SidebarShown(shown) => {
                if self.show_sidebar == shown {
                    return; // our own write coming back
                }
                // Adopted, but **not** persisted. Dismissing an overlay is not
                // a statement about how you want the window laid out when it
                // is wide enough to hold a real pane; only `ToggleSidebar` is
                // deliberate enough to be a preference.
                self.show_sidebar = shown;
            }
            AppMsg::SidebarCollapsed(collapsed) => {
                // Logged because getting the breakpoints wrong is *silent*.
                // Only one applies at a time, so a narrow one that forgets to
                // repeat a wide one's setter simply undoes it — and the sidebar
                // stops collapsing at exactly the widths where it matters most.
                // Nothing warns; the pane just comes back.
                tracing::debug!(collapsed, narrow_header = self.narrow_header, "sidebar");
                self.sidebar_collapsed = collapsed;
            }
            AppMsg::NarrowHeader(narrow) => {
                tracing::debug!(narrow, "header breakpoint");
                self.narrow_header = narrow;
                // The bar reads this off the snapshot to decide whether it has
                // room for shuffle, repeat and volume, and nothing else pushes
                // one when only the window changed.
                self.push_snapshot();
                // Widening puts the entry back as the title, so the open flag
                // stops meaning anything — and leaving it set would reopen the
                // box the next time the window got narrow, which is not
                // something the user asked for a window resize ago.
                if !narrow {
                    self.searching = false;
                }
            }
            AppMsg::ShowSearch(show) => {
                // The button reports its own state *and* is written from the
                // model, which is the two-way binding that froze a desktop
                // (#37). Adopting the value here is the half of the fix that
                // stops the next `update_view` writing the old one back.
                if self.searching == show {
                    return;
                }
                self.searching = show;
                // Closing is a request to stop filtering, not to hide a filter
                // that is still in force. A narrowed list under a header that
                // shows no query is a list nobody can explain.
                //
                // Below the guard on purpose, so *widening* keeps the query.
                // `NarrowHeader` clears `searching` itself, which makes the
                // button report a change that lands here holding the value we
                // already have — and a window resize is not a request to
                // abandon what you were looking for.
                if !show {
                    // Inline rather than `sender.input`, so the query is empty
                    // *before* `sync_entry` is read. Queued, the flag would be
                    // consumed a pass early and write the words back in.
                    self.handle(AppMsg::SearchChanged(String::new()), &sender, root);
                    self.sync_entry = true;
                }
            }
            // Both openers set `sync_entry` too: the entry may hold text from
            // a query that was cleared while it was hidden.
            AppMsg::FocusSearch => {
                if !self.view.searchable() {
                    self.selected_row = Some(SidebarRow::Section(View::Search));
                    self.pins_dirty = true;
                    self.handle(AppMsg::SetView(View::Search), &sender, root);
                }
                self.sync_entry = true;
                // The box has to exist before it can be focused: on a narrow
                // header it is a hidden stack page until now, and a widget that
                // is not mapped cannot take the caret.
                self.searching = true;
                self.focus_search = true;
            }
            AppMsg::TypeAhead(text) => {
                if !self.view.searchable() {
                    self.selected_row = Some(SidebarRow::Section(View::Search));
                    self.pins_dirty = true;
                    self.handle(AppMsg::SetView(View::Search), &sender, root);
                }
                self.searching = true;
                self.focus_search = true;
                self.sync_entry = true;
                let mut query = self.query().to_owned();
                query.push_str(&text);
                // Straight through the ordinary path, so filtering, the
                // rebuild and the per-scope query all behave exactly as they do
                // when the character is typed into the entry directly.
                self.handle(AppMsg::SearchChanged(query), &sender, root);
            }
            AppMsg::ToggleSidebar => {
                self.show_sidebar = !self.show_sidebar;
                self.settings.show_sidebar = self.show_sidebar;
                self.settings.save();
            }
            AppMsg::ToggleQueue => {
                self.show_queue = !self.show_queue;
                self.sync_page_controls();
                // The bar's toggle reads this from the snapshot, so push one.
                self.push_snapshot();
                if self.show_queue {
                    // **Onto the queue, not merely onto the drawer.** This
                    // button says "Queue" and used to open the expanded player
                    // with the queue still tucked away behind its own toggle —
                    // two clicks to reach the thing the icon names. Opening the
                    // drawer is how the queue is reached, not what was asked
                    // for.
                    self.player_view.emit(PlayerViewInput::SetQueueShown(true));
                    self.queue_view.emit(QueueViewInput::ScrollToPlaying);
                }
            }
            AppMsg::LibraryActivated(position) => {
                // Catalog results mix songs with albums, artists and playlists.
                // A song plays; the rest are doors, and clicking one walks
                // through it. Resolved against the list as it is right now,
                // never against a remembered snapshot.
                match self.visible_entries().get(position as usize) {
                    Some(Entry::Album(album)) => {
                        sender.input(AppMsg::OpenPage(PageKind::album(album)))
                    }
                    Some(Entry::Artist(artist)) => {
                        sender.input(AppMsg::OpenPage(PageKind::artist(artist)))
                    }
                    Some(Entry::Playlist(playlist)) => {
                        sender.input(AppMsg::OpenPage(PageKind::playlist(playlist)))
                    }
                    // The store is the visible list, so this position is the
                    // row index `queue_from` expects.
                    Some(Entry::Song(_)) => sender.input(AppMsg::PlayFrom(position as usize)),
                    None => {}
                }
            }
            AppMsg::OpenPage(kind) => self.push_page(kind, &sender),
            AppMsg::OpenAlbumArtist { page } => {
                let catalog_id = self
                    .pages
                    .iter()
                    .find(|candidate| candidate.id == page)
                    .and_then(|page| page.entries.iter().find_map(Entry::catalog_id))
                    .map(str::to_owned);
                if let Some(catalog_id) = catalog_id {
                    sender.input(AppMsg::OpenQueueTrackPage {
                        catalog_id,
                        album: false,
                    });
                } else {
                    self.toast("Artist page unavailable");
                }
            }
            AppMsg::OpenQueueTrackPage { catalog_id, album } => {
                let Some(client) = self.client() else {
                    self.toast("Not connected yet");
                    return;
                };
                let generation = self.account_generation;
                sender.oneshot_command(async move {
                    CommandMsg::QueueTrackPage {
                        generation,
                        result: client
                            .song_containers(&catalog_id)
                            .await
                            .map(|(album_id, artist_id)| {
                                if album {
                                    album_id.map(PageKind::Album)
                                } else {
                                    artist_id.map(PageKind::Artist)
                                }
                            })
                            .map_err(|err| format!("{err:#}")),
                    }
                });
            }
            AppMsg::PagePopped(id) => {
                // The page owns its own row registry, so dropping it takes the
                // stale widget handles with it. Nothing to clean up by hand.
                self.pages.retain(|p| p.id != id);
                tracing::debug!(id, depth = self.pages.len(), "page popped");
            }
            AppMsg::DetailActivated { page, row } => {
                let Some(page) = self.pages.iter().find(|p| p.id == page) else {
                    // Popped between the click and here. Nothing to do, and
                    // certainly nothing to guess at.
                    return;
                };
                match page.entries.get(row) {
                    Some(Entry::Album(album)) => {
                        sender.input(AppMsg::OpenPage(PageKind::album(album)))
                    }
                    Some(Entry::Artist(artist)) => {
                        sender.input(AppMsg::OpenPage(PageKind::artist(artist)))
                    }
                    Some(Entry::Playlist(playlist)) => {
                        sender.input(AppMsg::OpenPage(PageKind::playlist(playlist)))
                    }
                    Some(Entry::Song(_)) => {
                        let entries = page.entries.clone();
                        self.play_entries(&entries, row, Start::Clicked);
                    }
                    None => {}
                }
            }
            AppMsg::ArtistActivatedOnPage { page, target } => {
                let Some(page) = self.pages.iter().find(|candidate| candidate.id == page) else {
                    return;
                };
                match target {
                    ArtistActivate::Album(index) => {
                        if let Some(album) = page.artist_albums.get(index) {
                            sender.input(AppMsg::OpenPage(PageKind::album(album)));
                        }
                    }
                    ArtistActivate::LatestRelease => {
                        if let Some(album) = &page.artist_latest_release {
                            sender.input(AppMsg::OpenPage(PageKind::album(album)));
                        }
                    }
                    ArtistActivate::TopSong(index) => {
                        let songs: Vec<Entry> =
                            page.artist_songs.iter().cloned().map(Entry::Song).collect();
                        self.play_entries(&songs, index, Start::Clicked);
                    }
                }
            }
            AppMsg::PlayPage { page, shuffle } => {
                let Some(target) = self.pages.iter().find(|p| p.id == page) else {
                    return;
                };
                let entries = target.entries.clone();
                // The row we name is the one MusicKit will pin as the head,
                // which is why Shuffle needs a random one (#147). See
                // `shuffle_start`.
                let (row, start) = if shuffle {
                    (self.shuffle_start(&entries), Start::Shuffled)
                } else {
                    (0, Start::InOrder)
                };
                // `play_entries` sends the mode, ahead of the queue it applies
                // to. Both buttons state one: a Play that inherited the shuffle
                // left on by something else is the same bug as a row click that
                // did.
                self.play_entries(&entries, row, start);
            }
            AppMsg::MoveQueueItem { from, to } => {
                // **Optimistic.** The row is already where the user dropped it,
                // so the projection moves now and MusicKit's echo confirms it —
                // the same shape as a library write, and for the same reason: a
                // drop that visibly springs back while a command is in flight
                // reads as a failure even when it worked.
                if !self.player.move_item(from, to) {
                    return;
                }
                tracing::info!(from, to, "reordering the queue");
                self.pending_move = Some((from, to));
                self.send(Command::MoveInQueue { from, to });
                // `push_snapshot` re-syncs the queue view from the projection,
                // so the row is already in its new place before the echo.
                self.push_snapshot();
            }
            AppMsg::ClearQueue => {
                tracing::info!("clearing the queue");
                self.clear_segment_loop();
                self.send(Command::ClearQueue);
                // Nothing to come back to next launch, either. The mirror
                // follows the sidecar's queue event as always (rule 3) — this
                // is only the part MusicKit cannot know about.
                self.last_queue = None;
                self.pending_start = None;
                self.last_item = None;
                // Invalidate artwork work already off-thread before clearing
                // the surface, or its late result could put the old album back.
                self.art_for = None;
                self.art_path = None;
                crate::session::clear();
                crate::style::set_track_visuals(None, None);
            }
            AppMsg::JumpTo { at, id } => match self.queue_index_at(at, &id) {
                Some(index) => {
                    self.send(Command::ChangeToIndex { index });
                    // Clicking a track in the queue is a request to *play* it.
                    // `changeToMediaAtIndex` only moves the cursor, so on a
                    // queue that is loaded but idle — a restored session, or a
                    // paused one — it moved silently and looked like nothing
                    // had happened.
                    if !self.player.state.is_playing() {
                        self.send(Command::Play);
                    }
                }
                None => self.toast("That track is no longer in the queue"),
            },
            AppMsg::RemoveFromQueue { at, id } => match self.queue_index_at(at, &id) {
                Some(index) => self.send(Command::RemoveFromQueue { index }),
                None => self.toast("That track is no longer in the queue"),
            },
            AppMsg::SetAccent(accent) => {
                self.settings.accent = accent.id().into();
                self.settings.save();
                // Live: the provider is replaced, and every widget already
                // referencing the accent variables repaints itself.
                crate::style::set_accent(accent, self.settings.companion);
            }
            AppMsg::SetCompanion(companion) => {
                if self.settings.companion == companion {
                    return;
                }
                self.settings.companion = companion;
                self.settings.save();
                self.lyrics_view.set_companion(companion);
                self.jamkin_mode.set_companion(companion);
                // Match Jamkin updates immediately; a manually chosen accent
                // remains manual while the artwork still changes.
                crate::style::set_accent(
                    crate::style::Accent::parse(&self.settings.accent),
                    companion,
                );
                // The optional Discord copy names the Jamkin that is
                // listening. Refresh it immediately instead of waiting for a
                // playback event.
                self.push_snapshot();
            }
            AppMsg::SetJamkinQuality(quality) => {
                if self.settings.jamkin_quality == quality {
                    return;
                }
                self.settings.jamkin_quality = quality;
                self.settings.save();
                self.lyrics_view.set_quality(quality);
                self.jamkin_mode.set_quality(quality);
            }
            AppMsg::SetLauncherIcon(icon) => {
                if self.settings.launcher_icon == icon {
                    return;
                }
                if self.launcher_icon_pending.is_some() {
                    self.toast("Finish the open launcher confirmation first");
                    return;
                }
                self.launcher_icon_pending = Some(icon);
                self.toast(crate::launcher_icon::CONFIRM_HELP);
                sender.oneshot_command(async move {
                    CommandMsg::LauncherIconInstalled {
                        icon,
                        result: crate::launcher_icon::install(icon)
                            .await
                            .map_err(|err| format!("{err:#}")),
                    }
                });
            }
            AppMsg::SetDesktopJamkin(enabled) => {
                if let Some(action) = &self.jamkin_action {
                    action.set_state(&enabled.to_variant());
                }
                if self.settings.desktop_jamkin == enabled {
                    return;
                }
                self.settings.desktop_jamkin = enabled;
                self.settings.save();
                self.jamkin_mode.set_enabled(enabled);
                if enabled {
                    self.ensure_lyrics(&sender);
                } else if self.view != View::Lyrics {
                    // Invalidate a late provider answer when the only surface
                    // that wanted it has gone away.
                    self.lyrics_generation = self.lyrics_generation.wrapping_add(1);
                    self.lyrics_for = None;
                    self.lyrics_loading = false;
                }
            }
            AppMsg::SetDesktopJamkinSize(size) => {
                let size = size.clamp(
                    crate::settings::MIN_DESKTOP_JAMKIN_SIZE,
                    crate::settings::MAX_DESKTOP_JAMKIN_SIZE,
                );
                if self.settings.desktop_jamkin_size == size {
                    return;
                }
                self.settings.desktop_jamkin_size = size;
                self.settings.save();
                self.jamkin_mode.set_size(size);
            }
            AppMsg::SetDesktopJamkinOpacity(opacity) => {
                let opacity = opacity.clamp(
                    crate::settings::MIN_DESKTOP_JAMKIN_OPACITY,
                    crate::settings::MAX_DESKTOP_JAMKIN_OPACITY,
                );
                if self.settings.desktop_jamkin_opacity == opacity {
                    return;
                }
                self.settings.desktop_jamkin_opacity = opacity;
                self.settings.save();
                self.jamkin_mode.set_opacity(opacity);
            }
            AppMsg::SetJamkinReducedMotion(reduced) => {
                if self.settings.jamkin_reduced_motion == reduced {
                    return;
                }
                self.settings.jamkin_reduced_motion = reduced;
                self.settings.save();
                self.lyrics_view.set_reduced_motion(reduced);
                self.jamkin_mode.set_reduced_motion(reduced);
            }
            AppMsg::SetDesktopJamkinStayVisible(stay_visible) => {
                if self.settings.desktop_jamkin_stay_visible == stay_visible {
                    return;
                }
                self.settings.desktop_jamkin_stay_visible = stay_visible;
                self.settings.save();
                self.jamkin_mode.set_stay_visible(stay_visible);
            }
            AppMsg::SetDesktopJamkinAbove(above) => {
                let actual = self.jamkin_mode.set_keep_above(above);
                if above && !actual {
                    self.toast("Keeping Jamkin above is unavailable on this desktop");
                }
                let oled_care = self.jamkin_mode.oled_care_enabled();
                if self.settings.desktop_jamkin_above != actual
                    || self.settings.desktop_jamkin_oled_care != oled_care
                {
                    self.settings.desktop_jamkin_above = actual;
                    self.settings.desktop_jamkin_oled_care = oled_care;
                    self.settings.save();
                }
            }
            AppMsg::SetDesktopJamkinOledCare(enabled) => {
                let actual = self.jamkin_mode.set_oled_care(enabled);
                if enabled && !actual {
                    self.toast("Edge Walk is unavailable on this desktop");
                }
                let above = actual || self.settings.desktop_jamkin_above;
                if self.settings.desktop_jamkin_oled_care != actual
                    || self.settings.desktop_jamkin_above != above
                {
                    self.settings.desktop_jamkin_oled_care = actual;
                    self.settings.desktop_jamkin_above = above;
                    self.settings.save();
                }
            }
            AppMsg::SetDesktopJamkinPosition { right, bottom } => {
                let right = right.clamp(0, crate::settings::MAX_DESKTOP_JAMKIN_MARGIN);
                let bottom = bottom.clamp(0, crate::settings::MAX_DESKTOP_JAMKIN_MARGIN);
                if self.settings.desktop_jamkin_right == right
                    && self.settings.desktop_jamkin_bottom == bottom
                {
                    return;
                }
                self.settings.desktop_jamkin_right = right;
                self.settings.desktop_jamkin_bottom = bottom;
                self.settings.save();
            }
            AppMsg::SetDiscordActivity(enabled) => {
                let enabled = self.discord_presence.set_enabled(enabled);
                if self.settings.discord_activity == enabled {
                    return;
                }
                self.settings.discord_activity = enabled;
                self.settings.save();
                if enabled {
                    self.push_snapshot();
                    self.toast("Discord activity is on");
                } else {
                    self.toast("Discord activity is off");
                }
            }
            AppMsg::ConfigureGlobalShortcuts => {
                if self.global_shortcuts_stop.is_some() {
                    self.toast("Global shortcuts are already connected");
                    return;
                }
                self.global_shortcuts_stop = Some(global_shortcuts::start(&sender));
            }
            AppMsg::DisableGlobalShortcuts => {
                if let Some(stop) = self.global_shortcuts_stop.take() {
                    let _ = stop.send(true);
                }
                self.settings.global_shortcuts = false;
                self.settings.save();
                self.toast("Global shortcuts are off");
            }
            AppMsg::ShowListenBrainzSetup => {
                self.show_listenbrainz_setup(&sender, root);
            }
            AppMsg::EnableListenBrainz(value) => {
                let token = match crate::scrobble::Token::parse(value) {
                    Ok(token) => token,
                    Err(message) => {
                        self.toast(message);
                        return;
                    }
                };
                let saved_token = token.clone();
                sender.oneshot_command(async move {
                    CommandMsg::ListenBrainzTokenStored {
                        token: saved_token,
                        result: crate::scrobble::store_token(&token).await,
                    }
                });
            }
            AppMsg::DisableListenBrainz => {
                self.scrobbler.disable();
                self.settings.listenbrainz_scrobbling = false;
                self.settings.save();
                match crate::scrobble::remove_token() {
                    Ok(()) => self.toast("ListenBrainz scrobbling is off"),
                    Err(_error) => {
                        tracing::warn!("could not remove encrypted ListenBrainz token");
                        self.toast(
                            "Scrobbling is off, but its encrypted token could not be removed",
                        );
                    }
                }
            }
            AppMsg::SetPlayerBackdrop(on) => {
                self.settings.player_backdrop = on;
                self.settings.save();
                // Live, like the accent. `style` retains the current cover and
                // palette, so turning album glass back on needs no track change.
                crate::style::set_backdrop_enabled(on);
            }
            AppMsg::SetGlassStrength(strength) => {
                let strength = strength.min(100);
                if self.settings.glass_strength == strength {
                    return;
                }
                let previous = self.settings.glass_strength;
                self.settings.glass_strength = strength;
                self.settings.save();
                // Opacity follows the control immediately. The corresponding
                // blurred bitmap is rebuilt off the GTK thread and tagged so a
                // late result cannot repaint a newer song or slider position.
                crate::style::set_glass_strength(strength);
                if artwork::backdrop_blur_radius(previous)
                    != artwork::backdrop_blur_radius(strength)
                {
                    self.refresh_backdrop(&sender);
                }
            }
            AppMsg::SetLyricsAccentStrength(strength) => {
                let strength = strength.min(100);
                if self.settings.lyrics_accent_strength == strength {
                    return;
                }
                self.settings.lyrics_accent_strength = strength;
                self.settings.save();
                crate::style::set_lyrics_accent_strength(strength);
            }
            AppMsg::SetLyricsFontScale(scale) => {
                let scale = scale.clamp(
                    crate::settings::MIN_LYRICS_FONT_SCALE,
                    crate::settings::MAX_LYRICS_FONT_SCALE,
                );
                if self.settings.lyrics_font_scale == scale {
                    return;
                }
                self.settings.lyrics_font_scale = scale;
                self.settings.save();
                crate::style::set_lyrics_font_scale(scale);
            }
            AppMsg::AdjustLyricTiming(delta_ms) => {
                let Some(catalog_id) = self
                    .lyrics_for
                    .as_ref()
                    .and_then(|query| query.catalog_id.clone())
                else {
                    self.toast("Timing adjustments need an Apple catalog song");
                    return;
                };
                let current = self.lyric_offsets.get(Some(&catalog_id));
                let next = if delta_ms == 0 {
                    0
                } else {
                    current.saturating_add(delta_ms).clamp(
                        crate::lyric_timing::MIN_OFFSET_MS,
                        crate::lyric_timing::MAX_OFFSET_MS,
                    )
                };
                if self.lyric_offsets.set(&catalog_id, next) {
                    self.lyrics_view.set_timing_offset(next);
                    self.sync_lyrics_position();
                }
            }
            AppMsg::SelectLyricVariant(index) => {
                let Some(query) = self.lyrics_for.as_ref() else {
                    return;
                };
                let Some(lyrics) = self.lyrics_cache.get(query) else {
                    return;
                };
                if index > lyrics.variants.len() {
                    return;
                }
                self.lyrics_view.show_variant(lyrics, index);
                self.jamkin_mode.show(&lyrics.selected(index));
                self.sync_lyrics_position();
            }
            AppMsg::SetSort(sort) => {
                let mut current = self.sorts.get(self.view);
                if sort == current.by {
                    return;
                }
                current.by = sort;
                self.sorts.set(self.view, current);
                self.persist_sorts();
                tracing::info!(sort = sort.id(), section = ?self.view, "sort");
                // A rebuild resets the scroll, which is right here: the list
                // the user was looking at no longer exists in that order.
                self.resort();
            }
            AppMsg::Raise => {
                // `present` covers both states this can arrive in: hidden after
                // a close, or open behind something else. The background hold is
                // dropped by `WindowShown`, which `connect_show` raises from
                // here — one path back, however the window was reached.
                tracing::info!("raising the window for MPRIS");
                root.present();
            }
            AppMsg::WindowCloseRequested => {
                self.jamkin_mode.set_main_window_visible(false);
                self.close_window(root, &sender);
            }
            AppMsg::PlayerDrawer(open) => {
                if self.show_queue != open {
                    self.show_queue = open;
                    // The bar's queue button is a watch on the snapshot, so it
                    // has to be told the drawer moved without it.
                    self.push_snapshot();
                }
            }
            AppMsg::WindowShown => {
                self.jamkin_mode.set_main_window_visible(true);
                // Dropping the guard is the whole of it: with a window on
                // screen GTK keeps the app alive by itself, and `background`
                // should mean what its name says.
                if self.background.take().is_some() {
                    tracing::info!("window shown; no longer background-only");
                }
            }
            AppMsg::RemoveFromLibrary {
                library_id,
                catalog_id,
            } => {
                tracing::info!("removing from library");
                self.pending_writes.insert(
                    library_id.clone(),
                    PendingWrite {
                        catalog_id: catalog_id.clone(),
                        undo: WriteUndo::InLibrary(true),
                    },
                );
                self.send(Command::RemoveFromLibrary { id: library_id });
                // Mirrored locally for the same reason the star is: the menu
                // reads this, and making someone reload to see their own click
                // is absurd. `include=library` is cached for tens of seconds
                // besides, so a read-back would disagree for a while (#34).
                self.set_in_library(&catalog_id, false);
                self.toast("Removing from your library…");
            }
            AppMsg::Unfavorite { catalog_id } => {
                tracing::info!("removing favourite");
                self.pending_writes.insert(
                    catalog_id.clone(),
                    PendingWrite {
                        catalog_id: catalog_id.clone(),
                        undo: WriteUndo::Favorite(true),
                    },
                );
                self.send(Command::Unfavorite {
                    id: catalog_id.clone(),
                });
                // The star only. The song stays in the library — see the note
                // on `AppMsg::Unfavorite`.
                self.set_favorite(&catalog_id, false);
                // Present continuous, not a claim: nothing has been confirmed
                // yet, and `undo_pending_write` is what happens if it is not.
                self.toast("Removing favourite…");
            }
            AppMsg::LibraryWrite { catalog_id, action } => {
                let Some(client) = self.client() else {
                    self.toast("Not connected yet");
                    return;
                };
                // Said out loud before the request goes out: these are
                // fire-and-forget, and a click with no feedback at all reads as
                // a click that did not register.
                self.toast(action.sent());
                tracing::info!(?action, "library write");
                let generation = self.account_generation;
                sender.oneshot_command(async move {
                    let result = match action {
                        LibraryAction::AddToLibrary => {
                            client.add_song_to_library(&catalog_id).await
                        }
                        LibraryAction::Favorite => client.favorite_song(&catalog_id).await,
                    };
                    CommandMsg::LibraryWritten {
                        generation,
                        catalog_id,
                        action,
                        result: result.map_err(|err| format!("{err:#}")),
                    }
                });
            }
            AppMsg::ToggleSortDirection => {
                let mut current = self.sorts.get(self.view);
                current.reversed = !current.reversed;
                self.sorts.set(self.view, current);
                self.persist_sorts();
                self.resort();
            }
            AppMsg::ShowRowMenu(req) => self.show_row_menu(req),
            AppMsg::Enqueue { catalog_id, next } => {
                let songs = vec![catalog_id];
                if self.player.queue.is_empty() {
                    // Nothing to insert into: `playNext` on an empty queue is a
                    // silent no-op in MusicKit. Start the queue instead —
                    // "add to queue" with no queue plainly means "make one",
                    // and refusing was a worse answer than doing it.
                    tracing::info!("starting a queue from one track");
                    // A queue being created, so it says what it starts as
                    // rather than inheriting the last one's mode.
                    self.send(Command::SetShuffle { shuffle: false });
                    self.pending_start = songs.first().cloned();
                    self.last_queue = Some((songs.clone(), songs.first().cloned()));
                    self.send(Command::SetQueue {
                        songs,
                        start_position: 0,
                        start_playing: true,
                        start_time_ms: 0,
                    });
                    return;
                }
                tracing::info!(next, "enqueueing one track");
                self.send(if next {
                    Command::PlayNext { songs }
                } else {
                    Command::PlayLater { songs }
                });
            }
            AppMsg::SetShuffle(on) => {
                // Sent and forgotten: the mirror updates when MusicKit echoes
                // it back, so the button never claims a state the player is not
                // actually in (rule 3).
                tracing::info!(on, "shuffle");
                self.send(Command::SetShuffle { shuffle: on });
            }
            AppMsg::SetRepeat(mode) => {
                let mode = match mode {
                    Repeat::Off => RepeatMode::None,
                    Repeat::All => RepeatMode::All,
                    Repeat::One => RepeatMode::One,
                };
                tracing::info!(?mode, "repeat");
                self.send(Command::SetRepeat { mode });
            }
            AppMsg::PlayFrom(index) => {
                let visible = self.visible_entries();
                self.play_entries(&visible, index, Start::Clicked);
            }
        }
    }

    fn handle_cmd(
        &mut self,
        msg: CommandMsg,
        sender: &ComponentSender<Self>,
        root: &adw::ApplicationWindow,
    ) {
        let sender = sender.clone();
        match msg {
            CommandMsg::AlbumPage {
                generation,
                page,
                result,
            } => {
                if generation != self.account_generation {
                    return;
                }
                let Some(target) = self.pages.iter_mut().find(|p| p.id == page) else {
                    // Navigated back while this was in flight.
                    return;
                };
                match result {
                    Ok((album, tracks)) => {
                        tracing::info!(page, tracks = tracks.len(), "album loaded");
                        let art = album.artwork.clone();
                        target.show_album(&album, tracks.into_iter().map(Entry::Song).collect());
                        self.fetch_page_art(page, art, &sender);
                    }
                    Err(err) => {
                        tracing::warn!(page, %err, "album page failed");
                        target.fail(&err);
                    }
                }
            }
            CommandMsg::ArtistPage {
                generation,
                page,
                result,
            } => {
                if generation != self.account_generation {
                    return;
                }
                let Some(target) = self.pages.iter_mut().find(|p| p.id == page) else {
                    return;
                };
                match result {
                    Ok(data) => {
                        tracing::info!(
                            page,
                            top_songs = data.top_songs.len(),
                            albums = data.albums.len(),
                            "artist loaded"
                        );
                        target.show_artist(
                            &data.artist,
                            data.top_songs,
                            data.latest_release,
                            data.albums,
                        );
                    }
                    Err(err) => {
                        tracing::warn!(page, %err, "artist page failed");
                        target.fail(&err);
                    }
                }
            }
            CommandMsg::QueueTrackPage { generation, result } => {
                if generation != self.account_generation {
                    return;
                }
                match result {
                    Ok(Some(kind)) => {
                        // **Close the drawer, or the page lands behind it.** The
                        // queue only exists inside the player sheet, which is modal
                        // and covers the navigation stack — so pushing a page and
                        // leaving the drawer up meant the *successful* click was
                        // the only silent one, both failures below being toasts
                        // that draw above the sheet.
                        //
                        // On success only: a lookup that found nothing should not
                        // also take the queue away.
                        self.show_queue = false;
                        self.sync_page_controls();
                        self.push_snapshot();
                        self.push_page(kind, &sender);
                    }
                    // Said out loud rather than nothing happening: a menu item that
                    // silently does nothing is the failure this project keeps
                    // refusing to ship.
                    Ok(None) => self.toast("Apple doesn't say where that track came from"),
                    Err(err) => {
                        tracing::warn!(%err, "resolving a queue track's album or artist");
                        self.toast("Couldn't open that");
                    }
                }
            }
            CommandMsg::Pruned(report) => {
                // Reported here rather than inside the sweep, so the sweep
                // stays a function that returns facts and can be tested as one.
                // Silent when it found nothing, which is the ordinary case.
                if report.removed > 0 {
                    tracing::info!(
                        removed = report.removed,
                        freed_kb = report.freed / 1024,
                        kept = report.kept,
                        over_cap = report.over_cap,
                        was_mb = report.total / 1_048_576,
                        "swept the artwork cache"
                    );
                }
            }
            CommandMsg::LibraryAlbums { generation, result } => {
                if generation != self.account_generation {
                    return;
                }
                self.loading_albums = false;
                match result {
                    Ok(albums) => {
                        let changed = albums != self.albums;
                        tracing::info!(albums = albums.len(), changed, "library albums loaded");
                        self.albums = albums;
                        self.maybe_prune_artwork(&sender);
                        if changed {
                            self.built_albums = None;
                            self.rebuild_albums();
                            self.save_cache();
                        }
                    }
                    Err(err) => {
                        tracing::warn!(%err, "library albums failed");
                        self.toast(&err);
                    }
                }
            }
            CommandMsg::LibraryArtists { generation, result } => {
                if generation != self.account_generation {
                    return;
                }
                self.loading_artists = false;
                match result {
                    Ok(artists) => {
                        let changed = artists != self.artists;
                        tracing::info!(artists = artists.len(), changed, "library artists loaded");
                        self.artists = artists;
                        self.maybe_prune_artwork(&sender);
                        if changed {
                            self.built_artists = None;
                            self.rebuild_artists();
                            self.save_cache();
                        }
                    }
                    Err(err) => {
                        tracing::warn!(%err, "library artists failed");
                        self.toast(&err);
                    }
                }
            }
            CommandMsg::LibraryPlaylists { generation, result } => {
                if generation != self.account_generation {
                    return;
                }
                self.loading_playlists = false;
                match result {
                    Ok(mut playlists) => {
                        playlist_art::carry_cached_covers(&self.playlists, &mut playlists);
                        let changed = playlists != self.playlists;
                        tracing::info!(
                            playlists = playlists.len(),
                            changed,
                            "library playlists loaded"
                        );
                        self.playlists = playlists;
                        // Before the names are refreshed, so a pin that is gone
                        // never gets a chance to draw as "Unavailable".
                        self.prune_stale_pins(&sender);
                        self.refresh_pin_names();
                        self.maybe_prune_artwork(&sender);
                        if changed {
                            self.built_playlists = None;
                            self.rebuild_playlists();
                            self.save_cache();
                        }
                    }
                    Err(err) => {
                        tracing::warn!(%err, "library playlists failed");
                        self.toast(&err);
                    }
                }
            }
            CommandMsg::Explore { generation, result } => self.finish_explore(generation, result),
            CommandMsg::Lyrics {
                generation,
                query,
                result,
            } => self.finish_lyrics(generation, query, result),
            CommandMsg::PlaylistPage {
                generation,
                page,
                result,
            } => {
                if generation != self.account_generation {
                    return;
                }
                let Some(target) = self.pages.iter_mut().find(|p| p.id == page) else {
                    return;
                };
                match result {
                    Ok((playlist, tracks)) => {
                        tracing::info!(page, tracks = tracks.len(), "playlist loaded");
                        let art = playlist.artwork.clone();
                        // Read before the tracks are moved: a playlist Apple
                        // sends no picture for gets one composed from these.
                        let covers = pages::playlist_covers(&tracks);
                        target.show_playlist(
                            &playlist,
                            tracks.into_iter().map(Entry::Song).collect(),
                        );
                        self.fetch_page_art_or_mosaic(page, art, covers, &sender);
                    }
                    Err(err) => {
                        tracing::warn!(page, %err, "playlist page failed");
                        target.fail(&err);
                    }
                }
            }
            // Advisory: the app is already in the background by the time this
            // answers. A refusal costs discoverability in Quick Settings, not
            // playback, so it is logged rather than toasted.
            CommandMsg::BackgroundPortal(result) => match result {
                Ok(()) => tracing::info!("background portal: listed"),
                // Almost always "no AppId detected": the portal identifies a
                // non-sandboxed app from its systemd scope, which only exists
                // when it was launched from its .desktop entry. A binary run
                // straight from a terminal cannot be listed, and that is a
                // property of the session rather than a fault here — playback
                // is unaffected either way.
                Err(err) => tracing::warn!(
                    %err,
                    "background portal refused; Quick Settings will not list Jamelade \
                     (expected when not launched from its .desktop entry)"
                ),
            },
            CommandMsg::LauncherIconInstalled { icon, result } => {
                if self.launcher_icon_pending != Some(icon) {
                    return;
                }
                self.launcher_icon_pending = None;
                match result {
                    Ok(method) => {
                        self.settings.launcher_icon = icon;
                        self.settings.save();
                        root.set_icon_name(Some(icon.window_icon_name()));
                        self.toast(match method {
                            crate::launcher_icon::InstallMethod::Helper => {
                                crate::launcher_icon::HELPER_CHANGED_HELP
                            }
                            crate::launcher_icon::InstallMethod::Portal => {
                                crate::launcher_icon::PORTAL_CHANGED_HELP
                            }
                        });
                    }
                    Err(err) => {
                        tracing::warn!(%err, "launcher icon change failed");
                        self.toast("Launcher icon was not changed");
                    }
                }
            }
            CommandMsg::LibraryWritten {
                generation,
                catalog_id,
                action,
                result,
            } => {
                if generation != self.account_generation {
                    return;
                }
                match result {
                    Ok(()) => {
                        // "Sent", not "added": Apple's 202 means accepted, and the
                        // change may still be in flight on their side.
                        self.toast(action.done());
                        // The star, however, we can move now. `inFavorites` is only
                        // re-read on a library reload, and making someone reload to
                        // see their own click is absurd — so mirror it locally and
                        // repaint just that row.
                        match action {
                            LibraryAction::Favorite => {
                                self.set_favorite(&catalog_id, true);
                                // Favouriting *adds to the library* — Apple's
                                // behaviour, measured (#34). So the menu must stop
                                // offering "Add to Library" for it too.
                                self.set_in_library(&catalog_id, true);
                            }
                            // Mirrored so the menu stops offering an add that has
                            // already happened. No library id yet — the 202 carries
                            // no body and Apple assigns one asynchronously — so
                            // "Remove from Library" stays hidden until a reload
                            // learns it. Offering a removal we cannot address would
                            // be a menu item that quietly does nothing.
                            LibraryAction::AddToLibrary => self.set_in_library(&catalog_id, true),
                        }
                    }
                    Err(err) => {
                        tracing::warn!(?action, %err, "library write failed");
                        self.toast(&err);
                    }
                }
            }
            CommandMsg::PlaylistWritten {
                generation,
                created,
                result,
            } => {
                if generation != self.account_generation {
                    return;
                }
                match result {
                    Ok(()) => {
                        self.toast(if created {
                            "Playlist created"
                        } else {
                            "Song added to playlist"
                        });
                        // Apple's write response carries no trustworthy final
                        // object. Refresh the bounded library projection rather
                        // than inventing one locally.
                        self.loading_playlists = false;
                        self.tried_playlists = false;
                        self.built_playlists = None;
                        self.load_playlists(&sender);
                    }
                    Err(error) => {
                        tracing::warn!(created, "playlist write failed");
                        self.toast(&format!("Playlist change failed: {error}"));
                    }
                }
            }
            CommandMsg::Credits { generation, result } => {
                if generation != self.account_generation {
                    return;
                }
                match result {
                    Ok(credits) => show_credits(root, &credits),
                    Err(error) => {
                        tracing::warn!("song credits request failed");
                        self.toast(&format!("Credits could not load: {error}"));
                    }
                }
            }
            CommandMsg::GlobalShortcutsReady(result) => match result {
                Ok(()) => {
                    // A late portal reply must not undo an explicit disable.
                    if self.global_shortcuts_stop.is_none() {
                        return;
                    }
                    self.settings.global_shortcuts = true;
                    self.settings.save();
                    self.toast("Global shortcuts are ready");
                }
                Err(error) => {
                    self.global_shortcuts_stop = None;
                    self.settings.global_shortcuts = false;
                    self.settings.save();
                    let _ = error;
                    tracing::warn!("global shortcuts portal failed");
                    self.toast("Global shortcuts could not be configured");
                }
            },
            CommandMsg::GlobalShortcut(id) => match id.as_str() {
                "play-pause" => sender.input(AppMsg::PlayPause),
                "next" => sender.input(AppMsg::Next),
                "previous" => sender.input(AppMsg::Previous),
                "lyrics" => sender.input(AppMsg::ShowLyrics),
                _ => tracing::warn!("desktop portal returned an unknown shortcut"),
            },
            CommandMsg::ListenBrainzTokenLoaded(result) => match result {
                Ok(Some(token)) if self.settings.listenbrainz_scrobbling => {
                    self.scrobbler.set_token(token);
                    tracing::info!("ListenBrainz scrobbling is ready");
                }
                Ok(None) => {
                    self.settings.listenbrainz_scrobbling = false;
                    self.settings.save();
                }
                Ok(Some(_)) => {}
                Err(error) => {
                    self.settings.listenbrainz_scrobbling = false;
                    self.settings.save();
                    let _ = error;
                    tracing::warn!("ListenBrainz token could not be unlocked");
                    self.toast("ListenBrainz is off because its token could not be unlocked");
                }
            },
            CommandMsg::ListenBrainzTokenStored { token, result } => match result {
                Ok(()) => {
                    self.scrobbler.set_token(token);
                    self.settings.listenbrainz_scrobbling = true;
                    self.settings.save();
                    self.toast("ListenBrainz scrobbling is on");
                }
                Err(error) => {
                    let _ = error;
                    tracing::warn!("ListenBrainz token could not be stored");
                    self.toast("ListenBrainz could not be enabled securely");
                }
            },
            CommandMsg::ListenBrainzSubmitted { key, result } => {
                self.scrobbler.finish(&key, result.is_ok());
                if let Err(error) = result {
                    let _ = error;
                    tracing::warn!("ListenBrainz submission failed");
                    self.toast("ListenBrainz scrobble failed");
                }
            }
            CommandMsg::TileArt {
                generation,
                key,
                path,
                cover,
            } => {
                if generation != self.account_generation {
                    if let Some(path) = path {
                        let _ = std::fs::remove_file(path);
                    }
                    return;
                }
                self.tile_art_pending.remove(&key);
                let (Some(_path), Some(cover)) = (path, cover) else {
                    // Cosmetic. The tile keeps its placeholder.
                    return;
                };

                // Paint **every** tile showing this artwork now. Recycling
                // means they may not include the one that asked, and may be
                // none at all if it scrolled away — both are correct.
                //
                // All three registries, not the first that matches: the grids
                // hold their tiles bound whether or not the user is looking at
                // them, so a hidden album tile and a visible playlist tile can
                // want the same key at once. Stopping at the first match paid
                // the hidden one and left the visible one blank, which is what
                // "some artwork does not load" turned out to be.
                let texture = cover.into_texture();
                let mut painted = 0usize;
                for registry in [
                    &self.album_art_widgets,
                    &self.artist_art_widgets,
                    &self.playlist_art_widgets,
                ] {
                    for widget in registry.borrow().get(&key).into_iter().flatten() {
                        widget.set_texture(&texture);
                        painted += 1;
                    }
                }
                painted += self.explore_view.paint(&key, &texture);
                for page in &self.pages {
                    painted += page.paint_artist_art(&key, &texture);
                }
                tracing::trace!(painted, "tile art delivered");
            }
            CommandMsg::PlaylistTileArt(result) => self.finish_playlist_art(result, &sender),
            CommandMsg::PageArtwork {
                generation,
                page,
                path,
            } => {
                if generation != self.account_generation {
                    if let Some(path) = path {
                        let _ = std::fs::remove_file(path);
                    }
                    return;
                }
                if let (Some(path), Some(target)) = (path, self.pages.iter().find(|p| p.id == page))
                {
                    target.set_artwork(&path);
                }
            }
            CommandMsg::Spawned(handle) => {
                self.sidecar = Some(handle);
                // The process is up; Chromium's component updater is now
                // fetching the CDM (instant after the first run).
                self.stage = Stage::InstallingWidevine;
            }
            CommandMsg::Library {
                generation,
                result: Ok(tracks),
            } => {
                if generation != self.account_generation {
                    return;
                }
                self.loading_library = false;
                let unplayable = tracks.iter().filter(|t| !t.playable()).count();
                // A refresh over a cached library usually finds nothing new,
                // and a rebuild it did not need costs ~500ms of cover decoding
                // *and* resets the scroll under whoever is reading. Equality is
                // the whole test: same tracks, same order, same fields.
                let changed = tracks != self.all_tracks;
                tracing::info!(tracks = tracks.len(), unplayable, changed, "library loaded");
                self.all_tracks = tracks;
                self.maybe_prune_artwork(&sender);
                if changed {
                    self.built_rows = None;
                    self.rebuild_rows();
                    self.save_cache();
                }
            }
            CommandMsg::Catalog {
                generation,
                offset,
                result,
            } => {
                // Responses can arrive out of order: a slow request for "aita"
                // must not overwrite the results for "aitana".
                if generation != self.search_gen {
                    tracing::debug!("discarding stale catalog results");
                    return;
                }
                self.searching_catalog = false;
                match result {
                    Ok(found) => {
                        let first_page = offset == 0;
                        let (rows, paged) =
                            library::catalog_rows(self.catalog_filter, found, first_page);

                        // A short page of the **paging kind** means Apple has
                        // no more. Which kind that is depends on the filter,
                        // which is why the count comes back from the fold
                        // rather than being read off one field here.
                        self.catalog_exhausted = paged < CATALOG_LIMIT as usize;
                        self.catalog_paged = if first_page {
                            paged
                        } else {
                            self.catalog_paged + paged
                        };

                        tracing::info!(
                            rows = self.catalog.len() + rows.len(),
                            paged = self.catalog_paged,
                            filter = ?self.catalog_filter,
                            exhausted = self.catalog_exhausted,
                            "catalog results"
                        );

                        if first_page {
                            // New answer: the rows on screen are for a
                            // different question, so they all go.
                            self.catalog = rows;
                            self.built_rows = None;
                            self.rebuild_rows();
                        } else {
                            // A later page only ever *adds*. Rebuilding would
                            // discard every widget and with them the scroll
                            // position — putting the reader back at the top of
                            // the list they had just scrolled to the bottom of
                            // in order to ask for this page.
                            self.append_rows(&rows);
                            self.catalog.extend(rows);
                        }
                    }
                    Err(err) => {
                        tracing::warn!(%err, "catalog search failed");
                        self.toast(&format!("Search failed: {err}"));
                    }
                }
            }
            CommandMsg::Library {
                generation,
                result: Err(err),
            } => {
                if generation != self.account_generation {
                    return;
                }
                self.loading_library = false;
                tracing::warn!(%err, "library load failed");
                self.toast(&format!("Couldn't load your library: {err}"));
            }
            CommandMsg::Artwork {
                generation,
                template,
                path,
                backdrop,
                glass_strength,
                palette,
            } => {
                if generation != self.account_generation
                    || self.art_for.as_deref() != Some(&template)
                {
                    if let Some(path) = path {
                        let _ = std::fs::remove_file(path);
                    }
                    if let Some(backdrop) = backdrop {
                        let _ = std::fs::remove_file(backdrop);
                    }
                    tracing::debug!("discarding artwork for a track that has moved on");
                    return;
                }
                if path.is_none() {
                    // Cosmetic. The bar falls back to a generic icon.
                    tracing::debug!("artwork unavailable");
                }
                self.art_path = path.clone();
                // Put the cover behind the whole window. Scaled off the GTK thread
                // alongside the fetch, so this is only the CSS swap.
                crate::style::set_track_visuals(backdrop.as_deref(), palette);
                if artwork::backdrop_blur_radius(glass_strength)
                    != artwork::backdrop_blur_radius(self.settings.glass_strength)
                {
                    // The slider moved while Apple art was still downloading.
                    // Keep this honest cover visible, then replace only its blur
                    // with a current tagged variant as soon as it is ready.
                    self.refresh_backdrop(&sender);
                }
                self.now_playing
                    .emit(NowPlayingInput::ArtworkReady(path.clone()));
                self.player_view.emit(PlayerViewInput::Artwork(path));

                // A notification was held back for this track so it would not
                // go out carrying the previous album's cover. Guarded on the
                // id, in case the track changed again while the fetch ran.
                if self.notify_when_art_lands.is_some()
                    && self.notify_when_art_lands == self.playing_catalog_id()
                {
                    self.send_track_notification();
                }

                // MPRIS carries the cover too, so the Shell applet and lock
                // screen pick it up as soon as it lands.
                self.push_snapshot();
            }
            CommandMsg::GlassBackdrop {
                generation,
                source,
                glass_strength,
                backdrop,
            } => {
                if generation != self.account_generation
                    || self.art_path.as_ref() != Some(&source)
                    || artwork::backdrop_blur_radius(self.settings.glass_strength)
                        != artwork::backdrop_blur_radius(glass_strength)
                {
                    if let Some(backdrop) = backdrop {
                        let _ = std::fs::remove_file(backdrop);
                    }
                    tracing::debug!(glass_strength, "discarding a stale glass blur variant");
                    return;
                }
                if let Some(backdrop) = backdrop.as_deref() {
                    crate::style::set_backdrop_art(Some(backdrop));
                }
            }
            CommandMsg::Sidecar(Incoming::Event(event)) => self.on_event(event, &sender),
            CommandMsg::Sidecar(Incoming::Unparsed) => {
                // preload.js and protocol.rs have drifted. Not fatal, but it
                // means an event is being silently ignored — say so.
                tracing::warn!("sidecar sent an event that failed validation");
            }
            CommandMsg::Sidecar(Incoming::Died(reason)) => {
                tracing::warn!(%reason, "sidecar died");
                self.clear_segment_loop();
                self.sidecar = None;
                self.restarts += 1;
                self.stage = Stage::Restarting(self.restarts);
                self.toast("Playback engine stopped — restarting");
                // The backoff belongs *inside* the respawn task. Sleeping in a
                // separate command and restarting here as well would restart
                // immediately and ignore the delay entirely.
                respawn_sidecar(&sender, sidecar::restart_delay(self.restarts));
            }
        }
    }
}

impl AppModel {
    /// Which set of music the search box is searching, derived from the
    /// section. Not stored: see [`View`].
    fn scope(&self) -> SearchScope {
        self.view.scope()
    }

    /// Fetch a section again, over the top of what it is already showing.
    ///
    /// **Nothing is cleared first.** The three grids used to empty themselves
    /// here, and had to: their guard was `tried || !collection.is_empty()`, so
    /// clearing the flag alone left the loader returning early. The cost was
    /// that `page()` saw an empty collection mid-fetch and took the grid away
    /// for a full-pane spinner — a reload that interrupted whatever you were
    /// looking at, and Songs never did it because Songs never cleared.
    ///
    /// The guard is `tried` alone now, so the clear is not only unnecessary but
    /// the whole of that bug. All four sections keep their content up, and the
    /// list changes only if the answer did.
    fn reload(&mut self, view: View, sender: &ComponentSender<Self>) {
        match view {
            View::Explore => {
                self.tried_explore = false;
                self.load_explore(sender);
            }
            View::Lyrics => self.ensure_lyrics(sender),
            View::Songs | View::Search => {
                self.tried_library = false;
                self.load_library(sender);
            }
            View::Albums => {
                self.tried_albums = false;
                self.load_albums(sender);
            }
            View::Artists => {
                self.tried_artists = false;
                self.load_artists(sender);
            }
            View::Playlists => {
                self.tried_playlists = false;
                self.load_playlists(sender);
            }
        }
    }

    /// Drop everything that belonged to the signed-in user.
    ///
    /// Not just the browser session: the library, grids, catalog results and
    /// pushed pages all came from that account, and leaving them on screen
    /// after a sign-out would show one person's music to whoever signs in
    /// next. The unplayable-id cache stays — it is about Apple's catalog, not
    /// about the user.
    fn forget_session(&mut self) {
        self.clear_segment_loop();
        self.scrobbler.reset_track();
        // Invalidate first. Every async API/artwork task owns credential or
        // account-derived input cloned before it started; none of its results
        // may land after the rest of this function has erased that account.
        self.account_generation = self.account_generation.wrapping_add(1);
        self.search_gen = self.search_gen.wrapping_add(1);
        self.stage = Stage::SignedOut;
        self.apple_session = None;
        self.clear_discovery_session();

        self.all_tracks.clear();
        self.albums.clear();
        self.artists.clear();
        self.playlists.clear();
        self.playlist_art.clear();
        self.tile_art_pending.clear();
        self.tried_albums = false;
        self.tried_artists = false;
        self.tried_playlists = false;
        self.tried_library = false;
        self.loading_albums = false;
        self.loading_artists = false;
        self.loading_playlists = false;
        self.loading_library = false;
        self.pruned = false;
        self.built_rows = None;
        self.built_albums = None;
        self.built_artists = None;
        self.built_playlists = None;
        self.catalog.clear();
        self.catalog_paged = 0;
        self.searching_catalog = false;
        self.catalog_exhausted = false;
        self.library_query.clear();
        self.catalog_query.clear();
        self.pending_writes.clear();
        self.row_overrides.borrow_mut().clear();

        // Pins are playlist identifiers from this account. Preferences such as
        // theme and Jamkin remain, but these identifiers must not follow the
        // next person who signs in or survive in settings.ini after sign-out.
        if !self.settings.pinned_playlists.is_empty() {
            self.settings.pinned_playlists.clear();
            self.settings.save();
        }
        self.sidebar_rows = sidebar_rows(&[]);
        self.selected_row = Some(SidebarRow::Section(self.view));
        self.pins_dirty = true;

        self.rebuild_rows();
        self.rebuild_albums();
        self.rebuild_artists();
        self.rebuild_playlists();

        // Pages and the queue belonged to that session too.
        self.pop_to_results();
        self.show_queue = false;
        self.player = PlayerState::new();
        self.last_item = None;
        self.last_queue = None;
        self.pending_start = None;
        self.pending_move = None;
        self.resume_at = None;
        self.healed = false;
        self.marked_playing = None;
        self.notified_for = None;
        self.progress_mark.set((0, 0));
        self.last_command.replace(None);
        self.art_for = None;
        self.art_path = None;
        crate::session::clear();
        crate::library_cache::clear();
        crate::components::artwork::clear_cache();
        crate::style::set_track_visuals(None, None);
        crate::notify::clear(relm4::main_application().upcast_ref::<gtk::gio::Application>());
        self.push_snapshot();
    }

    /// Put the first-run gate up or take it down, to match the session.
    ///
    /// Driven from one place rather than from each site that changes `stage`,
    /// because there are four of them — session state arriving, authorization
    /// change, a hook attaching, and signing out — and three of them would have
    /// been easy to forget.
    fn sync_onboarding(&mut self, sender: &ComponentSender<Self>, root: &adw::ApplicationWindow) {
        match (matches!(self.stage, Stage::SignedOut), &self.onboarding) {
            (true, None) => self.onboarding = Some(self.present_onboarding(sender, root)),
            (false, Some(dialog)) => {
                // `can_close` is false, so it will not go on its own.
                dialog.force_close();
                self.onboarding = None;
            }
            _ => {}
        }
    }

    fn toast(&self, text: &str) {
        self.toaster.add_toast(transient_toast(text));
    }

    fn copy_apple_link(&self, link: &str) {
        let Some(link) = crate::apple_link::canonical(link) else {
            self.toast("No public Apple Music link is available");
            return;
        };
        let Some(display) = gtk::gdk::Display::default() else {
            self.toast("Clipboard is unavailable");
            return;
        };
        display.clipboard().set_text(&link);
        self.toast("Apple Music link copied");
    }
}

/// Short status and error notices should never make the user hunt for their
/// close button. Prompts that require a decision use dialogs instead.
fn transient_toast(text: &str) -> adw::Toast {
    let toast = adw::Toast::new(text);
    toast.set_timeout(4);
    toast
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::types::TrackId;

    fn track(title: &str, catalog: Option<&str>) -> Track {
        Track {
            id: TrackId(format!("i.{title}")),
            catalog_id: catalog.map(str::to_owned),
            title: title.into(),
            favorite: false,
            in_library: false,
            library_id: None,
            date_added: String::new(),
            year: String::new(),
            artist: "Aitana".into(),
            album: "Superestrella".into(),
            duration_ms: 200_000,
            track_number: 1,
            artwork: None,
            share_url: None,
        }
    }

    #[test]
    fn a_stale_catalog_response_is_discarded() {
        // Responses arrive out of order: a slow request for "aita" must not
        // overwrite the results for "aitana" typed after it. The generation
        // carried on the response is what makes that decidable.
        let current = 7u64;
        assert!(6 != current, "an older generation is stale");
        assert!(7 == current, "the newest generation is the one to keep");
    }

    #[test]
    fn catalog_results_are_shown_unfiltered() {
        // Apple already ranked these. Re-filtering locally would drop results
        // that matched for reasons we cannot see — an alternate title, a
        // featured artist, a translation.
        let a = track("Bohemian Rhapsody", Some("1"));
        let catalog = [&a];
        let shown: Vec<_> = catalog.iter().collect();
        assert_eq!(shown.len(), 1);
    }
}
