// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use relm4::{ComponentSender, gtk};

use super::{AppModel, AppMsg};
use crate::music::types::format_duration;
use crate::player::protocol::Command;
use crate::segment_loop::MarkOutcome;

impl AppModel {
    pub(super) fn cycle_segment_loop(&mut self, sender: &ComponentSender<Self>) {
        let outcome = self.segment_loop.cycle(
            self.player.interpolated_position_ms(),
            self.player.duration_ms,
        );
        match outcome {
            MarkOutcome::StartSet(_) => self.toast("Loop start set — choose point B"),
            MarkOutcome::LoopSet { start_ms, end_ms } => self.toast(&format!(
                "Looping {}–{}",
                format_duration(start_ms),
                format_duration(end_ms)
            )),
            MarkOutcome::Cleared => self.toast("Section loop off"),
            MarkOutcome::Unavailable => self.toast("This track cannot be looped yet"),
            MarkOutcome::StartTooLate => {
                self.toast("Choose point A at least one second before the end")
            }
            MarkOutcome::EndTooEarly => self.toast("Point B must be at least one second after A"),
        }
        self.sync_segment_loop_timer(sender);
        self.push_snapshot();
    }

    pub(super) fn clear_segment_loop(&mut self) {
        self.segment_loop.clear();
        if let Some(id) = self.segment_loop_tick.take() {
            id.remove();
        }
    }

    /// Run a native timer only while an A-B loop is actually playing. The
    /// hidden Chromium player freezes its own timers, so it cannot enforce it.
    pub(super) fn sync_segment_loop_timer(&mut self, sender: &ComponentSender<Self>) {
        const LOOP_TICK_MS: u64 = 50;
        let want = self.segment_loop.marks().is_active() && self.player.state.is_playing();
        match (want, self.segment_loop_tick.is_some()) {
            (true, false) => {
                let sender = sender.clone();
                self.segment_loop_tick = Some(gtk::glib::timeout_add_local(
                    std::time::Duration::from_millis(LOOP_TICK_MS),
                    move || {
                        sender.input(AppMsg::SegmentLoopTick);
                        gtk::glib::ControlFlow::Continue
                    },
                ));
            }
            (false, true) => {
                if let Some(id) = self.segment_loop_tick.take() {
                    id.remove();
                }
            }
            _ => {}
        }
    }

    pub(super) fn enforce_segment_loop(&mut self) {
        let Some(target_ms) = self.segment_loop.seek_target(
            self.player.interpolated_position_ms(),
            std::time::Instant::now(),
        ) else {
            return;
        };
        self.send(Command::Seek {
            position_ms: target_ms,
        });
        self.mpris.seeked(target_ms);
    }
}
