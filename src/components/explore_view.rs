// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! A bounded, responsive Explore dashboard.
//!
//! This is a plain widget owner, like `DetailPage`: the shelves are replaced as
//! one network answer and do not need their own relm4 task. It receives only
//! Slipmat's parsed `Explore` types, asks the app for artwork through the same
//! callback as the library grids, and reports clicks as intent.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

use crate::components::cover::Cover;
use crate::components::grid_item::ArtRequest;
use crate::music::types::{Album, Explore, ExploreItem, ExploreSection, Playlist, Track};

const CARD_PX: i32 = 148;

#[derive(Debug, Clone)]
pub enum ExploreAction {
    Album(Album),
    Playlist(Playlist),
    PlayTracks { tracks: Vec<Track>, start: usize },
    PlayStation(String),
}

type ActionHandler = Rc<dyn Fn(ExploreAction)>;

pub struct ExploreView {
    root: gtk::Stack,
    sections: gtk::Box,
    error: adw::StatusPage,
    artwork: RefCell<HashMap<String, Vec<Cover>>>,
    request_art: ArtRequest,
    action: ActionHandler,
}

impl ExploreView {
    pub fn new(request_art: ArtRequest, action: impl Fn(ExploreAction) + 'static) -> Self {
        let sections = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(30)
            .margin_top(22)
            .margin_bottom(30)
            .margin_start(18)
            .margin_end(18)
            .build();
        let content = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(1_280)
                    .child(&sections)
                    .build(),
            )
            .build();
        content.add_css_class("plain-scroller");

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
        loading.append(
            &gtk::Label::builder()
                .label("Building your Explore page")
                .css_classes(["title-2"])
                .build(),
        );

        let empty = adw::StatusPage::builder()
            .icon_name("audio-x-generic-symbolic")
            .title("Nothing to explore yet")
            .description(
                "Apple did not return recommendations or charts. Try reloading in a moment.",
            )
            .build();
        let error = adw::StatusPage::builder()
            .icon_name("network-offline-symbolic")
            .title("Explore could not load")
            .description("Your library and playback are unaffected.")
            .build();

        let root = gtk::Stack::new();
        root.set_transition_type(gtk::StackTransitionType::Crossfade);
        root.add_named(&loading, Some("loading"));
        root.add_named(&content, Some("content"));
        root.add_named(&empty, Some("empty"));
        root.add_named(&error, Some("error"));
        root.set_visible_child_name("loading");

        Self {
            root,
            sections,
            error,
            artwork: RefCell::new(HashMap::new()),
            request_art,
            action: Rc::new(action),
        }
    }

    pub fn widget(&self) -> &gtk::Stack {
        &self.root
    }

    pub fn loading(&self) {
        self.root.set_visible_child_name("loading");
    }

    pub fn fail(&self, detail: &str) {
        // API error strings contain no response body or request metadata. A
        // bounded description still prevents a nested context chain from
        // turning this page into a wall of text.
        let detail: String = detail.chars().take(240).collect();
        self.error.set_description(Some(&detail));
        self.root.set_visible_child_name("error");
    }

    pub fn clear(&self) {
        self.remove_sections();
        self.root.set_visible_child_name("loading");
    }

    pub fn show(&self, explore: &Explore) {
        self.remove_sections();
        if explore.sections.is_empty() {
            self.root.set_visible_child_name("empty");
            return;
        }

        self.sections.append(&hero());
        for section in &explore.sections {
            self.sections.append(&self.shelf(section));
        }
        self.root.set_visible_child_name("content");
    }

    /// Paint every visible card waiting on this artwork. Shelves are bounded
    /// and not recycled, so clearing the registry on a rebuild is sufficient.
    pub fn paint(&self, key: &str, texture: &gtk::gdk::MemoryTexture) -> usize {
        let mut painted = 0;
        for cover in self.artwork.borrow().get(key).into_iter().flatten() {
            cover.set_texture(texture);
            painted += 1;
        }
        painted
    }

    fn remove_sections(&self) {
        while let Some(child) = self.sections.first_child() {
            self.sections.remove(&child);
        }
        self.artwork.borrow_mut().clear();
    }

    fn shelf(&self, section: &ExploreSection) -> gtk::Box {
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .build();

        let heading = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .margin_start(4)
            .margin_end(4)
            .build();
        heading.append(
            &gtk::Label::builder()
                .label(&section.title)
                .xalign(0.0)
                .wrap(true)
                .css_classes(["title-2"])
                .build(),
        );
        if !section.subtitle.trim().is_empty() {
            heading.append(
                &gtk::Label::builder()
                    .label(&section.subtitle)
                    .xalign(0.0)
                    .wrap(true)
                    .css_classes(["dim-label"])
                    .build(),
            );
        }
        column.append(&heading);

        let cards = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .build();
        let track_queue: Vec<Track> = section
            .items
            .iter()
            .filter_map(|item| match item {
                ExploreItem::Track(track) => Some(track.clone()),
                ExploreItem::Album(_) | ExploreItem::Playlist(_) | ExploreItem::Station(_) => None,
            })
            .collect();
        let mut track_index = 0usize;
        for item in &section.items {
            let action = match item {
                ExploreItem::Album(album) => ExploreAction::Album(album.clone()),
                ExploreItem::Playlist(playlist) => ExploreAction::Playlist(playlist.clone()),
                ExploreItem::Track(_) => {
                    let start = track_index;
                    track_index += 1;
                    ExploreAction::PlayTracks {
                        tracks: track_queue.clone(),
                        start,
                    }
                }
                ExploreItem::Station(station) => ExploreAction::PlayStation(station.id.clone()),
            };
            cards.append(&self.card(item, action));
        }

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&cards)
            .build();
        scroller.add_css_class("explore-shelf");
        column.append(&scroller);
        column
    }

    fn card(&self, item: &ExploreItem, action: ExploreAction) -> gtk::Button {
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .width_request(CARD_PX)
            .build();
        let cover = Cover::new(CARD_PX);
        cover.square(match item {
            ExploreItem::Album(_) => "media-optical-symbolic",
            ExploreItem::Playlist(_) => "view-list-symbolic",
            ExploreItem::Track(_) => "audio-x-generic-symbolic",
            ExploreItem::Station(_) => "audio-speakers-symbolic",
        });
        cover.attach_first(&body);

        body.append(
            &gtk::Label::builder()
                .label(item.title())
                .tooltip_text(item.title())
                .xalign(0.0)
                .max_width_chars(1)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["heading"])
                .build(),
        );
        let subtitle = item
            .subtitle()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let subtitle_label = gtk::Label::builder()
            .label(&subtitle)
            .xalign(0.0)
            .max_width_chars(1)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .visible(!subtitle.is_empty())
            .css_classes(["caption", "dim-label"])
            .build();
        body.append(&subtitle_label);

        if let Some(art) = item.artwork() {
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
            .tooltip_text(item.title())
            .css_classes(["flat", "explore-card"])
            .build();
        let handler = self.action.clone();
        button.connect_clicked(move |_| handler(action.clone()));
        button
    }
}

fn hero() -> gtk::Box {
    let hero = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(7)
        .margin_bottom(2)
        .css_classes(["explore-hero"])
        .build();
    hero.append(
        &gtk::Label::builder()
            .label("APPLE MUSIC")
            .xalign(0.0)
            .css_classes(["caption", "explore-kicker"])
            .build(),
    );
    hero.append(
        &gtk::Label::builder()
            .label("Your Apple Music home, made native")
            .xalign(0.0)
            .wrap(true)
            .css_classes(["title-1"])
            .build(),
    );
    hero.append(
        &gtk::Label::builder()
            .label("Personalized shelves, stations, recent listening, heavy rotation and current charts — everything playable that Apple’s public MusicKit API returns.")
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(70)
            .css_classes(["dim-label"])
            .build(),
    );
    hero
}
