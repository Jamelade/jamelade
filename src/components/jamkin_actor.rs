// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A tiny frame-sprite actor shared by the full lyrics page and Jamkin Mode.
//!
//! The timer exists only while the picture is mapped, playback is active and
//! GTK's animation setting permits motion. This is intentionally not an
//! infinite CSS animation: an earlier decorative loop cost a fifth of one CPU
//! core even while nobody was looking at it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use relm4::adw::prelude::*;
use relm4::gtk::{self, gdk_pixbuf};

use crate::companion::Companion;
use crate::settings::JamkinQuality;

type SourceFrames = Rc<Vec<gdk_pixbuf::Pixbuf>>;
type SourceFrameKey = (Companion, bool);
type WeakSourceFrames = Weak<Vec<gdk_pixbuf::Pixbuf>>;
type SourceFrameCache = RefCell<HashMap<SourceFrameKey, WeakSourceFrames>>;

thread_local! {
    /// Every visible surface needs its own scaled textures, but decoding the
    /// same six 1280 px PNGs three times would waste roughly 80 MB. Weak cache
    /// entries share one decoded master set while actors use it and retain
    /// nothing after the last actor switches companion or quality.
    static SOURCE_FRAME_CACHE: SourceFrameCache = RefCell::new(HashMap::new());
}

struct ActorCore {
    picture: gtk::Picture,
    companion: Cell<Companion>,
    quality: Cell<JamkinQuality>,
    high_resolution: Cell<bool>,
    /// Shared full-resolution local masters. Keeping these in memory lets the
    /// desktop size slider resample without repeatedly reading disk, while the
    /// weak cache above avoids decoding one set separately for every surface.
    source_frames: RefCell<SourceFrames>,
    frames: RefCell<Vec<gtk::gdk::Texture>>,
    frame: Cell<usize>,
    size: Cell<i32>,
    playing: Cell<bool>,
    reduced_motion: Cell<bool>,
    mapped: Cell<bool>,
    timer: RefCell<Option<gtk::glib::SourceId>>,
}

impl ActorCore {
    fn animations_enabled() -> bool {
        gtk::Settings::default()
            .map(|settings| settings.is_gtk_enable_animations())
            .unwrap_or(true)
    }

    fn wants_animation(&self) -> bool {
        self.playing.get()
            && !self.reduced_motion.get()
            && self.mapped.get()
            && self.frames.borrow().len() > 1
            && Self::animations_enabled()
    }

    fn paint(&self, index: usize) {
        let frames = self.frames.borrow();
        let Some(frame) = frames.get(index) else {
            return;
        };
        self.picture.set_paintable(Some(frame));
        self.frame.set(index);
    }

    /// Give every paintable the requested intrinsic size.
    ///
    /// A GtkPicture width request is only a *minimum*. The old 320px textures
    /// therefore kept their natural 320px allocation through most of the size
    /// slider, making the control appear broken. Resampling the bundled
    /// local frames changes their natural size as well as the request, so both
    /// the visible sprite and its transparent click surface scale together.
    fn rebuild_scaled_frames(&self) {
        let size = self.size.get().max(1);
        let frames: Vec<_> = self
            .source_frames
            .borrow()
            .iter()
            .filter_map(|source| {
                source
                    .scale_simple(size, size, gdk_pixbuf::InterpType::Bilinear)
                    .map(|scaled| {
                        let format = if scaled.has_alpha() {
                            gtk::gdk::MemoryFormat::R8g8b8a8
                        } else {
                            gtk::gdk::MemoryFormat::R8g8b8
                        };
                        gtk::gdk::MemoryTexture::new(
                            scaled.width(),
                            scaled.height(),
                            format,
                            &scaled.read_pixel_bytes(),
                            scaled.rowstride() as usize,
                        )
                        .upcast()
                    })
            })
            .collect();
        let visible = !frames.is_empty();
        *self.frames.borrow_mut() = frames;
        self.picture.set_visible(visible);
        self.frame.set(0);
        self.paint(0);
    }

    fn stop_timer(&self) {
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
    }

    fn sync_timer(core: &Rc<Self>) {
        if !core.wants_animation() {
            core.stop_timer();
            core.paint(0);
            return;
        }
        if core.timer.borrow().is_some() {
            return;
        }

        let interval = core.companion.get().animation_interval_ms();
        let weak: Weak<Self> = Rc::downgrade(core);
        let timer =
            gtk::glib::timeout_add_local(std::time::Duration::from_millis(interval), move || {
                let Some(core) = weak.upgrade() else {
                    return gtk::glib::ControlFlow::Break;
                };
                if !core.wants_animation() {
                    return gtk::glib::ControlFlow::Break;
                }
                let count = core.frames.borrow().len();
                core.paint((core.frame.get() + 1) % count);
                gtk::glib::ControlFlow::Continue
            });
        *core.timer.borrow_mut() = Some(timer);
    }

    fn read_animation(companion: Companion, high_resolution: bool) -> Vec<gdk_pixbuf::Pixbuf> {
        let animated: Vec<_> = companion
            .animation_frame_paths(high_resolution)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|path| match gdk_pixbuf::Pixbuf::from_file(&path) {
                Ok(frame) => Some(frame),
                Err(_) => {
                    tracing::warn!("could not load Jamkin animation frame");
                    None
                }
            })
            .collect();
        animated
    }

    fn load(companion: Companion, high_resolution: bool) -> SourceFrames {
        let key = (companion, high_resolution);
        if let Some(frames) = SOURCE_FRAME_CACHE
            .with(|cache| cache.borrow().get(&key).and_then(std::rc::Weak::upgrade))
        {
            return frames;
        }

        let mut frames = Self::read_animation(companion, high_resolution);
        if frames.len() != 6 && high_resolution {
            // A partial optional HQ install must not blank or flash the actor.
            // The complete original loop remains the safe local fallback.
            frames = Self::read_animation(companion, false);
        }
        if frames.len() != 6 {
            frames = companion
                .image_path()
                .and_then(|path| match gdk_pixbuf::Pixbuf::from_file(&path) {
                    Ok(frame) => Some(frame),
                    Err(_) => {
                        tracing::warn!("could not load Jamkin artwork");
                        None
                    }
                })
                .into_iter()
                .collect();
        }

        let frames = Rc::new(frames);
        SOURCE_FRAME_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            cache.retain(|_, frames| frames.strong_count() > 0);
            cache.insert(key, Rc::downgrade(&frames));
        });
        frames
    }

    fn resolved_high_resolution(&self) -> bool {
        self.quality
            .get()
            .uses_high_resolution(self.size.get(), self.picture.scale_factor().max(1))
    }

    /// Reload only when Auto crossed a size/scale boundary or the preference
    /// explicitly changed set. Returns whether scaled textures need rebuilding.
    fn ensure_source_quality(&self) -> bool {
        let high_resolution = self.resolved_high_resolution();
        if self.high_resolution.get() == high_resolution && !self.source_frames.borrow().is_empty()
        {
            return false;
        }
        self.high_resolution.set(high_resolution);
        *self.source_frames.borrow_mut() = Self::load(self.companion.get(), high_resolution);
        true
    }

    fn refresh_quality(core: &Rc<Self>) {
        core.stop_timer();
        if core.ensure_source_quality() {
            core.rebuild_scaled_frames();
        }
        Self::sync_timer(core);
    }
}

pub struct JamkinActor {
    core: Rc<ActorCore>,
}

impl JamkinActor {
    pub fn new(companion: Companion, size: i32, quality: JamkinQuality) -> Self {
        let picture = gtk::Picture::builder()
            .width_request(size)
            .height_request(size)
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .css_classes(["jamkin-sprite"])
            .build();
        let core = Rc::new(ActorCore {
            picture,
            companion: Cell::new(companion),
            quality: Cell::new(quality),
            high_resolution: Cell::new(false),
            source_frames: RefCell::new(Rc::new(Vec::new())),
            frames: RefCell::new(Vec::new()),
            frame: Cell::new(0),
            size: Cell::new(size.max(1)),
            playing: Cell::new(false),
            reduced_motion: Cell::new(false),
            mapped: Cell::new(false),
            timer: RefCell::new(None),
        });

        {
            let weak = Rc::downgrade(&core);
            core.picture.connect_map(move |_| {
                if let Some(core) = weak.upgrade() {
                    core.mapped.set(true);
                    ActorCore::refresh_quality(&core);
                }
            });
        }
        {
            let weak = Rc::downgrade(&core);
            core.picture.connect_unmap(move |_| {
                if let Some(core) = weak.upgrade() {
                    core.mapped.set(false);
                    ActorCore::sync_timer(&core);
                }
            });
        }
        {
            let weak = Rc::downgrade(&core);
            core.picture.connect_scale_factor_notify(move |_| {
                if let Some(core) = weak.upgrade() {
                    ActorCore::refresh_quality(&core);
                }
            });
        }
        if let Some(settings) = gtk::Settings::default() {
            let weak = Rc::downgrade(&core);
            settings.connect_gtk_enable_animations_notify(move |_| {
                if let Some(core) = weak.upgrade() {
                    ActorCore::sync_timer(&core);
                }
            });
        }

        let actor = Self { core };
        actor.set_companion(companion);
        actor
    }

    pub fn widget(&self) -> &gtk::Picture {
        &self.core.picture
    }

    pub fn set_companion(&self, companion: Companion) {
        self.core.stop_timer();
        self.core.companion.set(companion);
        // The Desktop Jamkin already opens its lyric bubble on hover. A GTK
        // name tooltip competes with that bubble and lands across the sprite.
        self.core.picture.set_tooltip_text(None);
        let high_resolution = self.core.resolved_high_resolution();
        self.core.high_resolution.set(high_resolution);
        *self.core.source_frames.borrow_mut() = ActorCore::load(companion, high_resolution);
        self.core.rebuild_scaled_frames();
        ActorCore::sync_timer(&self.core);
    }

    pub fn set_quality(&self, quality: JamkinQuality) {
        if self.core.quality.replace(quality) == quality {
            return;
        }
        ActorCore::refresh_quality(&self.core);
    }

    pub fn set_playing(&self, playing: bool) {
        if self.core.playing.replace(playing) == playing {
            return;
        }
        ActorCore::sync_timer(&self.core);
    }

    /// Freeze decorative frames without changing the selected artwork. The
    /// system-wide GTK animation setting remains an independent upper bound.
    pub fn set_reduced_motion(&self, reduced: bool) {
        if self.core.reduced_motion.replace(reduced) == reduced {
            return;
        }
        ActorCore::sync_timer(&self.core);
    }

    /// Resize from the in-memory masters without re-reading the frame files.
    pub fn set_size(&self, size: i32) {
        let size = size.max(1);
        if self.core.size.replace(size) == size {
            return;
        }
        self.core.stop_timer();
        self.core.picture.set_width_request(size);
        self.core.picture.set_height_request(size);
        self.core.ensure_source_quality();
        self.core.rebuild_scaled_frames();
        ActorCore::sync_timer(&self.core);
    }
}

impl Drop for JamkinActor {
    fn drop(&mut self) {
        self.core.stop_timer();
    }
}
