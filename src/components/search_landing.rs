// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The useful empty state for catalog search.
//!
//! Recent queries are supplied by the native model, category shortcuts are
//! fixed local intents, and Trending Now receives parsed Apple chart tracks.
//! This widget owns presentation only and never reads storage or the network.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

use crate::components::cover::Cover;
use crate::components::grid_item::ArtRequest;
use crate::music::types::Track;

const TREND_ART_PX: i32 = 64;

#[derive(Debug, Clone)]
pub enum SearchLandingAction {
    Search(String),
    Remove(String),
    Clear,
    PlayTracks { tracks: Vec<Track>, start: usize },
}

type ActionHandler = Rc<dyn Fn(SearchLandingAction)>;

pub struct SearchLanding {
    root: gtk::ScrolledWindow,
    history_section: gtk::Box,
    history_flow: gtk::FlowBox,
    trending_section: gtk::Box,
    trending_flow: gtk::FlowBox,
    artwork: RefCell<HashMap<String, Vec<Cover>>>,
    request_art: ArtRequest,
    action: ActionHandler,
}

impl SearchLanding {
    pub fn new(request_art: ArtRequest, action: impl Fn(SearchLandingAction) + 'static) -> Self {
        let action: ActionHandler = Rc::new(action);
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(30)
            .margin_top(26)
            .margin_bottom(34)
            .margin_start(26)
            .margin_end(26)
            .build();

        let history_flow = flow(4, 10, 14);
        let clear = gtk::Button::builder()
            .label(crate::i18n::tr("Clear History"))
            .css_classes(["flat", "search-clear-history"])
            .valign(gtk::Align::Center)
            .build();
        let history_heading = heading(crate::i18n::tr("Recent Searches"), Some(&clear));
        let history_section = section(&history_heading, &history_flow);
        history_section.set_visible(false);
        content.append(&history_section);

        let categories = flow(5, 12, 14);
        for (label, query, class) in [
            ("New Music", "new music", "category-new"),
            ("Hip-Hop", "hip-hop", "category-hiphop"),
            ("Indie", "indie", "category-indie"),
            ("Electronic", "electronic", "category-electronic"),
            ("Chill", "chill", "category-chill"),
        ] {
            let button = category_card(crate::i18n::tr(label), class);
            let handler = action.clone();
            let query = query.to_owned();
            button.connect_clicked(move |_| handler(SearchLandingAction::Search(query.clone())));
            categories.insert(&button, -1);
        }
        let categories_section = section(
            &heading(crate::i18n::tr("Browse Categories"), None),
            &categories,
        );
        content.append(&categories_section);

        let trending_flow = flow(5, 12, 14);
        let trending_section = section(
            &heading(crate::i18n::tr("Trending Now"), None),
            &trending_flow,
        );
        trending_section.set_visible(false);
        content.append(&trending_section);

        let root = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(1_300)
                    .child(&content)
                    .build(),
            )
            .css_classes(["plain-scroller", "search-landing"])
            .build();

        let clear_action = action.clone();
        clear.connect_clicked(move |_| clear_action(SearchLandingAction::Clear));

        Self {
            root,
            history_section,
            history_flow,
            trending_section,
            trending_flow,
            artwork: RefCell::new(HashMap::new()),
            request_art,
            action,
        }
    }

    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.root
    }

    pub fn set_history(&self, entries: &[String]) {
        remove_all(&self.history_flow);
        for query in entries {
            self.history_flow.insert(&self.history_pill(query), -1);
        }
        self.history_section.set_visible(!entries.is_empty());
    }

    pub fn set_trending(&self, tracks: Vec<Track>) {
        remove_all(&self.trending_flow);
        self.artwork.borrow_mut().clear();
        if tracks.is_empty() {
            self.trending_section.set_visible(false);
            return;
        }
        for (start, track) in tracks.iter().enumerate() {
            self.trending_flow
                .insert(&self.trending_card(track, tracks.clone(), start), -1);
        }
        self.trending_section.set_visible(true);
    }

    pub fn paint(&self, key: &str, texture: &gtk::gdk::MemoryTexture) -> usize {
        let mut painted = 0;
        for cover in self.artwork.borrow().get(key).into_iter().flatten() {
            cover.set_texture(texture);
            painted += 1;
        }
        painted
    }

    fn history_pill(&self, query: &str) -> gtk::Box {
        let text = gtk::Label::builder()
            .label(query)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .xalign(0.0)
            .hexpand(true)
            .build();
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .hexpand(true)
            .build();
        body.append(&gtk::Image::from_icon_name("document-open-recent-symbolic"));
        body.append(&text);
        let open = gtk::Button::builder()
            .child(&body)
            .hexpand(true)
            .css_classes(["flat", "search-history-open"])
            .build();
        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text("Remove from search history")
            .css_classes(["flat", "circular"])
            .build();
        let pill = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(2)
            .hexpand(true)
            .css_classes(["search-history-pill"])
            .build();
        pill.append(&open);
        pill.append(&close);

        let handler = self.action.clone();
        let selected = query.to_owned();
        open.connect_clicked(move |_| handler(SearchLandingAction::Search(selected.clone())));
        let handler = self.action.clone();
        let removed = query.to_owned();
        close.connect_clicked(move |_| handler(SearchLandingAction::Remove(removed.clone())));

        let context = gtk::GestureClick::new();
        context.set_button(gtk::gdk::BUTTON_SECONDARY);
        let handler = self.action.clone();
        let removed = query.to_owned();
        context.connect_pressed(move |gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            handler(SearchLandingAction::Remove(removed.clone()));
        });
        pill.add_controller(context);
        pill
    }

    fn trending_card(&self, track: &Track, tracks: Vec<Track>, start: usize) -> gtk::Button {
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .build();
        let cover = Cover::new(TREND_ART_PX);
        cover.square("audio-x-generic-symbolic");
        cover.attach_first(&body);
        let labels = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(1)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        labels.append(&landing_label(&track.title, &["heading"]));
        labels.append(&landing_label(&track.artist, &["caption", "dim-label"]));
        labels.append(&landing_label(
            "Apple Music chart",
            &["caption", "dim-label"],
        ));
        body.append(&labels);

        if let Some(art) = &track.artwork {
            let key = art.cache_key();
            self.artwork
                .borrow_mut()
                .entry(key.clone())
                .or_default()
                .push(cover);
            (self.request_art)(key, art.clone());
        }

        let button = gtk::Button::builder()
            .child(&body)
            .tooltip_text(&track.title)
            .css_classes(["flat", "search-trending-card"])
            .build();
        let handler = self.action.clone();
        button.connect_clicked(move |_| {
            handler(SearchLandingAction::PlayTracks {
                tracks: tracks.clone(),
                start,
            })
        });
        button
    }
}

fn flow(columns: u32, row_spacing: u32, column_spacing: u32) -> gtk::FlowBox {
    gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(1)
        .max_children_per_line(columns)
        .row_spacing(row_spacing)
        .column_spacing(column_spacing)
        .build()
}

fn heading(title: &str, suffix: Option<&gtk::Button>) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    row.append(
        &gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["title-2"])
            .build(),
    );
    if let Some(suffix) = suffix {
        row.append(suffix);
    }
    row
}

fn section(heading: &gtk::Box, content: &impl IsA<gtk::Widget>) -> gtk::Box {
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();
    section.append(heading);
    section.append(content);
    section
}

fn category_card(label: &str, class: &str) -> gtk::Button {
    let title = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .valign(gtk::Align::End)
        .vexpand(true)
        .css_classes(["heading", "search-category-title"])
        .build();
    gtk::Button::builder()
        .child(&title)
        .width_request(170)
        .height_request(108)
        .css_classes(["flat", "search-category-card", class])
        .build()
}

fn landing_label(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .max_width_chars(1)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(classes)
        .build()
}

fn remove_all(flow: &gtk::FlowBox) {
    while let Some(child) = flow.first_child() {
        flow.remove(&child);
    }
}
