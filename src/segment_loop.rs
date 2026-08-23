// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Process-local A-B looping.
//!
//! The marks are deliberately not persisted: they describe one occurrence of
//! one track in the current queue, not a preference that should reappear next
//! launch.

use std::time::{Duration, Instant};

pub(crate) const MIN_SPAN_MS: u64 = 1_000;
const SEEK_RETRY_AFTER: Duration = Duration::from_secs(1);
const SEEK_REARM_MARGIN_MS: u64 = 250;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopMarks {
    #[default]
    Off,
    Start(u64),
    Active {
        start_ms: u64,
        end_ms: u64,
    },
}

impl LoopMarks {
    pub(crate) fn is_active(self) -> bool {
        matches!(self, Self::Active { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkOutcome {
    StartSet(u64),
    LoopSet { start_ms: u64, end_ms: u64 },
    Cleared,
    Unavailable,
    StartTooLate,
    EndTooEarly,
}

#[derive(Debug, Default)]
pub(crate) struct SegmentLoop {
    marks: LoopMarks,
    last_seek: Option<Instant>,
}

impl SegmentLoop {
    pub(crate) fn marks(&self) -> LoopMarks {
        self.marks
    }

    pub(crate) fn clear(&mut self) {
        self.marks = LoopMarks::Off;
        self.last_seek = None;
    }

    /// Off -> mark A -> mark B -> off.
    pub(crate) fn cycle(&mut self, position_ms: u64, duration_ms: u64) -> MarkOutcome {
        if duration_ms == 0 {
            return MarkOutcome::Unavailable;
        }
        let position_ms = position_ms.min(duration_ms);
        match self.marks {
            LoopMarks::Off => {
                if duration_ms.saturating_sub(position_ms) < MIN_SPAN_MS {
                    return MarkOutcome::StartTooLate;
                }
                self.marks = LoopMarks::Start(position_ms);
                self.last_seek = None;
                MarkOutcome::StartSet(position_ms)
            }
            LoopMarks::Start(start_ms) => {
                if position_ms.saturating_sub(start_ms) < MIN_SPAN_MS {
                    return MarkOutcome::EndTooEarly;
                }
                self.marks = LoopMarks::Active {
                    start_ms,
                    end_ms: position_ms,
                };
                self.last_seek = None;
                MarkOutcome::LoopSet {
                    start_ms,
                    end_ms: position_ms,
                }
            }
            LoopMarks::Active { .. } => {
                self.clear();
                MarkOutcome::Cleared
            }
        }
    }

    /// Return A once playback reaches B.
    ///
    /// A seek stays latched until its echo places playback safely below B. If
    /// MusicKit drops the command, one retry per second avoids both a stuck loop
    /// and a seek storm.
    pub(crate) fn seek_target(&mut self, position_ms: u64, now: Instant) -> Option<u64> {
        let LoopMarks::Active { start_ms, end_ms } = self.marks else {
            return None;
        };

        if position_ms < end_ms.saturating_sub(SEEK_REARM_MARGIN_MS) {
            self.last_seek = None;
            return None;
        }
        if position_ms < end_ms {
            return None;
        }
        if self
            .last_seek
            .is_some_and(|sent| now.saturating_duration_since(sent) < SEEK_RETRY_AFTER)
        {
            return None;
        }
        self.last_seek = Some(now);
        Some(start_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_clicks_set_a_set_b_and_clear() {
        let mut loop_ = SegmentLoop::default();
        assert_eq!(loop_.cycle(12_000, 60_000), MarkOutcome::StartSet(12_000));
        assert_eq!(
            loop_.cycle(18_500, 60_000),
            MarkOutcome::LoopSet {
                start_ms: 12_000,
                end_ms: 18_500
            }
        );
        assert_eq!(loop_.cycle(15_000, 60_000), MarkOutcome::Cleared);
        assert_eq!(loop_.marks(), LoopMarks::Off);
    }

    #[test]
    fn points_must_leave_a_useful_span() {
        let mut loop_ = SegmentLoop::default();
        assert_eq!(loop_.cycle(59_500, 60_000), MarkOutcome::StartTooLate);
        assert_eq!(loop_.marks(), LoopMarks::Off);

        assert!(matches!(
            loop_.cycle(10_000, 60_000),
            MarkOutcome::StartSet(_)
        ));
        assert_eq!(loop_.cycle(10_999, 60_000), MarkOutcome::EndTooEarly);
        assert_eq!(loop_.marks(), LoopMarks::Start(10_000));
    }

    #[test]
    fn reaching_b_seeks_once_until_the_echo_rearms_it() {
        let mut loop_ = SegmentLoop::default();
        loop_.cycle(10_000, 60_000);
        loop_.cycle(20_000, 60_000);
        let now = Instant::now();

        assert_eq!(loop_.seek_target(19_999, now), None);
        assert_eq!(loop_.seek_target(20_000, now), Some(10_000));
        assert_eq!(loop_.seek_target(20_100, now), None);
        assert_eq!(loop_.seek_target(10_050, now), None);
        assert_eq!(loop_.seek_target(20_000, now), Some(10_000));
    }

    #[test]
    fn a_dropped_seek_can_retry_without_spamming() {
        let mut loop_ = SegmentLoop::default();
        loop_.cycle(10_000, 60_000);
        loop_.cycle(20_000, 60_000);
        let now = Instant::now();

        assert_eq!(loop_.seek_target(20_000, now), Some(10_000));
        assert_eq!(
            loop_.seek_target(20_500, now + Duration::from_millis(999)),
            None
        );
        assert_eq!(
            loop_.seek_target(21_000, now + Duration::from_secs(1)),
            Some(10_000)
        );
    }
}
