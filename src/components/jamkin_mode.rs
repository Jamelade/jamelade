// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Optional floating Jamkin Mode: one movable sprite and a hover lyric bubble.
//!
//! The default is a standards-compliant undecorated GTK toplevel. When someone
//! explicitly asks to keep it above other windows, a second surface uses the
//! narrow Wayland layer-shell protocol where the compositor supports it. This
//! needs no KWin scripting, global-input access, or extra Flatpak permission.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use relm4::adw::prelude::*;
use relm4::gtk;

use crate::companion::Companion;
use crate::components::jamkin_actor::JamkinActor;
use crate::lyrics::Lyrics;
use crate::settings::{
    JamkinQuality, MAX_DESKTOP_JAMKIN_OPACITY, MIN_DESKTOP_JAMKIN_OPACITY, Settings,
};

mod hover;
mod oled;
use oled::{InteractionState, OledCare};

const DISPLAY_LINE_MAX: usize = 180;
const DRAG_THRESHOLD: f64 = 4.0;
const FALLBACK_MARGIN_LIMIT: i32 = 8_192;

type Callback = Rc<dyn Fn()>;
type PositionCallback = Rc<dyn Fn(i32, i32)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    Movable,
    Above,
}

#[derive(Clone, Copy)]
pub struct JamkinModeConfig {
    companion: Companion,
    size: u16,
    quality: JamkinQuality,
    opacity: u8,
    reduced_motion: bool,
    stay_visible: bool,
    keep_above: bool,
    oled_care: bool,
    position: (i32, i32),
}

impl JamkinModeConfig {
    /// Take one coherent snapshot rather than threading a growing list of
    /// preferences through the app model's already busy constructor.
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            companion: settings.companion,
            size: settings.desktop_jamkin_size,
            quality: settings.jamkin_quality,
            opacity: settings.desktop_jamkin_opacity,
            reduced_motion: settings.jamkin_reduced_motion,
            stay_visible: settings.desktop_jamkin_stay_visible,
            keep_above: settings.desktop_jamkin_above,
            oled_care: settings.desktop_jamkin_oled_care,
            position: (
                settings.desktop_jamkin_right,
                settings.desktop_jamkin_bottom,
            ),
        }
    }
}

struct JamkinSurface {
    window: gtk::Window,
    actor: JamkinActor,
    popover: gtk::Popover,
    current_line: gtk::Label,
    next_line: gtk::Label,
    placement: Placement,
    position_changed: Option<PositionCallback>,
    interaction: Rc<InteractionState>,
}

impl JamkinSurface {
    fn new(
        config: JamkinModeConfig,
        placement: Placement,
        position_changed: Option<PositionCallback>,
        open_lyrics: Callback,
        disable: Callback,
    ) -> Self {
        let actor = JamkinActor::new(config.companion, i32::from(config.size), config.quality);
        actor.set_reduced_motion(config.reduced_motion);
        actor.widget().set_opacity(
            f64::from(
                config
                    .opacity
                    .clamp(MIN_DESKTOP_JAMKIN_OPACITY, MAX_DESKTOP_JAMKIN_OPACITY),
            ) / 100.0,
        );
        actor.widget().add_css_class("desktop-jamkin-sprite");

        let current_line = gtk::Label::builder()
            .label("Pick a song and I'll sing along.")
            .justify(gtk::Justification::Center)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(36)
            .css_classes(["jamkin-bubble-current"])
            .build();
        let next_line = gtk::Label::builder()
            .label("Hover here whenever you want the current line.")
            .justify(gtk::Justification::Center)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(38)
            .css_classes(["caption", "dim-label", "jamkin-bubble-next"])
            .build();
        let copy = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(15)
            .margin_end(15)
            .build();
        copy.append(&current_line);
        copy.append(&next_line);

        let popover = gtk::Popover::builder()
            .position(gtk::PositionType::Top)
            .autohide(false)
            .has_arrow(true)
            .can_focus(false)
            .focusable(false)
            .css_classes(["jamkin-lyrics-popover"])
            .child(&copy)
            .build();
        // A popup surface leaves no invisible rectangle above the sprite to
        // intercept desktop clicks while the lyric bubble is closed.
        popover.set_parent(actor.widget());

        let interaction = Rc::new(InteractionState::default());
        hover::install(actor.widget(), &popover, interaction.clone());

        let dragged = Rc::new(Cell::new(false));
        let click = gtk::GestureClick::new();
        click.set_button(gtk::gdk::BUTTON_PRIMARY);
        {
            let dragged = dragged.clone();
            click.connect_released(move |_, presses, _, _| {
                if presses == 1 && !dragged.replace(false) {
                    open_lyrics();
                }
            });
        }
        actor.widget().add_controller(click);

        let window = match placement {
            Placement::Movable => {
                // WindowHandle supplies compositor-native dragging for the
                // ordinary xdg-toplevel, including under Wayland.
                let handle = gtk::WindowHandle::builder()
                    .child(actor.widget())
                    .css_classes(["desktop-jamkin-handle"])
                    .build();
                jamkin_window(config.size, &handle)
            }
            Placement::Above => jamkin_window(config.size, actor.widget()),
        };

        if placement == Placement::Above {
            // This changes only this alternate, initially hidden window. The
            // default Jamkin remains an ordinary movable GTK toplevel.
            window.init_layer_shell();
            window.set_namespace(Some("jamelade-jamkin"));
            window.set_layer(Layer::Overlay);
            window.set_keyboard_mode(KeyboardMode::None);
            window.set_exclusive_zone(0);
            window.set_anchor(Edge::Right, true);
            window.set_anchor(Edge::Bottom, true);
            window.set_margin(Edge::Right, config.position.0.max(0));
            window.set_margin(Edge::Bottom, config.position.1.max(0));

            let report = position_changed
                .as_ref()
                .expect("the Keep Above surface has a position callback")
                .clone();
            {
                let report = report.clone();
                window.connect_map(move |window| {
                    let before = (window.margin(Edge::Right), window.margin(Edge::Bottom));
                    let actual = clamp_layer_margins(window, window.width());
                    if actual != before {
                        report(actual.0, actual.1);
                    }
                });
            }
            add_layer_drag(
                actor.widget(),
                &window,
                dragged,
                interaction.clone(),
                report,
            );
        }

        window.connect_close_request(move |_| {
            disable();
            gtk::glib::Propagation::Stop
        });

        Self {
            window,
            actor,
            popover,
            current_line,
            next_line,
            placement,
            position_changed,
            interaction,
        }
    }

    fn set_visible(&self, visible: bool) {
        if !visible {
            self.popover.popdown();
        }
        // Mapping this helper never asks for focus; it must not steal the
        // keyboard from the application somebody is actually using.
        self.window.set_visible(visible);
    }

    fn set_companion(&self, companion: Companion) {
        self.actor.set_companion(companion);
        self.window
            .set_title(Some(&format!("{} — Jamelade", companion.label())));
    }

    fn set_size(&self, size: u16) {
        self.actor.set_size(i32::from(size));
        self.window
            .set_default_size(i32::from(size), i32::from(size));
        if self.placement == Placement::Above {
            let before = (
                self.window.margin(Edge::Right),
                self.window.margin(Edge::Bottom),
            );
            let actual = clamp_layer_margins(&self.window, i32::from(size));
            if actual != before
                && let Some(report) = &self.position_changed
            {
                report(actual.0, actual.1);
            }
        }
    }

    fn set_opacity(&self, opacity: u8) {
        let opacity = opacity.clamp(MIN_DESKTOP_JAMKIN_OPACITY, MAX_DESKTOP_JAMKIN_OPACITY);
        self.actor.widget().set_opacity(f64::from(opacity) / 100.0);
    }

    fn set_reduced_motion(&self, reduced: bool) {
        self.actor.set_reduced_motion(reduced);
    }

    fn set_copy(&self, current: &str, next: &str) {
        self.current_line.set_label(current);
        self.next_line.set_label(next);
        self.next_line.set_visible(!next.is_empty());
    }
}

impl Drop for JamkinSurface {
    fn drop(&mut self) {
        // A manually parented popover must be detached before its anchor goes
        // away, or GTK reports a leaked child.
        self.popover.unparent();
    }
}

pub struct JamkinMode {
    movable: JamkinSurface,
    above: Option<JamkinSurface>,
    timed: RefCell<Vec<(u64, String)>>,
    current: Cell<Option<usize>>,
    enabled: Cell<bool>,
    main_window_visible: Cell<bool>,
    stay_visible: Cell<bool>,
    keep_above: Cell<bool>,
    oled_care: Cell<bool>,
    oled: Option<Rc<OledCare>>,
}

impl JamkinMode {
    pub fn keep_above_supported() -> bool {
        gtk::is_initialized_main_thread() && gtk4_layer_shell::is_supported()
    }

    pub fn new(
        config: JamkinModeConfig,
        position_changed: impl Fn(i32, i32) + 'static,
        open_lyrics: impl Fn() + 'static,
        disable: impl Fn() + 'static,
    ) -> Self {
        let position_changed: PositionCallback = Rc::new(position_changed);
        let open_lyrics: Callback = Rc::new(open_lyrics);
        let disable: Callback = Rc::new(disable);
        let movable = JamkinSurface::new(
            config,
            Placement::Movable,
            None,
            open_lyrics.clone(),
            disable.clone(),
        );
        let above = Self::keep_above_supported().then(|| {
            JamkinSurface::new(
                config,
                Placement::Above,
                Some(position_changed),
                open_lyrics,
                disable,
            )
        });
        let keep_above = (config.keep_above || config.oled_care) && above.is_some();
        let oled = above.as_ref().map(OledCare::new);
        if let Some(oled) = &oled {
            oled.set_reduced_motion(config.reduced_motion);
        }
        let oled_care = config.oled_care && oled.is_some();
        Self {
            movable,
            above,
            timed: RefCell::new(Vec::new()),
            current: Cell::new(None),
            enabled: Cell::new(false),
            main_window_visible: Cell::new(true),
            stay_visible: Cell::new(config.stay_visible),
            keep_above: Cell::new(keep_above),
            oled_care: Cell::new(oled_care),
            oled,
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
        self.sync_visibility();
    }

    pub fn set_main_window_visible(&self, visible: bool) {
        if self.main_window_visible.replace(visible) != visible {
            self.sync_visibility();
        }
    }

    pub fn set_stay_visible(&self, stay_visible: bool) {
        if self.stay_visible.replace(stay_visible) != stay_visible {
            self.sync_visibility();
        }
    }

    /// Switch between the ordinary movable window and the compositor overlay.
    /// Returns the state actually applied so callers never persist an
    /// unsupported request as though it were active.
    pub fn set_keep_above(&self, above: bool) -> bool {
        let actual = above && self.above.is_some();
        if !actual {
            self.oled_care.set(false);
        }
        if self.keep_above.replace(actual) != actual {
            self.sync_visibility();
        }
        actual
    }

    /// Enable OLED movement only on the compositor-controlled overlay. Turning
    /// it on also selects that surface; ordinary Wayland toplevels cannot be
    /// moved programmatically without misleading the user about support.
    pub fn set_oled_care(&self, enabled: bool) -> bool {
        let actual = enabled && self.oled.is_some();
        if actual {
            self.keep_above.set(true);
        }
        self.oled_care.set(actual);
        self.sync_visibility();
        actual
    }

    pub fn oled_care_enabled(&self) -> bool {
        self.oled_care.get()
    }

    pub fn set_companion(&self, companion: Companion) {
        for surface in self.surfaces() {
            surface.set_companion(companion);
        }
    }

    pub fn set_quality(&self, quality: JamkinQuality) {
        for surface in self.surfaces() {
            surface.actor.set_quality(quality);
        }
    }

    pub fn set_size(&self, size: u16) {
        for surface in self.surfaces() {
            surface.set_size(size);
        }
    }

    pub fn set_opacity(&self, opacity: u8) {
        for surface in self.surfaces() {
            surface.set_opacity(opacity);
        }
    }

    pub fn set_reduced_motion(&self, reduced: bool) {
        for surface in self.surfaces() {
            surface.set_reduced_motion(reduced);
        }
        if let Some(oled) = &self.oled {
            oled.set_reduced_motion(reduced);
        }
    }

    pub fn set_playing(&self, playing: bool) {
        for surface in self.surfaces() {
            surface.actor.set_playing(playing);
        }
    }

    pub fn disabled(&self) {
        self.clear_timeline();
        self.set_copy("Lyrics are off", "Enable a source in Privacy Preferences.");
    }

    pub fn waiting(&self) {
        self.clear_timeline();
        self.set_copy(
            "Pick a song and I'll sing along.",
            "Click me to open the full lyrics view.",
        );
    }

    pub fn loading(&self, title: &str) {
        self.clear_timeline();
        self.set_copy(
            &format!("Finding lyrics for “{}”…", display_line(title)),
            "Playback carries on while I look.",
        );
    }

    pub fn show(&self, lyrics: &Lyrics) {
        self.clear_timeline();
        if lyrics.instrumental {
            self.set_copy("No words this time", "Just listening along with you.");
            return;
        }
        if lyrics.lines.is_empty() {
            self.set_copy(
                "I couldn't find this one",
                "Click me to retry in the full lyrics view.",
            );
            return;
        }
        if !lyrics.synced {
            let current = display_line(&lyrics.lines[0].text);
            let next = lyrics
                .lines
                .get(1)
                .map(|line| display_line(&line.text))
                .unwrap_or_default();
            self.set_copy(&current, &next);
            return;
        }

        *self.timed.borrow_mut() = lyrics
            .lines
            .iter()
            .filter_map(|line| line.at_ms.map(|at| (at, line.text.clone())))
            .collect();
        let first = self
            .timed
            .borrow()
            .first()
            .map(|(_, line)| display_line(line))
            .unwrap_or_else(|| "Waiting for the first line…".into());
        self.set_copy("♪", &first);
    }

    pub fn fail(&self) {
        self.clear_timeline();
        self.set_copy(
            "I couldn't reach the lyrics source",
            "Playback is unaffected.",
        );
    }

    pub fn sync_position(&self, position_ms: u64) {
        let timed = self.timed.borrow();
        if timed.is_empty() {
            return;
        }
        let next = active_index(&timed, position_ms);
        if self.current.replace(next) == next {
            return;
        }

        match next {
            Some(index) => {
                let current = display_line(&timed[index].1);
                let following = timed
                    .get(index + 1)
                    .map(|(_, line)| display_line(line))
                    .unwrap_or_default();
                self.set_copy(&current, &following);
            }
            None => {
                self.set_copy("♪", &display_line(&timed[0].1));
            }
        }
    }

    fn surfaces(&self) -> impl Iterator<Item = &JamkinSurface> {
        std::iter::once(&self.movable).chain(self.above.iter())
    }

    fn sync_visibility(&self) {
        let enabled = should_show_desktop_jamkin(
            self.enabled.get(),
            self.main_window_visible.get(),
            self.stay_visible.get(),
        );
        let above = self.keep_above.get() && self.above.is_some();
        self.movable.set_visible(enabled && !above);
        if let Some(surface) = &self.above {
            surface.set_visible(enabled && above);
        }
        if let Some(oled) = &self.oled {
            oled.set_active(enabled && above && self.oled_care.get());
        }
    }

    fn clear_timeline(&self) {
        self.timed.borrow_mut().clear();
        self.current.set(None);
    }

    fn set_copy(&self, current: &str, next: &str) {
        for surface in self.surfaces() {
            surface.set_copy(current, next);
        }
    }
}

fn jamkin_window(size: u16, child: &impl IsA<gtk::Widget>) -> gtk::Window {
    gtk::Window::builder()
        .application(&relm4::main_application())
        .title("Jamelade Jamkin")
        // Fix the toplevel as well as the picture, otherwise a mapped window
        // can retain its old allocation and scale a newly smaller actor up.
        .default_width(i32::from(size))
        .default_height(i32::from(size))
        .decorated(false)
        .resizable(false)
        .hide_on_close(true)
        .focusable(false)
        .css_classes(["desktop-jamkin-window"])
        .child(child)
        .build()
}

fn add_layer_drag(
    widget: &impl IsA<gtk::Widget>,
    window: &gtk::Window,
    dragged: Rc<Cell<bool>>,
    interaction: Rc<InteractionState>,
    position_changed: PositionCallback,
) {
    let start = Rc::new(Cell::new((0, 0)));
    let moved = Rc::new(Cell::new(false));
    let drag = gtk::GestureDrag::new();
    drag.set_button(gtk::gdk::BUTTON_PRIMARY);
    {
        let window = window.clone();
        let start = start.clone();
        let dragged = dragged.clone();
        let moved = moved.clone();
        let interaction = interaction.clone();
        drag.connect_drag_begin(move |_, _, _| {
            interaction.set_drag(true);
            dragged.set(false);
            moved.set(false);
            start.set((window.margin(Edge::Right), window.margin(Edge::Bottom)));
        });
    }
    {
        let window = window.clone();
        let moved = moved.clone();
        drag.connect_drag_update(move |_, offset_x, offset_y| {
            if offset_x.abs().max(offset_y.abs()) < DRAG_THRESHOLD {
                return;
            }
            dragged.set(true);
            moved.set(true);
            let (start_right, start_bottom) = start.get();
            let (max_right, max_bottom) = layer_margin_limits(&window, window.width());
            window.set_margin(
                Edge::Right,
                dragged_margin(start_right, offset_x, max_right),
            );
            window.set_margin(
                Edge::Bottom,
                dragged_margin(start_bottom, offset_y, max_bottom),
            );
        });
    }
    {
        let window = window.clone();
        let interaction = interaction.clone();
        drag.connect_drag_end(move |_, _, _| {
            interaction.set_drag(false);
            if moved.replace(false) {
                let actual = clamp_layer_margins(&window, window.width());
                position_changed(actual.0, actual.1);
            }
        });
    }
    widget.add_controller(drag);
}

fn layer_margin_limits(window: &gtk::Window, size: i32) -> (i32, i32) {
    let Some(surface) = window.surface() else {
        return (FALLBACK_MARGIN_LIMIT, FALLBACK_MARGIN_LIMIT);
    };
    let Some(monitor) = surface.display().monitor_at_surface(&surface) else {
        return (FALLBACK_MARGIN_LIMIT, FALLBACK_MARGIN_LIMIT);
    };
    let geometry = monitor.geometry();
    (
        geometry.width().saturating_sub(size).max(0),
        geometry.height().saturating_sub(size).max(0),
    )
}

fn clamp_layer_margins(window: &gtk::Window, size: i32) -> (i32, i32) {
    let (max_right, max_bottom) = layer_margin_limits(window, size);
    let right = window.margin(Edge::Right).clamp(0, max_right);
    let bottom = window.margin(Edge::Bottom).clamp(0, max_bottom);
    window.set_margin(Edge::Right, right);
    window.set_margin(Edge::Bottom, bottom);
    (right, bottom)
}

fn dragged_margin(start: i32, offset: f64, maximum: i32) -> i32 {
    if !offset.is_finite() {
        return start.clamp(0, maximum);
    }
    (f64::from(start) - offset)
        .round()
        .clamp(0.0, f64::from(maximum)) as i32
}

fn active_index(timed: &[(u64, String)], position_ms: u64) -> Option<usize> {
    timed
        .partition_point(|(at, _)| *at <= position_ms)
        .checked_sub(1)
}

fn display_line(value: &str) -> String {
    let mut chars = value.trim().chars();
    let line: String = chars.by_ref().take(DISPLAY_LINE_MAX).collect();
    if chars.next().is_some() {
        format!("{line}…")
    } else {
        line
    }
}

fn should_show_desktop_jamkin(enabled: bool, main_visible: bool, stay_visible: bool) -> bool {
    enabled && (main_visible || stay_visible)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline() -> Vec<(u64, String)> {
        vec![
            (1_000, "one".into()),
            (2_000, "two".into()),
            (3_000, "three".into()),
        ]
    }

    #[test]
    fn lyric_boundary_belongs_to_the_line_that_starts_there() {
        let timed = timeline();
        assert_eq!(active_index(&timed, 999), None);
        assert_eq!(active_index(&timed, 1_000), Some(0));
        assert_eq!(active_index(&timed, 2_999), Some(1));
        assert_eq!(active_index(&timed, 99_000), Some(2));
    }

    #[test]
    fn desktop_copy_is_bounded_even_for_a_malformed_provider_line() {
        let huge = "x".repeat(DISPLAY_LINE_MAX + 40);
        let shown = display_line(&huge);
        assert_eq!(shown.chars().count(), DISPLAY_LINE_MAX + 1);
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn desktop_copy_preserves_cjk_and_rtl_text() {
        assert_eq!(display_line("  夜に駆ける  "), "夜に駆ける");
        assert_eq!(display_line("  مرحبا بالعالم  "), "مرحبا بالعالم");
    }

    #[test]
    fn stay_visible_only_matters_while_the_main_window_is_hidden() {
        assert!(should_show_desktop_jamkin(true, true, false));
        assert!(should_show_desktop_jamkin(true, false, true));
        assert!(!should_show_desktop_jamkin(true, false, false));
        assert!(!should_show_desktop_jamkin(false, true, true));
    }

    #[test]
    fn pinned_drag_moves_and_clamps_from_the_anchored_edges() {
        assert_eq!(dragged_margin(24, 10.0, 500), 14);
        assert_eq!(dragged_margin(24, -10.0, 500), 34);
        assert_eq!(dragged_margin(24, 100.0, 500), 0);
        assert_eq!(dragged_margin(24, -1_000.0, 500), 500);
        assert_eq!(dragged_margin(24, f64::NAN, 500), 24);
    }
}
