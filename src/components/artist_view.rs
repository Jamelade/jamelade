// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The artist destination: identity, newest release, top songs, albums and bio.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use relm4::adw::prelude::*;
use relm4::{adw, gtk};

use crate::components::cover::Cover;
use crate::components::grid_item::ArtRequest;
use crate::music::types::{Album, Artist, Artwork, Track};

const PORTRAIT_PX: i32 = 190;
const RELEASE_PX: i32 = 176;
const SONG_PX: i32 = 58;
const ALBUM_PX: i32 = 156;

#[derive(Debug, Clone, Copy)]
pub enum ArtistActivate {
    TopSong(usize),
    LatestRelease,
    Album(usize),
}

type Activate = Rc<dyn Fn(ArtistActivate)>;

pub struct ArtistView {
    root: gtk::ScrolledWindow,
    sections: gtk::Box,
    artwork: RefCell<HashMap<String, Vec<Cover>>>,
    request_art: ArtRequest,
    activate: Activate,
}

impl ArtistView {
    pub fn new(request_art: ArtRequest, activate: impl Fn(ArtistActivate) + 'static) -> Self {
        let sections = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(28)
            .margin_top(22)
            .margin_bottom(32)
            .margin_start(18)
            .margin_end(18)
            .build();
        let root = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(1_120)
                    .child(&sections)
                    .build(),
            )
            .build();
        root.add_css_class("plain-scroller");

        Self {
            root,
            sections,
            artwork: RefCell::new(HashMap::new()),
            request_art,
            activate: Rc::new(activate),
        }
    }

    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.root
    }

    pub fn show(
        &self,
        artist: &Artist,
        top_songs: &[Track],
        latest_release: Option<&Album>,
        albums: &[Album],
    ) {
        self.clear();
        self.sections.append(&self.hero(artist));

        if let Some(album) = latest_release {
            self.sections
                .append(&self.latest_release(album, ArtistActivate::LatestRelease));
        }
        if !top_songs.is_empty() {
            self.sections.append(&self.top_songs(top_songs));
        }
        if !albums.is_empty() {
            self.sections.append(&self.albums(albums));
        }
    }

    pub fn paint(&self, key: &str, texture: &gtk::gdk::MemoryTexture) -> usize {
        let mut painted = 0;
        for cover in self.artwork.borrow().get(key).into_iter().flatten() {
            cover.set_texture(texture);
            painted += 1;
        }
        painted
    }

    fn clear(&self) {
        while let Some(child) = self.sections.first_child() {
            self.sections.remove(&child);
        }
        self.artwork.borrow_mut().clear();
    }

    fn hero(&self, artist: &Artist) -> gtk::Box {
        let hero = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(22)
            .margin_bottom(2)
            .css_classes(["explore-hero"])
            .build();
        let cover = Cover::new(PORTRAIT_PX);
        cover.round(&artist.name);
        let portrait_body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cover.attach_first(&portrait_body);
        let portrait = gtk::Button::builder()
            .child(&portrait_body)
            .tooltip_text(format!("About {}", artist.name))
            .css_classes(["flat", "circular"])
            .build();
        {
            let parent = self.root.clone();
            let name = artist.name.clone();
            let biography = artist.biography.clone();
            portrait.connect_clicked(move |_| show_biography(&parent, &name, &biography));
        }
        hero.append(&portrait);

        let words = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .hexpand(true)
            .valign(gtk::Align::End)
            .build();
        words.append(
            &gtk::Label::builder()
                .label("ARTIST")
                .xalign(0.0)
                .css_classes(["caption", "explore-kicker"])
                .build(),
        );
        words.append(
            &gtk::Label::builder()
                .label(&artist.name)
                .xalign(0.0)
                .wrap(true)
                .use_markup(false)
                .css_classes(["title-1"])
                .build(),
        );
        if !artist.genres.is_empty() {
            words.append(
                &gtk::Label::builder()
                    .label(&artist.genres)
                    .xalign(0.0)
                    .wrap(true)
                    .use_markup(false)
                    .css_classes(["dim-label"])
                    .build(),
            );
        }
        hero.append(&words);
        self.request_cover(artist.artwork.as_ref(), cover);
        hero
    }

    fn latest_release(&self, album: &Album, action: ArtistActivate) -> gtk::Box {
        let section = section("Latest Release");
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(18)
            .build();
        let cover = Cover::new(RELEASE_PX);
        cover.square("media-optical-symbolic");
        cover.attach_first(&row);

        let words = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(7)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        if !album.year.is_empty() {
            words.append(
                &gtk::Label::builder()
                    .label(&album.year)
                    .xalign(0.0)
                    .css_classes(["caption", "explore-kicker"])
                    .build(),
            );
        }
        words.append(
            &gtk::Label::builder()
                .label(&album.name)
                .xalign(0.0)
                .wrap(true)
                .use_markup(false)
                .css_classes(["title-2"])
                .build(),
        );
        let details = match album.track_count {
            0 => "Album".to_owned(),
            1 => "Album · 1 song".to_owned(),
            count => format!("Album · {count} songs"),
        };
        words.append(
            &gtk::Label::builder()
                .label(&details)
                .xalign(0.0)
                .css_classes(["dim-label"])
                .build(),
        );
        row.append(&words);

        let button = gtk::Button::builder()
            .child(&row)
            .tooltip_text(&album.name)
            .css_classes(["flat", "explore-card"])
            .build();
        let activate = self.activate.clone();
        button.connect_clicked(move |_| activate(action));
        section.append(&button);
        self.request_cover(album.artwork.as_ref(), cover);
        section
    }

    fn top_songs(&self, tracks: &[Track]) -> gtk::Box {
        let section = section("Top Songs");
        let rows = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .build();
        for (index, track) in tracks.iter().enumerate() {
            rows.append(&self.song_row(track, index));
        }
        section.append(&rows);
        section
    }

    fn song_row(&self, track: &Track, index: usize) -> gtk::Button {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .build();
        let cover = Cover::new(SONG_PX);
        cover.square("audio-x-generic-symbolic");
        cover.attach_first(&row);

        let words = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        words.append(
            &gtk::Label::builder()
                .label(&track.title)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .use_markup(false)
                .build(),
        );
        words.append(
            &gtk::Label::builder()
                .label(&track.album)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .use_markup(false)
                .css_classes(["dim-label"])
                .build(),
        );
        row.append(&words);
        row.append(
            &gtk::Label::builder()
                .label(track.duration_label())
                .valign(gtk::Align::Center)
                .css_classes(["numeric", "dim-label"])
                .build(),
        );

        let button = gtk::Button::builder()
            .child(&row)
            .tooltip_text(format!("Play {}", track.title))
            .css_classes(["flat", "explore-card"])
            .build();
        let activate = self.activate.clone();
        button.connect_clicked(move |_| activate(ArtistActivate::TopSong(index)));
        self.request_cover(track.artwork.as_ref(), cover);
        button
    }

    fn albums(&self, albums: &[Album]) -> gtk::Box {
        let section = section("Albums");
        let cards = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .build();
        for (index, album) in albums.iter().enumerate() {
            cards.append(&self.album_card(album, index));
        }
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&cards)
            .build();
        scroller.add_css_class("explore-shelf");
        section.append(&scroller);
        section
    }

    fn album_card(&self, album: &Album, index: usize) -> gtk::Button {
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .width_request(ALBUM_PX)
            .build();
        let cover = Cover::new(ALBUM_PX);
        cover.square("media-optical-symbolic");
        cover.attach_first(&body);
        body.append(
            &gtk::Label::builder()
                .label(&album.name)
                .tooltip_text(&album.name)
                .xalign(0.0)
                .max_width_chars(1)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .use_markup(false)
                .css_classes(["heading"])
                .build(),
        );
        body.append(
            &gtk::Label::builder()
                .label(&album.year)
                .xalign(0.0)
                .visible(!album.year.is_empty())
                .css_classes(["caption", "dim-label"])
                .build(),
        );

        let button = gtk::Button::builder()
            .child(&body)
            .tooltip_text(&album.name)
            .css_classes(["flat", "explore-card"])
            .build();
        let activate = self.activate.clone();
        button.connect_clicked(move |_| activate(ArtistActivate::Album(index)));
        self.request_cover(album.artwork.as_ref(), cover);
        button
    }

    fn request_cover(&self, artwork: Option<&Artwork>, cover: Cover) {
        let Some(artwork) = artwork else {
            return;
        };
        let key = artwork.cache_key();
        self.artwork
            .borrow_mut()
            .entry(key.clone())
            .or_default()
            .push(cover);
        (self.request_art)(key, artwork.clone());
    }
}

fn section(title: &str) -> gtk::Box {
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .build();
    section.append(
        &gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .css_classes(["title-2"])
            .build(),
    );
    section
}

fn show_biography(parent: &impl IsA<gtk::Widget>, artist: &str, biography: &str) {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(24)
        .margin_bottom(30)
        .margin_start(26)
        .margin_end(26)
        .build();
    body.append(
        &gtk::Label::builder()
            .label(artist)
            .xalign(0.0)
            .wrap(true)
            .use_markup(false)
            .css_classes(["title-2"])
            .build(),
    );
    body.append(
        &gtk::Label::builder()
            .label("About this artist")
            .xalign(0.0)
            .css_classes(["caption", "explore-kicker"])
            .build(),
    );
    let text = if biography.is_empty() {
        "Apple Music does not provide a biography for this artist."
    } else {
        biography
    };
    body.append(
        &gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .use_markup(false)
            .build(),
    );

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&adw::Clamp::builder().maximum_size(620).child(&body).build())
        .build();
    scroller.add_css_class("plain-scroller");

    let header = adw::HeaderBar::new();
    let toolbar = adw::ToolbarView::builder().content(&scroller).build();
    toolbar.add_top_bar(&header);

    let dialog = adw::Dialog::builder()
        .title(format!("About {artist}"))
        .content_width(660)
        .content_height(520)
        .child(&toolbar)
        .build();
    dialog.present(Some(parent));
}
