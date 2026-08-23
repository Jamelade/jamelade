// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! Lyrics as native labels, with optional LRC timestamps highlighted against
//! the mirrored playback clock.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

use crate::companion::Companion;
use crate::components::jamkin_actor::JamkinActor;
use crate::lyrics::Lyrics;
use crate::settings::JamkinQuality;

const LINE_FADE_MS: u32 = 420;
const SCROLL_GLIDE_MS: u32 = 560;
const EARLIER_OPACITY: f64 = 0.34;
const PREVIOUS_OPACITY: f64 = 0.46;
const NEXT_OPACITY: f64 = 0.68;
const FOLLOWING_OPACITY: f64 = 0.57;
const DISTANT_OPACITY: f64 = 0.46;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineMoment {
    Earlier,
    Previous,
    Current,
    Next,
    Following,
    Distant,
}

impl LineMoment {
    const ALL_CLASSES: [&'static str; 6] = [
        "lyrics-earlier",
        "lyrics-previous",
        "lyrics-current",
        "lyrics-next",
        "lyrics-following",
        "lyrics-distant",
    ];

    fn at(index: usize, current: Option<usize>) -> Self {
        let Some(current) = current else {
            return Self::Distant;
        };
        match index.cmp(&current) {
            std::cmp::Ordering::Equal => Self::Current,
            std::cmp::Ordering::Less if index + 1 == current => Self::Previous,
            std::cmp::Ordering::Less => Self::Earlier,
            std::cmp::Ordering::Greater if index == current.saturating_add(1) => Self::Next,
            std::cmp::Ordering::Greater if index == current.saturating_add(2) => Self::Following,
            std::cmp::Ordering::Greater => Self::Distant,
        }
    }

    const fn opacity(self) -> f64 {
        match self {
            Self::Earlier => EARLIER_OPACITY,
            Self::Previous => PREVIOUS_OPACITY,
            Self::Current => 1.0,
            Self::Next => NEXT_OPACITY,
            Self::Following => FOLLOWING_OPACITY,
            Self::Distant => DISTANT_OPACITY,
        }
    }

    const fn css_class(self) -> &'static str {
        match self {
            Self::Earlier => Self::ALL_CLASSES[0],
            Self::Previous => Self::ALL_CLASSES[1],
            Self::Current => Self::ALL_CLASSES[2],
            Self::Next => Self::ALL_CLASSES[3],
            Self::Following => Self::ALL_CLASSES[4],
            Self::Distant => Self::ALL_CLASSES[5],
        }
    }
}

/// A deliberately tiny semantic state model for future local-only animation.
/// It contains no credentials or raw player messages; CSS (and later a small
/// sprite controller) only needs to know what the companion is doing.
#[derive(Debug, Clone, Copy)]
enum JamkinState {
    Private,
    Idle,
    Fetching,
    Singing,
    Listening,
    Missing,
    Error,
}

impl JamkinState {
    const ALL_CLASSES: [&'static str; 7] = [
        "jamkin-private",
        "jamkin-idle",
        "jamkin-fetching",
        "jamkin-singing",
        "jamkin-listening",
        "jamkin-missing",
        "jamkin-error",
    ];

    const fn css_class(self) -> &'static str {
        match self {
            Self::Private => Self::ALL_CLASSES[0],
            Self::Idle => Self::ALL_CLASSES[1],
            Self::Fetching => Self::ALL_CLASSES[2],
            Self::Singing => Self::ALL_CLASSES[3],
            Self::Listening => Self::ALL_CLASSES[4],
            Self::Missing => Self::ALL_CLASSES[5],
            Self::Error => Self::ALL_CLASSES[6],
        }
    }
}

pub struct LyricsView {
    root: gtk::Box,
    pages: gtk::Stack,
    scroller: gtk::ScrolledWindow,
    lines_box: gtk::Box,
    lyrics_lines: gtk::Box,
    loading_title: gtk::Label,
    error: adw::StatusPage,
    no_lyrics: adw::StatusPage,
    source: gtk::Label,
    companion_stage: gtk::Box,
    companion_actor: JamkinActor,
    companion_name: gtk::Label,
    timed: RefCell<Vec<(u64, gtk::Label)>>,
    current: Cell<Option<usize>>,
    line_animations: RefCell<Vec<adw::TimedAnimation>>,
    scroll_animation: RefCell<Option<adw::TimedAnimation>>,
    seek: Rc<dyn Fn(u64)>,
}

impl LyricsView {
    pub fn new(
        companion: Companion,
        quality: JamkinQuality,
        reduced_motion: bool,
        enable: impl Fn() + 'static,
        seek: impl Fn(u64) + 'static,
    ) -> Self {
        let disabled = adw::StatusPage::builder()
            .icon_name("view-list-symbolic")
            .title("Lyrics are not connected yet")
            .description(
                "Sign in to use Apple Music lyrics, or review the separately optional third-party fallbacks in Privacy Preferences.",
            )
            .build();
        let enable_button = gtk::Button::builder()
            .label("Open Privacy Preferences")
            .halign(gtk::Align::Center)
            .css_classes(["suggested-action", "pill"])
            .build();
        enable_button.connect_clicked(move |_| enable());
        disabled.set_child(Some(&enable_button));

        let waiting = adw::StatusPage::builder()
            .icon_name("view-list-symbolic")
            .title("Nothing playing")
            .description("Start a song, then its lyrics will appear here.")
            .build();
        let no_lyrics = adw::StatusPage::builder()
            .icon_name("view-list-symbolic")
            .title("No lyrics found")
            .description("None of the enabled sources has a match for this recording.")
            .build();
        let instrumental = adw::StatusPage::builder()
            .icon_name("audio-x-generic-symbolic")
            .title("Instrumental")
            .description("This recording has no sung lyrics.")
            .build();
        let error = adw::StatusPage::builder()
            .icon_name("network-offline-symbolic")
            .title("Lyrics could not load")
            .description("Playback is unaffected.")
            .build();

        let loading_title = gtk::Label::builder()
            .label("Finding lyrics")
            .css_classes(["title-2"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        let loading = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .spacing(16)
            .build();
        loading.append(
            &adw::Spinner::builder()
                .width_request(42)
                .height_request(42)
                .build(),
        );
        loading.append(&loading_title);

        let source = gtk::Label::builder()
            .label("Connect Apple Music or enable an optional fallback")
            .xalign(0.0)
            .css_classes(["caption", "dim-label"])
            .build();
        let companion_actor = JamkinActor::new(companion, 142, quality);
        companion_actor.set_reduced_motion(reduced_motion);
        let companion_name = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["title-1", "jamkin-name"])
            .build();
        let companion_copy = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .spacing(5)
            .build();
        companion_copy.append(&companion_name);
        companion_copy.append(&source);

        // This stage lives outside the lyrics scroller. It is deliberately a
        // plain layout rather than a card: transparent companion art can read
        // as a real sprite, and the Jamkin stays beside the user while lines
        // glide independently below it.
        let companion_stage = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(18)
            .hexpand(true)
            .margin_top(8)
            .margin_bottom(0)
            .margin_start(28)
            .margin_end(28)
            .css_classes(["jamkin-stage"])
            .build();
        companion_stage.append(companion_actor.widget());
        companion_stage.append(&companion_copy);

        let lyrics_lines = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .hexpand(true)
            .build();
        let lines_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .hexpand(true)
            .margin_top(0)
            .margin_bottom(50)
            .margin_start(28)
            .margin_end(28)
            .build();
        lines_box.append(&lyrics_lines);
        let scroller = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(
                &adw::Clamp::builder()
                    // Wide enough to use a tiled window, still bounded so a
                    // lyric never becomes an unreadable single-line ribbon.
                    .maximum_size(1080)
                    .child(&lines_box)
                    .build(),
            )
            .build();
        scroller.add_css_class("plain-scroller");

        let pages = gtk::Stack::new();
        pages.set_transition_type(gtk::StackTransitionType::Crossfade);
        pages.set_vexpand(true);
        pages.add_named(&disabled, Some("disabled"));
        pages.add_named(&waiting, Some("waiting"));
        pages.add_named(&loading, Some("loading"));
        pages.add_named(&scroller, Some("lyrics"));
        pages.add_named(&no_lyrics, Some("none"));
        pages.add_named(&instrumental, Some("instrumental"));
        pages.add_named(&error, Some("error"));
        pages.set_visible_child_name("disabled");

        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .vexpand(true)
            .build();
        root.append(
            &adw::Clamp::builder()
                .maximum_size(1080)
                .child(&companion_stage)
                .build(),
        );
        root.append(&pages);

        let view = Self {
            root,
            pages,
            scroller,
            lines_box,
            lyrics_lines,
            loading_title,
            error,
            no_lyrics,
            source,
            companion_stage,
            companion_actor,
            companion_name,
            timed: RefCell::new(Vec::new()),
            current: Cell::new(None),
            line_animations: RefCell::new(Vec::new()),
            scroll_animation: RefCell::new(None),
            seek: Rc::new(seek),
        };
        view.set_companion(companion);
        view.set_jamkin_state(JamkinState::Private);
        view
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn disabled(&self) {
        self.clear_lines();
        self.neutral_source("Connect Apple Music or enable an optional fallback");
        self.set_jamkin_state(JamkinState::Private);
        self.pages.set_visible_child_name("disabled");
    }

    pub fn waiting(&self) {
        self.clear_lines();
        self.neutral_source("Start a song to sing along");
        self.set_jamkin_state(JamkinState::Idle);
        self.pages.set_visible_child_name("waiting");
    }

    pub fn loading(&self, title: &str) {
        self.clear_lines();
        let title: String = title.chars().take(120).collect();
        self.loading_title
            .set_label(&format!("Finding lyrics for {title}"));
        self.neutral_source("Checking your enabled sources one at a time");
        self.set_jamkin_state(JamkinState::Fetching);
        self.pages.set_visible_child_name("loading");
    }

    pub fn fail(&self, detail: &str) {
        let detail: String = detail.chars().take(240).collect();
        self.error.set_description(Some(&detail));
        self.neutral_source("Lyrics source unavailable");
        self.set_jamkin_state(JamkinState::Error);
        self.pages.set_visible_child_name("error");
    }

    pub fn show(&self, lyrics: &Lyrics) {
        self.clear_lines();
        if lyrics.instrumental {
            self.neutral_source(&format!(
                "{} identifies this recording as instrumental",
                lyrics
                    .source
                    .map(|provider| provider.label())
                    .unwrap_or("A lyrics source")
            ));
            self.set_jamkin_state(JamkinState::Listening);
            self.pages.set_visible_child_name("instrumental");
            return;
        }
        if lyrics.lines.is_empty() {
            self.no_lyrics.set_description(Some(
                "None of the enabled sources has a match for this recording.",
            ));
            self.neutral_source("No enabled source found a match");
            self.set_jamkin_state(JamkinState::Missing);
            self.pages.set_visible_child_name("none");
            return;
        }

        let provider = lyrics
            .source
            .map(|provider| provider.label())
            .unwrap_or("lyrics source");
        if lyrics.synced {
            self.source.set_label("● LIVE");
            self.source
                .set_tooltip_text(Some(&format!("Line-synchronized lyrics from {provider}")));
            self.source.remove_css_class("dim-label");
            self.source.add_css_class("lyrics-live-source");
        } else {
            self.source.set_label("UNSYNCED");
            self.source
                .set_tooltip_text(Some(&format!("Plain lyrics from {provider}")));
            self.source.remove_css_class("lyrics-live-source");
            self.source.add_css_class("dim-label");
        }
        self.set_jamkin_state(JamkinState::Singing);
        let mut timed = self.timed.borrow_mut();
        for line in &lyrics.lines {
            let classes: &[&str] = if lyrics.synced {
                &["lyrics-line"]
            } else {
                &["lyrics-line", "lyrics-plain-line"]
            };
            let label = gtk::Label::builder()
                .label(&line.text)
                .xalign(0.5)
                .justify(gtk::Justification::Center)
                .hexpand(true)
                .halign(gtk::Align::Fill)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .selectable(!lyrics.synced)
                .css_classes(classes)
                .build();
            label.set_opacity(if lyrics.synced { DISTANT_OPACITY } else { 0.9 });
            if let Some(at_ms) = line.at_ms {
                let button = gtk::Button::builder()
                    .child(&label)
                    .has_frame(false)
                    .hexpand(true)
                    .halign(gtk::Align::Fill)
                    .tooltip_text("Jump to this line")
                    .css_classes(["flat", "lyrics-line-button"])
                    .build();
                let seek = self.seek.clone();
                button.connect_clicked(move |_| seek(at_ms));
                self.lyrics_lines.append(&button);
                timed.push((at_ms, label));
            } else {
                self.lyrics_lines.append(&label);
            }
        }
        drop(timed);
        self.current.set(None);
        self.scroller.vadjustment().set_value(0.0);
        self.pages.set_visible_child_name("lyrics");
    }

    /// Repaint the local companion immediately when Preferences changes.
    /// These files ship with Jamelade; no network lookup is involved.
    pub fn set_companion(&self, companion: Companion) {
        self.companion_name
            .set_label(&format!("{} is singing along with you", companion.label()));
        self.companion_actor.set_companion(companion);
    }

    pub fn set_quality(&self, quality: JamkinQuality) {
        self.companion_actor.set_quality(quality);
    }

    pub fn set_reduced_motion(&self, reduced: bool) {
        self.companion_actor.set_reduced_motion(reduced);
    }

    /// Playback is the honest animation gate. Having synchronized lyrics only
    /// means the Jamkin *can* sing; it does not mean music is currently moving.
    pub fn set_playing(&self, playing: bool) {
        self.companion_actor.set_playing(playing);
    }

    fn neutral_source(&self, label: &str) {
        self.source.set_label(label);
        self.source.set_tooltip_text(None);
        self.source.remove_css_class("lyrics-live-source");
        self.source.add_css_class("dim-label");
    }

    fn set_jamkin_state(&self, state: JamkinState) {
        for class in JamkinState::ALL_CLASSES {
            self.companion_stage.remove_css_class(class);
        }
        self.companion_stage.add_css_class(state.css_class());
    }

    pub fn sync_position(&self, position_ms: u64) {
        let timed = self.timed.borrow();
        if timed.is_empty() || self.pages.visible_child_name().as_deref() != Some("lyrics") {
            return;
        }
        let next = timed.partition_point(|(at, _)| *at <= position_ms);
        let next = next.checked_sub(1);
        if next == self.current.get() {
            return;
        }
        let previous = self.current.replace(next);
        for animation in self.line_animations.borrow_mut().drain(..) {
            animation.pause();
        }
        let adjacent_step = previous
            .zip(next)
            .is_some_and(|(from, to)| from.abs_diff(to) <= 1);
        let mut animations = Vec::with_capacity(6);
        for (index, (_, label)) in timed.iter().enumerate() {
            for class in LineMoment::ALL_CLASSES {
                label.remove_css_class(class);
            }
            // Remove the older generic class as well so a live update from a
            // pre-polish view cannot leave stale styling attached.
            label.remove_css_class("lyrics-past");
            let moment = LineMoment::at(index, next);
            label.add_css_class(moment.css_class());
            let target_opacity = moment.opacity();

            // A normal one-line advance changes the five-line temporal halo,
            // not merely the old and new current labels. Fade that bounded
            // neighbourhood together. A large seek still animates only the two
            // endpoints, so one click cannot start hundreds of GTK animations.
            let near_transition = adjacent_step
                && (previous.is_some_and(|at| index.abs_diff(at) <= 2)
                    || next.is_some_and(|at| index.abs_diff(at) <= 2));
            if (near_transition || previous == Some(index) || next == Some(index))
                && (label.opacity() - target_opacity).abs() > 0.01
            {
                let animated_label = label.clone();
                let animation = adw::TimedAnimation::new(
                    label,
                    label.opacity(),
                    target_opacity,
                    LINE_FADE_MS,
                    adw::CallbackAnimationTarget::new(move |value| {
                        animated_label.set_opacity(value);
                    }),
                );
                animation.set_easing(adw::Easing::EaseOutCubic);
                animation.play();
                animations.push(animation);
            } else {
                label.set_opacity(target_opacity);
            }
        }
        *self.line_animations.borrow_mut() = animations;
        let Some(index) = next else { return };
        let Some((_, label)) = timed.get(index) else {
            return;
        };

        // Glide the active line towards the middle without stealing keyboard
        // focus. The old instant adjustment made every timestamp feel like a
        // page jump even though only one lyric had changed.
        if let Some(bounds) = label.compute_bounds(&self.lines_box) {
            let adjustment = self.scroller.vadjustment();
            let target =
                f64::from(bounds.y()) - (adjustment.page_size() - f64::from(bounds.height())) / 2.0;
            let high = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
            let target = target.clamp(adjustment.lower(), high);
            if let Some(animation) = self.scroll_animation.borrow_mut().take() {
                animation.pause();
            }
            if (adjustment.value() - target).abs() > 0.5 {
                let animated_adjustment = adjustment.clone();
                let animation = adw::TimedAnimation::new(
                    &self.scroller,
                    adjustment.value(),
                    target,
                    SCROLL_GLIDE_MS,
                    adw::CallbackAnimationTarget::new(move |value| {
                        animated_adjustment.set_value(value);
                    }),
                );
                animation.set_easing(adw::Easing::EaseOutCubic);
                animation.play();
                *self.scroll_animation.borrow_mut() = Some(animation);
            }
        }
    }

    fn clear_lines(&self) {
        for animation in self.line_animations.borrow_mut().drain(..) {
            animation.pause();
        }
        if let Some(animation) = self.scroll_animation.borrow_mut().take() {
            animation.pause();
        }
        let mut child = self.lyrics_lines.first_child();
        while let Some(line) = child {
            child = line.next_sibling();
            self.lyrics_lines.remove(&line);
        }
        self.timed.borrow_mut().clear();
        self.current.set(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyric_emphasis_describes_time_around_the_current_line() {
        let states: Vec<_> = (0..7).map(|index| LineMoment::at(index, Some(2))).collect();
        assert_eq!(
            states,
            [
                LineMoment::Earlier,
                LineMoment::Previous,
                LineMoment::Current,
                LineMoment::Next,
                LineMoment::Following,
                LineMoment::Distant,
                LineMoment::Distant,
            ]
        );
        let opacities: Vec<_> = states.into_iter().map(LineMoment::opacity).collect();
        assert_eq!(
            opacities,
            [
                EARLIER_OPACITY,
                PREVIOUS_OPACITY,
                1.0,
                NEXT_OPACITY,
                FOLLOWING_OPACITY,
                DISTANT_OPACITY,
                DISTANT_OPACITY,
            ]
        );
    }

    #[test]
    fn lyrics_have_a_stable_pre_roll_emphasis_before_the_first_timestamp() {
        assert_eq!(LineMoment::at(0, None), LineMoment::Distant);
        assert_eq!(LineMoment::at(50, None).opacity(), DISTANT_OPACITY);
    }
}
