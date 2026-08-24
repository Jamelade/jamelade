// SPDX-FileCopyrightText: 2026 Miguel Rincon
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
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .margin_bottom(2)
            .css_classes(["explore-hero"])
            .build();
        let identity = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(22)
            .build();
        let cover = Cover::new(PORTRAIT_PX);
        cover.round(&artist.name);
        let portrait_body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cover.attach_first(&portrait_body);

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

        let biography = artist.biography.trim();
        if biography.is_empty() {
            portrait_body.set_tooltip_text(Some(&format!("Portrait of {}", artist.name)));
            identity.append(&portrait_body);
            words.append(
                &gtk::Label::builder()
                    .label("No biography is available from Apple Music.")
                    .xalign(0.0)
                    .wrap(true)
                    .use_markup(false)
                    .css_classes(["dim-label"])
                    .build(),
            );
            identity.append(&words);
            hero.append(&identity);
        } else {
            let portrait = gtk::Button::builder()
                .child(&portrait_body)
                .tooltip_text(format!("Expand the biography of {}", artist.name))
                .css_classes(["flat", "circular"])
                .build();
            identity.append(&portrait);

            let preview_row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .hexpand(true)
                .build();
            preview_row.append(
                &gtk::Label::builder()
                    .label(biography)
                    .xalign(0.0)
                    .hexpand(true)
                    .wrap(true)
                    .lines(3)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .use_markup(false)
                    .css_classes(["artist-biography-preview"])
                    .build(),
            );
            preview_row.append(&gtk::Image::from_icon_name("pan-down-symbolic"));
            let preview = gtk::Button::builder()
                .child(&preview_row)
                .tooltip_text("Read the full artist biography")
                .hexpand(true)
                .css_classes(["flat", "artist-biography-toggle"])
                .build();
            words.append(&preview);
            identity.append(&words);
            hero.append(&identity);

            let full = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(10)
                .css_classes(["artist-biography-full"])
                .build();
            full.append(
                &gtk::Label::builder()
                    .label("ABOUT THIS ARTIST")
                    .xalign(0.0)
                    .css_classes(["caption", "explore-kicker"])
                    .build(),
            );
            full.append(
                &gtk::Label::builder()
                    .label(biography)
                    .xalign(0.0)
                    .wrap(true)
                    .selectable(true)
                    .use_markup(false)
                    .build(),
            );

            let collapse_row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
            collapse_row.append(&gtk::Label::new(Some("Show less")));
            collapse_row.append(&gtk::Image::from_icon_name("pan-up-symbolic"));
            let collapse = gtk::Button::builder()
                .child(&collapse_row)
                .tooltip_text("Collapse the artist biography")
                .halign(gtk::Align::Start)
                .css_classes(["flat", "artist-biography-toggle"])
                .build();
            full.append(&collapse);

            let revealer = gtk::Revealer::builder()
                .child(&full)
                .reveal_child(false)
                .transition_duration(260)
                .transition_type(gtk::RevealerTransitionType::SlideDown)
                .build();
            hero.append(&revealer);

            let preview_after_close = preview.clone();
            revealer.connect_child_revealed_notify(move |revealer| {
                if !revealer.is_child_revealed() && !revealer.reveals_child() {
                    preview_after_close.set_visible(true);
                }
            });

            let toggle: Rc<dyn Fn()> = Rc::new({
                let revealer = revealer.clone();
                let preview = preview.clone();
                move || {
                    let expand = !revealer.reveals_child();
                    if expand {
                        preview.set_visible(false);
                    }
                    revealer.set_reveal_child(expand);
                }
            });
            {
                let toggle = toggle.clone();
                portrait.connect_clicked(move |_| toggle());
            }
            {
                let toggle = toggle.clone();
                preview.connect_clicked(move |_| toggle());
            }
            collapse.connect_clicked(move |_| toggle());
        }

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
