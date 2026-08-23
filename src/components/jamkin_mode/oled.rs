// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Edge Walk movement for the compositor-controlled Jamkin surface.
//!
//! This module is intentionally local-only: it reads the current monitor's
//! dimensions from GTK, moves layer-shell margins, and forgets every automatic
//! position. It requests no input, screenshot, identity or network access.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk4_layer_shell::{Edge, LayerShell};
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

use super::{JamkinSurface, clamp_layer_margins, layer_margin_limits};
const FIRST_MOVE: Duration = Duration::from_secs(12);
const MOVE_INTERVAL: Duration = Duration::from_secs(120);
const BUSY_RETRY: Duration = Duration::from_secs(10);
const EDGE_INSET: i32 = 24;
const TELEPORT_EVERY: u32 = 6;

#[derive(Default)]
pub(super) struct InteractionState {
    hover: Cell<bool>,
    drag: Cell<bool>,
}

impl InteractionState {
    pub(super) fn set_hover(&self, hover: bool) {
        self.hover.set(hover);
    }

    pub(super) fn set_drag(&self, drag: bool) {
        self.drag.set(drag);
    }

    fn busy(&self) -> bool {
        self.hover.get() || self.drag.get()
    }
}

/// Owns one movement timer for the layer-shell surface. It pauses whenever the
/// pointer or a drag is interacting with the Jamkin.
pub(super) struct OledCare {
    window: gtk::Window,
    interaction: Rc<InteractionState>,
    active: Cell<bool>,
    reduced_motion: Cell<bool>,
    step: Cell<u32>,
    timer: RefCell<Option<gtk::glib::SourceId>>,
    motion: RefCell<Option<adw::TimedAnimation>>,
}

impl OledCare {
    pub(super) fn new(surface: &JamkinSurface) -> Rc<Self> {
        Rc::new(Self {
            window: surface.window.clone(),
            interaction: surface.interaction.clone(),
            active: Cell::new(false),
            reduced_motion: Cell::new(false),
            step: Cell::new(0),
            timer: RefCell::new(None),
            motion: RefCell::new(None),
        })
    }

    fn animations_enabled(&self) -> bool {
        !self.reduced_motion.get()
            && gtk::Settings::default()
                .map(|settings| settings.is_gtk_enable_animations())
                .unwrap_or(true)
    }

    pub(super) fn set_reduced_motion(&self, reduced: bool) {
        if self.reduced_motion.replace(reduced) == reduced {
            return;
        }
        if reduced && let Some(animation) = self.motion.borrow_mut().take() {
            animation.pause();
        }
    }

    pub(super) fn set_active(self: &Rc<Self>, active: bool) {
        if self.active.replace(active) == active {
            return;
        }
        self.cancel_timer();
        if let Some(animation) = self.motion.borrow_mut().take() {
            animation.pause();
        }
        if active {
            self.schedule(FIRST_MOVE);
        }
    }

    fn cancel_timer(&self) {
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
    }

    fn schedule(self: &Rc<Self>, delay: Duration) {
        self.cancel_timer();
        let weak = Rc::downgrade(self);
        let timer = gtk::glib::timeout_add_local_once(delay, move || {
            if let Some(this) = weak.upgrade() {
                this.timer.borrow_mut().take();
                this.tick();
            }
        });
        *self.timer.borrow_mut() = Some(timer);
    }

    fn tick(self: &Rc<Self>) {
        if !self.active.get() {
            return;
        }
        if self.interaction.busy() {
            self.schedule(BUSY_RETRY);
            return;
        }

        let size = self.window.width().max(1);
        let limits = layer_margin_limits(&self.window, size);
        let current = clamp_layer_margins(&self.window, size);
        let step = self.step.get();
        self.step.set(step.wrapping_add(1));

        if step.is_multiple_of(TELEPORT_EVERY) {
            self.teleport(next_corner(current, limits));
        } else {
            let distance = (size * 3 / 4).max(64);
            self.walk(current, advance_perimeter(current, limits, distance));
        }
        self.schedule(MOVE_INTERVAL);
    }

    fn walk(&self, from: (i32, i32), to: (i32, i32)) {
        if let Some(animation) = self.motion.borrow_mut().take() {
            animation.pause();
        }
        if !self.animations_enabled() {
            set_layer_margins(&self.window, to);
            return;
        }
        let window = self.window.clone();
        let animation = adw::TimedAnimation::new(
            &self.window,
            0.0,
            1.0,
            2_400,
            adw::CallbackAnimationTarget::new(move |progress| {
                let right = interpolate_margin(from.0, to.0, progress);
                let bottom = interpolate_margin(from.1, to.1, progress);
                set_layer_margins(&window, (right, bottom));
            }),
        );
        animation.set_easing(adw::Easing::EaseInOutCubic);
        animation.play();
        *self.motion.borrow_mut() = Some(animation);
    }

    fn teleport(&self, to: (i32, i32)) {
        if let Some(animation) = self.motion.borrow_mut().take() {
            animation.pause();
        }
        set_layer_margins(&self.window, to);
    }
}

impl Drop for OledCare {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
    }
}

fn set_layer_margins(window: &gtk::Window, position: (i32, i32)) {
    window.set_margin(Edge::Right, position.0);
    window.set_margin(Edge::Bottom, position.1);
}

fn interpolate_margin(from: i32, to: i32, progress: f64) -> i32 {
    (f64::from(from) + f64::from(to - from) * progress.clamp(0.0, 1.0)).round() as i32
}

fn edge_bounds(limits: (i32, i32)) -> (i32, i32, i32, i32) {
    let right_inset = EDGE_INSET.min(limits.0 / 2).max(0);
    let bottom_inset = EDGE_INSET.min(limits.1 / 2).max(0);
    (
        right_inset,
        limits.0.saturating_sub(right_inset),
        bottom_inset,
        limits.1.saturating_sub(bottom_inset),
    )
}

/// Move clockwise around the screen perimeter in right/bottom-margin space.
fn advance_perimeter(point: (i32, i32), limits: (i32, i32), distance: i32) -> (i32, i32) {
    let (low_right, high_right, low_bottom, high_bottom) = edge_bounds(limits);
    let width = high_right - low_right;
    let height = high_bottom - low_bottom;
    let perimeter = 2 * (width + height);
    if perimeter <= 0 {
        return (low_right, low_bottom);
    }

    let right = point.0.clamp(low_right, high_right);
    let bottom = point.1.clamp(low_bottom, high_bottom);
    let distances = [
        bottom - low_bottom,
        high_right - right,
        high_bottom - bottom,
        right - low_right,
    ];
    let nearest = distances
        .iter()
        .enumerate()
        .min_by_key(|(_, value)| **value)
        .map(|(index, _)| index)
        .unwrap_or(0);
    let along = match nearest {
        0 => right - low_right,
        1 => width + bottom - low_bottom,
        2 => width + height + high_right - right,
        _ => 2 * width + height + high_bottom - bottom,
    };
    let along = (along + distance.max(1)).rem_euclid(perimeter);
    if along <= width {
        (low_right + along, low_bottom)
    } else if along <= width + height {
        (high_right, low_bottom + along - width)
    } else if along <= 2 * width + height {
        (high_right - (along - width - height), high_bottom)
    } else {
        (low_right, high_bottom - (along - 2 * width - height))
    }
}

fn next_corner(point: (i32, i32), limits: (i32, i32)) -> (i32, i32) {
    let (low_right, high_right, low_bottom, high_bottom) = edge_bounds(limits);
    let corners = [
        (low_right, low_bottom),
        (high_right, low_bottom),
        (high_right, high_bottom),
        (low_right, high_bottom),
    ];
    let nearest = corners
        .iter()
        .enumerate()
        .min_by_key(|(_, corner)| point.0.abs_diff(corner.0) + point.1.abs_diff(corner.1))
        .map(|(index, _)| index)
        .unwrap_or(0);
    corners[(nearest + 1) % corners.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_stays_on_the_inset_perimeter_and_turns_corners() {
        let limits = (900, 500);
        assert_eq!(advance_perimeter((24, 24), limits, 100), (124, 24));
        assert_eq!(advance_perimeter((850, 24), limits, 100), (876, 98));
        assert_eq!(advance_perimeter((876, 476), limits, 100), (776, 476));
    }

    #[test]
    fn teleport_chooses_a_different_adjacent_corner() {
        let limits = (900, 500);
        assert_eq!(next_corner((24, 24), limits), (876, 24));
        assert_eq!(next_corner((876, 24), limits), (876, 476));
    }

    #[test]
    fn a_tiny_screen_collapses_to_one_safe_point() {
        assert_eq!(advance_perimeter((999, 999), (0, 0), 100), (0, 0));
        assert_eq!(next_corner((0, 0), (0, 0)), (0, 0));
    }
}
