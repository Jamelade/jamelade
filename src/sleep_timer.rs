// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Process-local sleep timer state. It never changes the queue or persists a
//! listening schedule; expiry simply asks the existing player to pause.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Minutes(u16),
    EndOfTrack,
    Off,
}

impl Choice {
    pub const MENU: [Self; 5] = [
        Self::Minutes(15),
        Self::Minutes(30),
        Self::Minutes(60),
        Self::EndOfTrack,
        Self::Off,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Minutes(15) => "15 minutes",
            Self::Minutes(30) => "30 minutes",
            Self::Minutes(60) => "1 hour",
            Self::Minutes(_) => "Custom duration",
            Self::EndOfTrack => "End of this song",
            Self::Off => "Turn off",
        }
    }
}

#[derive(Debug, Default)]
pub struct Timer {
    generation: u64,
    end_track_id: Option<String>,
    active: bool,
}

impl Timer {
    pub fn set(
        &mut self,
        choice: Choice,
        current_track_id: Option<&str>,
    ) -> (u64, Option<std::time::Duration>) {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.active = !matches!(choice, Choice::Off);
        self.end_track_id = match choice {
            Choice::EndOfTrack => current_track_id.map(str::to_owned),
            _ => None,
        };
        let delay = match choice {
            Choice::Minutes(minutes) => {
                Some(std::time::Duration::from_secs(u64::from(minutes) * 60))
            }
            Choice::EndOfTrack | Choice::Off => None,
        };
        (self.generation, delay)
    }

    pub fn expires(&mut self, generation: u64) -> bool {
        if self.active && self.generation == generation {
            self.active = false;
            self.end_track_id = None;
            true
        } else {
            false
        }
    }

    pub fn track_changed(&mut self, new_track_id: Option<&str>, finished_naturally: bool) -> bool {
        let due = self.active
            && self.end_track_id.is_some()
            && self.end_track_id.as_deref() != new_track_id;
        if due {
            self.active = false;
            self.end_track_id = None;
        }
        due && finished_naturally
    }

    pub const fn active(&self) -> bool {
        self.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_or_cancelling_a_timer_invalidates_old_expiry() {
        let mut timer = Timer::default();
        let (first, _) = timer.set(Choice::Minutes(15), Some("1"));
        let (second, _) = timer.set(Choice::Minutes(30), Some("1"));
        assert!(!timer.expires(first));
        assert!(timer.expires(second));
        let (third, _) = timer.set(Choice::Minutes(15), Some("1"));
        timer.set(Choice::Off, Some("1"));
        assert!(!timer.expires(third));
    }

    #[test]
    fn end_of_track_fires_only_after_that_track_changes() {
        let mut timer = Timer::default();
        timer.set(Choice::EndOfTrack, Some("1000000001"));
        assert!(!timer.track_changed(Some("1000000001"), true));
        assert!(timer.track_changed(Some("1000000002"), true));
        assert!(!timer.track_changed(Some("1000000003"), true));
    }

    #[test]
    fn manually_skipping_cancels_end_of_track_without_pausing_the_next_song() {
        let mut timer = Timer::default();
        timer.set(Choice::EndOfTrack, Some("1000000001"));
        assert!(!timer.track_changed(Some("1000000002"), false));
        assert!(!timer.active());
    }
}
