// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! The expanded player: the Now Playing bar opened out into a drawer.
//!
//! A separate component from [`super::now_playing`] rather than a second mode
//! of it, for two reasons. That file is already at its size budget and this is
//! not a small view; and the two are genuinely different shapes — the bar is a
//! strip that must survive being 400px wide, this is a page that assumes room.
//!
//! What they share is deliberate: the same [`Snapshot`] in, the same
//! [`NowPlayingOutput`] out. The transport here cannot drift from the
//! transport there, because they are the same messages handled by the same
//! reducer arms. Anything else would be two players disagreeing about one
//! MusicKit.

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::{gdk, glib};
use relm4::prelude::*;

use self::transport::{Bits, build_transport};
use super::cover::{Cover, SWAP_MS};
use super::now_playing::{NowPlayingOutput, Snapshot};
use crate::segment_loop::LoopMarks;

mod transport;

pub struct PlayerView {
    snap: Snapshot,
    cover: Cover,
    /// True while the user is dragging the scrubber, so incoming positions do
    /// not yank the handle out from under them — the same rule the bar follows.
    scrubbing: bool,
    /// Bumped per drag movement; only the newest commit is honoured.
    scrub_gen: u64,
    /// Enough width to put the artwork beside the controls rather than above.
    wide: bool,
    /// How tall the drawer is, pushed in by [`fill_window`] because a widget
    /// cannot ask how much room it is about to be given. Starts at the floor,
    /// which is the honest answer before the window has been realised.
    room_for: i32,
    /// Whether the queue is showing inside the drawer.
    queue_shown: bool,
    /// The transport, built once and moved between two slots.
    transport: gtk::Box,
    /// The hand-built transport's refreshable pieces. See [`Bits`].
    bits: Option<Bits>,
    /// Keeps the empty right-hand column exactly as wide as the cover column,
    /// so the title and controls stay under the sheet's centred drag handle.
    art_balance_group: gtk::SizeGroup,
    /// The containers `relayout` moves things between. Only available once the
    /// widgets exist, which is after `view_output!`.
    slots: Option<Slots>,
    /// The artwork's live pixel size. In a `Cell` behind an `Rc` because the
    /// animation callback owns a copy and outlives any one `relayout` — and
    /// reading it back is what lets an interrupted transition resume from
    /// where it actually is rather than snapping to where it started.
    art_px: std::rc::Rc<std::cell::Cell<i32>>,
    /// Drives the artwork between its two sizes. `None` until `init` has a
    /// widget to hang it on: an animation needs a frame clock, and a frame
    /// clock comes from a widget.
    art_anim: Option<adw::TimedAnimation>,
}

/// The places the movable transport and queue can live.
struct Slots {
    queue: adw::ToolbarView,
    queue_wide_rev: gtk::Revealer,
    queue_compact_rev: gtk::Revealer,
    queue_wide: gtk::Box,
    queue_compact: gtk::Box,
    transport_wide: gtk::Box,
    transport_stacked: gtk::Box,
    transport_compact: gtk::Box,
}

/// Everything in the narrow vertical drawer that is not the artwork. Measured
/// at 302px; the wide strip does not use this arithmetic.
///
/// It does not shrink, so it sets the arithmetic for how short the drawer can
/// be: [`SHEET_NARROW_MIN_H`] is this plus the smallest useful cover.
const DRAWER_CHROME_H: i32 = 302;

/// The smallest the artwork may be squeezed to before it stops reading as a
/// record and starts reading as an icon.
const ART_FLOOR: i32 = 96;

/// The drawer height at which the queue stops having room for the cover beside
/// it, and the cover goes.
///
/// Measured with the queue open: at 420px the queue gets four rows *with* the
/// thumbnail, and below that it falls to three and then to one. The thumbnail
/// is 72px and its whole job is saying which record this is — which the blurred
/// backdrop, the title and the artist all still do, and the queue's own list
/// does as well. So below this it is worth more as another row.
///
/// Only ever asked with the queue open. Stacked, the cover *is* the view.
const QUEUE_NEEDS_ROOM: i32 = 420;

/// The wide player is a compact horizontal strip and normally claims one third
/// of the window. A narrow player still needs the vertical composition, so it
/// keeps the older, taller share rather than squeezing its controls together.
const WIDE_WINDOW_DIVISOR: i32 = 3;
const NARROW_WINDOW_NUMERATOR: i32 = 7;
const NARROW_WINDOW_DENOMINATOR: i32 = 10;

/// Floors in logical pixels. Wide puts art and controls beside each other;
/// narrow retains the vertical player's measured chrome plus its smallest art.
const SHEET_WIDE_MIN_H: i32 = 210;
const SHEET_NARROW_MIN_H: i32 = DRAWER_CHROME_H + ART_FLOOR;

fn drawer_height(width: i32, height: i32) -> i32 {
    if width >= WIDE_PX {
        (height / WIDE_WINDOW_DIVISOR).max(SHEET_WIDE_MIN_H)
    } else {
        ((height * NARROW_WINDOW_NUMERATOR) / NARROW_WINDOW_DENOMINATOR).max(SHEET_NARROW_MIN_H)
    }
}

/// Tie the drawer's height to the window's width-sensitive layout.
///
/// `AdwBottomSheet` sizes the sheet to its child's **natural height** and offers
/// no maximum or fraction of its own, so the number has to be computed and
/// pushed down.
///
/// The basis is the toplevel `GdkSurface`'s actual size. It notifies on *every*
/// resize, including tiling and maximising —
/// `GtkWindow:default-height` deliberately does not track those, because it
/// stores the size to restore *to*. And reading the surface keeps this acyclic:
/// our request changes the sheet's height, and the sheet never changes the
/// surface.
///
/// While closed this falls back to the current layout's floor — **not to
/// `-1`**. The
/// request has to come off, or it fights the user dragging the window shorter.
/// But `-1` does not restore the floor `view!` declared with `set_size_request`;
/// it *clears* it, because they are the same property, leaving the
/// `AdwBreakpointBin` with no minimum height and libadwaita warning by name:
///
/// ```text
/// AdwBreakpointBin does not have a minimum height, set the 'height-request'
/// property to specify it
/// ```
///
/// `Rc` rather than `Arc`: these are GTK signal handlers, all on the main
/// thread.
pub fn fill_window(
    window: &adw::ApplicationWindow,
    sheet: &adw::BottomSheet,
    content: &gtk::Widget,
    player: &relm4::Sender<PlayerViewInput>,
) {
    let apply: std::rc::Rc<dyn Fn()> = {
        let (window, sheet, content) = (window.clone(), sheet.clone(), content.clone());
        let player = player.clone();
        std::rc::Rc::new(move || {
            let (width, height) = window
                .surface()
                .map_or((0, 0), |surface| (surface.width(), surface.height()));
            let floor = if width >= WIDE_PX {
                SHEET_WIDE_MIN_H
            } else {
                SHEET_NARROW_MIN_H
            };
            let target = if sheet.is_open() && width > 0 && height > 0 {
                drawer_height(width, height)
            } else {
                floor
            };
            content.set_height_request(target);
            // Told the target rather than measuring itself: a widget cannot
            // ask how much room it is about to be given.
            let _ = player.send(PlayerViewInput::RoomFor(target));
        })
    };

    // One handler at a time, not one per realize.
    //
    // Hiding the window unrealizes it and `Ctrl`+`W` makes that routine (#32),
    // so `realize` fires more than once per session and each firing sees a new
    // `GdkSurface`. Connecting without disconnecting leaves a handler on every
    // surface the window has ever had. They are harmless — a dead surface never
    // notifies, and `set_size_request` no-ops on an unchanged value — but the
    // list only grows, which is the kind of thing that is free until it is not.
    let connected: std::rc::Rc<
        std::cell::RefCell<Option<(gdk::Surface, glib::SignalHandlerId, glib::SignalHandlerId)>>,
    > = std::rc::Rc::new(std::cell::RefCell::new(None));
    window.connect_realize({
        let apply = apply.clone();
        move |window| {
            let Some(surface) = window.surface() else {
                return;
            };
            if let Some((old, height_id, width_id)) = connected.borrow_mut().take() {
                old.disconnect(height_id);
                old.disconnect(width_id);
            }
            apply();
            let height_id = surface.connect_height_notify({
                let apply = apply.clone();
                move |_| apply()
            });
            let width_id = surface.connect_width_notify({
                let apply = apply.clone();
                move |_| apply()
            });
            *connected.borrow_mut() = Some((surface, height_id, width_id));
        }
    });

    sheet.connect_open_notify(move |_| apply());
}

/// Move a widget to a new parent, if it is not already there.
fn reparent(child: &impl IsA<gtk::Widget>, new_parent: &gtk::Box) {
    let child = child.as_ref();
    if child.parent().as_ref() == Some(new_parent.upcast_ref::<gtk::Widget>()) {
        return;
    }
    if let Some(old) = child.parent() {
        if let Some(old) = old.downcast_ref::<gtk::Box>() {
            old.remove(child);
        } else {
            child.unparent();
        }
    }
    new_parent.append(child);
}

/// Below this the artwork goes above the controls instead of beside them.
const WIDE_PX: i32 = 860;
/// Artwork sizes: generous when it is the subject, a thumbnail when the queue
/// has taken the space and it is only there to say which record this is.
const ART_LARGE: i32 = 260;
const ART_WIDE: i32 = 112;
const ART_THUMB: i32 = 72;

/// The queue and artwork share one transition so their movement finishes together.
const QUEUE_ANIM_MS: u32 = 250;
const SCRUB_COMMIT_MS: u64 = 250;

#[derive(Debug, Clone)]
pub enum PlayerViewInput {
    Sync(Box<Snapshot>),
    SegmentLoop(LoopMarks),
    Artwork(Option<std::path::PathBuf>),
    Scrub(f64),
    /// Only the newest scrub commits — the same generation trick the bar's seek
    /// uses, and for the same reason: dragging emits continuously and every
    /// intermediate value would be a seek MusicKit has to service.
    ScrubDone(u64, f64),
    PlayPause,
    Next,
    Previous,
    /// The width breakpoint crossed.
    Wide(bool),
    SetQueueShown(bool),
    /// Flip shuffle. No payload: the value is derived from the mirrored one,
    /// so this view never invents one (rule 3).
    ShuffleClicked,
    /// Cycle repeat from the mirrored mode.
    RepeatClicked,
    SegmentLoopClicked,
    OpenAlbum,
    OpenArtist,
    CopyLink,
    ShowLyrics,
    ToggleFavorite,
    SetSleepTimer(crate::sleep_timer::Choice),
    SetPlaybackRate(f64),
    SleepTimerActive(bool),
    ShowCredits,
    VolumeChanged(f64),
    /// How tall the drawer is about to be. See [`fill_window`].
    RoomFor(i32),
}

#[relm4::component(pub)]
impl SimpleComponent for PlayerView {
    type Init = ();
    type Input = PlayerViewInput;
    type Output = NowPlayingOutput;

    view! {
        #[name = "root"]
        adw::BreakpointBin {
            // Low enough that the drawer never becomes the window's floor.
            // The whole point of the compact layout is that the app can be
            // tiled to half a screen, and a minimum here would undo that.
            //
            // How tall it actually opens is [`fill_window`]'s business, not
            // this number's: this is the floor it may never go under, that is
            // the share of the window it asks for.
            set_size_request: (300, SHEET_WIDE_MIN_H),

            #[wrap(Some)]
            set_child = &gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                add_css_class: "np-sheet",

                // The player column and, when there is width for it, the queue
                // beside it. Horizontal always: what changes is whether the
                // queue column next to it is showing.
                #[name = "top"]
                gtk::Box {
                    // No padding and no spacing **here**: the queue is one of
                    // this box's two children and it wants to sit flush against
                    // the drawer's edge, the way it did as a sidebar. Padding
                    // is the player column's own business, below.
                    set_spacing: 0,
                    // Only claims the height when it is the thing worth
                    // looking at. In the compact layout with the queue open
                    // the queue is, and this shrinks to the thumbnail and the
                    // title above it.
                    #[watch]
                    set_vexpand: model.stacked(),

                    // Narrow: artwork, metadata and controls form a column.
                    // Wide: artwork sits beside a compact metadata/control
                    // column so the drawer can stay one third of the window.
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        set_spacing: 16,
                        set_margin_start: 24,
                        set_margin_end: 24,
                        // Not on `top`: the queue shares that box and wants to
                        // stay flush. The drawer's drag handle is drawn over
                        // the top edge, so without this the artwork starts
                        // under it and, in the compact layout, the title is
                        // written straight through it.
                        set_margin_top: 24,
                        // Generous under a full-height player, and pure gap when
                        // the queue is immediately below it — the queue brings
                        // its own header, which is separation enough.
                        #[watch]
                        set_margin_bottom: if model.stacked() { 24 } else { 8 },
                        // Centred in the drawer when it is a column, pinned to
                        // the top when the queue is below it and wants the rest.
                        #[watch]
                        set_valign: if model.stacked() {
                            gtk::Align::Center
                        } else {
                            gtk::Align::Start
                        },

                        // Artwork above metadata in the narrow player, beside
                        // it in the wide strip or a compact queue layout.
                        #[name = "head"]
                        gtk::Box {
                            set_spacing: 16,
                            #[watch]
                            set_orientation: if model.stacked() {
                                gtk::Orientation::Vertical
                            } else {
                                gtk::Orientation::Horizontal
                            },

                            #[name = "art_column"]
                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_spacing: 2,
                                #[watch]
                                set_halign: if model.stacked() {
                                    gtk::Align::Center
                                } else {
                                    gtk::Align::Start
                                },
                                #[watch]
                                set_valign: if model.stacked() {
                                    gtk::Align::End
                                } else {
                                    gtk::Align::Center
                                },

                                #[name = "art_slot"]
                                gtk::Button {
                                    add_css_class: "flat",
                                    add_css_class: "player-cover-link",
                                    set_has_frame: false,
                                    set_tooltip_text: Some("Open album"),
                                    #[watch]
                                    set_sensitive: model.snap.catalog_id.is_some(),
                                    connect_clicked => PlayerViewInput::OpenAlbum,
                                },

                                gtk::Button {
                                    add_css_class: "flat",
                                    add_css_class: "player-album-link",
                                    add_css_class: "player-metadata-link",
                                    set_tooltip_text: Some("Open album"),
                                    #[watch]
                                    set_visible: model.wide && !model.snap.album.is_empty(),
                                    #[watch]
                                    set_sensitive: model.snap.catalog_id.is_some(),
                                    connect_clicked => PlayerViewInput::OpenAlbum,

                                    #[wrap(Some)]
                                    set_child = &gtk::Label {
                                        set_ellipsize: gtk::pango::EllipsizeMode::End,
                                        set_max_width_chars: 18,
                                        set_use_markup: false,
                                        add_css_class: "heading",
                                        #[watch]
                                        set_label: &model.snap.album,
                                    },
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_hexpand: true,
                                set_valign: gtk::Align::Center,
                                set_spacing: 2,
                                #[watch]
                                set_halign: if model.centred_text() {
                                    gtk::Align::Center
                                } else {
                                    gtk::Align::Start
                                },

                                // Crossfaded, not flipped.
                                //
                                // These are two readings of one state, and a
                                // queue emptying used to cut between them: the
                                // title vanishing and the grey bars appearing
                                // in the same frame. A `GtkStack` dissolves
                                // between its pages instead, and the page is
                                // chosen in `post_view` rather than by a
                                // `#[watch]`, because a transition is an
                                // animation and animated properties are
                                // written on an edge.
                                #[name = "meta_stack"]
                                gtk::Stack {
                                    set_transition_type: gtk::StackTransitionType::Crossfade,
                                    set_transition_duration: SWAP_MS,
                                    // **A stack measures its largest child,
                                    // showing or not — on both axes.** Across,
                                    // the skeleton was setting the drawer's
                                    // minimum width with a track playing, which
                                    // is what `AdwBreakpointBin` complained
                                    // about: 376px asked for against 360.
                                    //
                                    // Down, it was pinning this block to the
                                    // skeleton's own 50px, so shrinking the
                                    // title beside the queue bought 6px of the
                                    // 26 it should have. Both, or neither is
                                    // worth setting.
                                    set_hhomogeneous: false,
                                    set_vhomogeneous: false,

                                    // They differ in **length**, not in weight:
                                    // a title runs long and an artist is
                                    // usually a name. Each carries its own
                                    // `halign` because a vertical `GtkBox`
                                    // fills its children across and
                                    // `set_size_request` is only a minimum, so
                                    // without it both drew the same length.
                                    add_named[Some("empty")] = &gtk::Box {
                                        set_orientation: gtk::Orientation::Vertical,
                                        set_spacing: 10,
                                        set_margin_top: 4,
                                        set_margin_bottom: 4,
                                        set_valign: gtk::Align::Center,

                                        gtk::Box {
                                            #[watch]
                                            set_halign: if model.centred_text() {
                                                gtk::Align::Center
                                            } else {
                                                gtk::Align::Start
                                            },
                                            // 240/120 before, which put a
                                            // 240px floor under a drawer that
                                            // has to fit a 360px window. Two
                                            // grey bars say where the title
                                            // goes; they need not be its width.
                                            set_size_request: (150, 16),
                                            add_css_class: "np-skeleton",
                                        },
                                        gtk::Box {
                                            #[watch]
                                            set_halign: if model.centred_text() {
                                                gtk::Align::Center
                                            } else {
                                                gtk::Align::Start
                                            },
                                            set_size_request: (90, 16),
                                            add_css_class: "np-skeleton",
                                        },
                                    },

                                    add_named[Some("track")] = &gtk::Box {
                                        set_orientation: gtk::Orientation::Vertical,
                                        set_spacing: 2,
                                        set_valign: gtk::Align::Center,

                                        // **Two sizes, because it is doing two
                                        // jobs.** Stacked, this is the caption
                                        // under a large cover and the largest
                                        // type in the app is right. Beside the
                                        // queue it is a label on a strip, and
                                        // `title-1` over `title-4` was 56px of
                                        // heading in a drawer with room for one
                                        // queue row — the block that ate the
                                        // 72px hiding the thumbnail was meant
                                        // to free.
                                        gtk::Box {
                                            set_orientation: gtk::Orientation::Horizontal,
                                            set_spacing: 4,
                                            #[watch]
                                            set_halign: if model.centred_text() {
                                                gtk::Align::Center
                                            } else {
                                                gtk::Align::Start
                                            },

                                            // Balance the credits button so the
                                            // title itself, rather than the
                                            // title-and-button pair, is centred.
                                            gtk::Box {
                                                set_size_request: (34, 1),
                                                #[watch]
                                                set_visible: model.snap.catalog_id.is_some(),
                                            },

                                            gtk::Label {
                                                set_ellipsize: gtk::pango::EllipsizeMode::End,
                                                set_max_width_chars: 28,
                                                set_use_markup: false,
                                                #[watch]
                                                set_css_classes: if model.stacked() {
                                                    &["title-1"]
                                                } else {
                                                    &["title-4"]
                                                },
                                                #[watch]
                                                set_label: &model.snap.title,
                                            },
                                            gtk::Button {
                                                set_icon_name: "avatar-default-symbolic",
                                                set_tooltip_text: Some("Song credits"),
                                                add_css_class: "flat",
                                                add_css_class: "circular",
                                                add_css_class: "player-state-control",
                                                #[watch]
                                                set_visible: model.snap.catalog_id.is_some(),
                                                #[watch]
                                                set_sensitive: model.snap.catalog_id.is_some(),
                                                connect_clicked => PlayerViewInput::ShowCredits,
                                            },
                                        },
                                        gtk::Box {
                                            set_orientation: gtk::Orientation::Vertical,
                                            set_spacing: 0,
                                            #[watch]
                                            set_halign: if model.centred_text() {
                                                gtk::Align::Center
                                            } else {
                                                gtk::Align::Start
                                            },

                                            gtk::Button {
                                                add_css_class: "flat",
                                                add_css_class: "player-album-link",
                                                add_css_class: "player-metadata-link",
                                                set_tooltip_text: Some("Open artist"),
                                                #[watch]
                                                set_visible: !model.snap.artist.is_empty(),
                                                #[watch]
                                                set_sensitive: model.snap.catalog_id.is_some(),
                                                connect_clicked => PlayerViewInput::OpenArtist,

                                                #[wrap(Some)]
                                                set_child = &gtk::Label {
                                                    set_ellipsize: gtk::pango::EllipsizeMode::End,
                                                    set_max_width_chars: 34,
                                                    set_use_markup: false,
                                                    #[watch]
                                                    set_css_classes: if model.stacked() {
                                                        &["title-2"]
                                                    } else {
                                                        &["heading"]
                                                    },
                                                    #[watch]
                                                    set_label: &model.snap.artist,
                                                },
                                            },
                                            gtk::Button {
                                                add_css_class: "flat",
                                                add_css_class: "player-album-link",
                                                add_css_class: "player-metadata-link",
                                                set_tooltip_text: Some("Open album"),
                                                #[watch]
                                                set_visible: !model.wide && !model.snap.album.is_empty(),
                                                #[watch]
                                                set_sensitive: model.snap.catalog_id.is_some(),
                                                connect_clicked => PlayerViewInput::OpenAlbum,

                                                #[wrap(Some)]
                                                set_child = &gtk::Label {
                                                    set_ellipsize: gtk::pango::EllipsizeMode::End,
                                                    set_max_width_chars: 34,
                                                    set_use_markup: false,
                                                    #[watch]
                                                    set_css_classes: if model.stacked() {
                                                        &["title-2"]
                                                    } else {
                                                        &["heading"]
                                                    },
                                                    #[watch]
                                                    set_label: &model.snap.album,
                                                },
                                            },
                                        },
                                    },
                                },

                                // On a wide window the whole player becomes a
                                // low horizontal strip: cover, metadata, then
                                // controls. Keeping the transport in the same
                                // row is what lets the drawer be one third of
                                // the window without clipping or scrolling.
                                #[name = "transport_wide"]
                                gtk::Box {
                                    set_orientation: gtk::Orientation::Vertical,
                                    set_valign: gtk::Align::Center,
                                    #[watch]
                                    set_visible: model.wide,
                                },
                            },

                            // The cover occupies the left of the wide strip.
                            // Mirror its exact allocated width on the right so
                            // metadata and transport remain centred to the
                            // BottomSheet handle rather than to the leftover
                            // space beside the cover.
                            #[name = "art_balance"]
                            gtk::Box {
                                set_width_request: ART_WIDE,
                                #[watch]
                                set_visible: model.wide,
                            },
                        },

                        // Where the transport lives in the narrow vertical
                        // player.
                        //
                        // **Hidden when it is that one.** An empty box is still
                        // a child, and a visible child takes its share of the
                        // column's 16px spacing — 16px of nothing between the
                        // title and the queue, which is where the queue could
                        // have put a third of a row.
                        #[name = "transport_stacked"]
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            #[watch]
                            set_visible: model.stacked(),
                        },
                    },

                    // Where the queue lives when it can be a column of its own.
                    //
                    // Wrapped in a `GtkRevealer` so it slides in from the edge
                    // rather than appearing. That also animates everything
                    // beside it for free: the revealer grows its own width
                    // over the transition, so the player column is squeezed
                    // continuously instead of jumping to its new size.
                    #[name = "queue_wide_rev"]
                    gtk::Revealer {
                        set_transition_type: gtk::RevealerTransitionType::SlideLeft,
                        set_transition_duration: QUEUE_ANIM_MS,

                        #[wrap(Some)]
                        #[name = "queue_wide"]
                        set_child = &gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            #[watch]
                            // Only when it is a column of its own; in the
                            // compact layout the queue is the full width and
                            // must not carry a floor.
                            set_width_request: if model.wide { 320 } else { -1 },
                        },
                    },
                },

                // ...and where each goes when it cannot. Upwards here, because
                // in the compact layout the queue rises from the foot of the
                // drawer rather than in from the side.
                #[name = "queue_compact_rev"]
                gtk::Revealer {
                    set_transition_type: gtk::RevealerTransitionType::SlideUp,
                    set_transition_duration: QUEUE_ANIM_MS,
                    // **Only while it is actually showing.** A collapsed
                    // revealer draws nothing but still claims its share of the
                    // expansion, so leaving this on meant this and `top` split
                    // the drawer's height between them — and the player,
                    // centred inside its half, sat in the upper part of the
                    // drawer with the rest of it empty below.
                    #[watch]
                    set_vexpand: model.queue_shown && !model.wide,

                    #[wrap(Some)]
                    #[name = "queue_compact"]
                    set_child = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_vexpand: true,
                    },
                },

                #[name = "transport_compact"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_margin_start: 24,
                    set_margin_end: 24,
                    set_margin_bottom: 18,
                    #[watch]
                    set_visible: !model.wide && model.queue_shown,
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut model = PlayerView {
            snap: Snapshot::default(),
            cover: Cover::new(ART_LARGE),
            scrubbing: false,
            scrub_gen: 0,
            wide: true,
            room_for: SHEET_WIDE_MIN_H,
            queue_shown: false,
            transport: gtk::Box::new(gtk::Orientation::Vertical, 12),
            slots: None,
            bits: None,
            art_balance_group: gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal),
            art_px: std::rc::Rc::new(std::cell::Cell::new(ART_LARGE)),
            art_anim: None,
        };
        // Rule 5: no `.expect()` here. A missing handover is a construction
        // order mistake rather than a runtime condition, so it should never
        // happen — but "should never happen" is exactly what the rule is about,
        // and a drawer with an empty queue pane is a far better failure than a
        // player that will not start. It is loud in the log and silent to the
        // user, who cannot act on it either way.
        let queue = QUEUE_SLOT.with(|q| q.borrow().clone()).unwrap_or_else(|| {
            tracing::error!("no queue widget was handed over; the drawer's queue will be empty");
            adw::ToolbarView::new()
        });
        let widgets = view_output!();
        model.art_balance_group.add_widget(&widgets.art_column);
        model.art_balance_group.add_widget(&widgets.art_balance);
        model.cover.attach_to_button(&widgets.art_slot);
        model.cover.empty_sleeve(ART_LARGE);

        model.bits = Some(build_transport(&model.transport, &sender));

        // The artwork has no widget that will animate a size request for it,
        // so this is the one place the drawer drives a value by hand. The
        // callback is deliberately idempotent — `AdwTimedAnimation` can hand
        // back the same rounded pixel twice on consecutive frames, and
        // re-setting the size would queue a resize for no change.
        let px = model.art_px.clone();
        let cover = model.cover.clone();
        let anim = adw::TimedAnimation::new(
            &widgets.art_slot,
            f64::from(ART_LARGE),
            f64::from(ART_LARGE),
            QUEUE_ANIM_MS,
            adw::CallbackAnimationTarget::new(move |value| {
                let size = value.round() as i32;
                if px.replace(size) != size {
                    cover.resize(size);
                }
            }),
        );
        anim.set_easing(adw::Easing::EaseOutCubic);
        model.art_anim = Some(anim);
        model.slots = Some(Slots {
            queue,
            queue_wide_rev: widgets.queue_wide_rev.clone(),
            queue_compact_rev: widgets.queue_compact_rev.clone(),
            queue_wide: widgets.queue_wide.clone(),
            queue_compact: widgets.queue_compact.clone(),
            transport_wide: widgets.transport_wide.clone(),
            transport_stacked: widgets.transport_stacked.clone(),
            transport_compact: widgets.transport_compact.clone(),
        });
        model.relayout();

        // One breakpoint. The other decision — whether the queue is showing —
        // is the user's, and combining the two is what `relayout` is for.
        if let Ok(condition) =
            adw::BreakpointCondition::parse(&format!("max-width: {}px", WIDE_PX - 1))
        {
            let breakpoint = adw::Breakpoint::new(condition);
            let narrowed = sender.clone();
            breakpoint.connect_apply(move |_| narrowed.input(PlayerViewInput::Wide(false)));
            let widened = sender.clone();
            breakpoint.connect_unapply(move |_| widened.input(PlayerViewInput::Wide(true)));
            widgets.root.add_breakpoint(breakpoint);
        } else {
            tracing::warn!("unparsable breakpoint; the player will not adapt");
        }

        ComponentParts { model, widgets }
    }

    /// Which face the metadata shows.
    ///
    /// Here rather than as a `#[watch] set_visible_child_name`, and guarded,
    /// because a stack with a transition is an animation: writing it is asking
    /// for a cross-fade, and a `#[watch]` would ask on every message. Same
    /// rule as the app's animated properties, for the same reason.
    fn post_view(&self, widgets: &mut Self::Widgets) {
        let want = if self.snap.active { "track" } else { "empty" };
        if widgets.meta_stack.visible_child_name().as_deref() != Some(want) {
            widgets.meta_stack.set_visible_child_name(want);
        }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            PlayerViewInput::Sync(snap) => {
                // While dragging, the position is the user's, not the player's.
                let position = if self.scrubbing {
                    self.snap.position_ms
                } else {
                    snap.position_ms
                };
                self.snap = *snap;
                self.snap.position_ms = position;
                self.refresh_transport();
            }
            PlayerViewInput::SegmentLoop(marks) => self.refresh_segment_loop(marks),
            PlayerViewInput::Artwork(path) => match path {
                Some(path) => self.cover.set_file(&path),
                // The same empty case the bar draws, at the drawer's size —
                // a place the artwork goes, rather than a bare glyph adrift in
                // a 260px square.
                None => self.cover.empty_sleeve(self.art_px.get()),
            },
            PlayerViewInput::Scrub(v) => {
                self.scrubbing = true;
                self.snap.position_ms = v as u64;
                self.refresh_transport();
                // Debounced: a drag emits on every motion event, and seeking on
                // each one would have MusicKit re-buffering continuously.
                self.scrub_gen = self.scrub_gen.wrapping_add(1);
                let generation = self.scrub_gen;
                let sender = sender.clone();
                gtk::glib::timeout_add_local_once(
                    std::time::Duration::from_millis(SCRUB_COMMIT_MS),
                    move || sender.input(PlayerViewInput::ScrubDone(generation, v)),
                );
            }
            PlayerViewInput::ScrubDone(generation, v) => {
                // A later drag supersedes this one.
                if generation != self.scrub_gen {
                    return;
                }
                self.scrubbing = false;
                let _ = sender.output(NowPlayingOutput::Seek(v as u64));
            }
            PlayerViewInput::PlayPause => {
                let _ = sender.output(NowPlayingOutput::PlayPause);
            }
            PlayerViewInput::Next => {
                let _ = sender.output(NowPlayingOutput::Next);
            }
            PlayerViewInput::Previous => {
                let _ = sender.output(NowPlayingOutput::Previous);
            }
            PlayerViewInput::Wide(wide) => {
                if self.wide != wide {
                    self.wide = wide;
                    self.relayout();
                }
            }
            PlayerViewInput::SetQueueShown(shown) => {
                if self.queue_shown == shown {
                    return; // our own echo
                }
                self.queue_shown = shown;
                self.relayout();
            }
            PlayerViewInput::ShuffleClicked => {
                let _ = sender.output(NowPlayingOutput::SetShuffle(!self.snap.shuffle));
            }
            PlayerViewInput::RoomFor(height) => {
                if self.room_for != height {
                    self.room_for = height;
                    self.relayout();
                }
            }
            PlayerViewInput::RepeatClicked => {
                let _ = sender.output(NowPlayingOutput::SetRepeat(self.snap.repeat.next()));
            }
            PlayerViewInput::SegmentLoopClicked => {
                let _ = sender.output(NowPlayingOutput::CycleSegmentLoop);
            }
            PlayerViewInput::OpenAlbum => {
                let _ = sender.output(NowPlayingOutput::OpenAlbum);
            }
            PlayerViewInput::OpenArtist => {
                let _ = sender.output(NowPlayingOutput::OpenArtist);
            }
            PlayerViewInput::CopyLink => {
                let _ = sender.output(NowPlayingOutput::CopyLink);
            }
            PlayerViewInput::ShowLyrics => {
                let _ = sender.output(NowPlayingOutput::ShowLyrics);
            }
            PlayerViewInput::ToggleFavorite => {
                let _ = sender.output(NowPlayingOutput::ToggleFavorite);
            }
            PlayerViewInput::SetSleepTimer(choice) => {
                let _ = sender.output(NowPlayingOutput::SetSleepTimer(choice));
            }
            PlayerViewInput::SetPlaybackRate(rate) => {
                let _ = sender.output(NowPlayingOutput::SetPlaybackRate(rate));
            }
            PlayerViewInput::SleepTimerActive(active) => self.refresh_sleep_timer(active),
            PlayerViewInput::ShowCredits => {
                let _ = sender.output(NowPlayingOutput::ShowCredits);
            }
            PlayerViewInput::VolumeChanged(v) => {
                // Same shape as the bar's. `refresh` blocks this handler while
                // it writes, so everything arriving here is a real gesture; the
                // guard is idempotence, not echo-catching.
                if crate::components::now_playing::volume_is_new(v, self.snap.volume) {
                    self.snap.volume = v;
                    let _ = sender.output(NowPlayingOutput::SetVolume(v));
                }
            }
        }
    }
}

thread_local! {
    /// Where the queue widget is left for `init` to collect.
    ///
    /// relm4's `view!` builds the widget tree before the model exists, and the
    /// queue is a sibling component owned by the app — there is no init payload
    /// that can carry a `&Widget` through. Handing it over on this cell keeps
    /// the queue a *moved* component rather than a second implementation, which
    /// is what issue #18 asked for.
    static QUEUE_SLOT: std::cell::RefCell<Option<adw::ToolbarView>> =
        const { std::cell::RefCell::new(None) };
}

/// Lend the queue widget to the player view being built next.
pub fn hand_over_queue(queue: adw::ToolbarView) {
    QUEUE_SLOT.with(|q| *q.borrow_mut() = Some(queue));
}

impl PlayerView {
    /// Whether the artwork sits **above** the rest of the player rather than
    /// beside it.
    ///
    /// Narrow without a queue is the one vertical composition. Wide windows
    /// use a compact horizontal strip; a narrow queue uses a thumbnail and
    /// gives the remaining height to its rows.
    fn stacked(&self) -> bool {
        !self.wide && !self.queue_shown
    }

    /// The main player reads as a centred composition. Only the compact queue
    /// layout aligns metadata left beside its thumbnail.
    fn centred_text(&self) -> bool {
        self.wide || self.stacked()
    }

    /// Put the transport and the queue where this layout wants them.
    ///
    /// They are **moved, not duplicated**. The transport is one widget with one
    /// set of signal handlers, and the queue is the app's own `QueueView` — a
    /// second copy of either would be two things claiming to be the same
    /// player, which is the failure this whole component is arranged to avoid.
    ///
    /// Called on a breakpoint or a toggle, so a handful of times a session
    /// rather than per frame.
    fn relayout(&self) {
        let Some(slots) = self.slots.as_ref() else {
            return;
        };
        // Wide keeps the transport beside the cover. Narrow puts it under the
        // cover, or below the queue when that queue needs the drawer's height.
        let transport_home = if self.wide {
            &slots.transport_wide
        } else if self.stacked() {
            &slots.transport_stacked
        } else {
            &slots.transport_compact
        };
        let queue_home = if self.wide {
            &slots.queue_wide
        } else {
            &slots.queue_compact
        };
        reparent(&self.transport, transport_home);
        reparent(&slots.queue, queue_home);

        // The artwork is large in the narrow player, restrained in the wide
        // strip, and only a thumbnail beside a narrow queue.
        // Beside the queue on a short drawer the cover goes entirely: its 72px
        // buys a row and a bit, and nothing that matters is lost — the sleeve is
        // still the backdrop behind all of this, and the title still names it.
        let room_for_cover = self.wide || self.stacked() || self.room_for >= QUEUE_NEEDS_ROOM;
        self.cover.set_shown(room_for_cover);
        if room_for_cover {
            self.resize_cover(if self.stacked() {
                (self.room_for - DRAWER_CHROME_H).clamp(ART_FLOOR, ART_LARGE)
            } else if self.wide {
                ART_WIDE
            } else {
                ART_THUMB
            });
        }

        // One control at a time: the transport's button opens the queue, the
        // queue's own header closes it. Two buttons, but never both on screen,
        // which is what keeps it from reading as a duplicate.
        if let Some(bits) = self.bits.as_ref() {
            bits.set_secondary_visible(!self.queue_shown);
        }

        // The revealers decide what is on screen now, so the queue itself
        // stays visible: hiding it would pre-empt the very transition the
        // revealer is there to play, and the close would be a cut.
        slots.queue.set_visible(true);
        slots
            .queue_wide_rev
            .set_reveal_child(self.queue_shown && self.wide);
        slots
            .queue_compact_rev
            .set_reveal_child(self.queue_shown && !self.wide);
    }

    /// Send the artwork to `target`, animating unless there is nothing to
    /// animate with.
    ///
    /// Interrupting is the case that matters — toggling the queue twice
    /// quickly — so the new run starts from the size on screen *now*, which is
    /// what `art_px` holds, rather than from the size it was meant to be.
    fn resize_cover(&self, target: i32) {
        if self.art_px.get() == target {
            return;
        }
        let Some(anim) = self.art_anim.as_ref() else {
            self.art_px.set(target);
            self.cover.resize(target);
            return;
        };
        anim.pause();
        anim.set_value_from(f64::from(self.art_px.get()));
        anim.set_value_to(f64::from(target));
        anim.play();
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn wide_drawer_uses_one_third_with_a_small_floor() {
        assert_eq!(drawer_height(1000, 900), 300);
        assert_eq!(drawer_height(1000, 600), SHEET_WIDE_MIN_H);
    }

    #[test]
    fn narrow_drawer_keeps_room_for_the_vertical_player() {
        assert_eq!(drawer_height(600, 800), 560);
        assert_eq!(drawer_height(600, 400), SHEET_NARROW_MIN_H);
    }
}
