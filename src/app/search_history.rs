// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded local catalogue-search history and its compact header controls.

use std::cell::Cell;

use relm4::adw::prelude::*;
use relm4::{ComponentSender, gtk};
use serde::{Deserialize, Serialize};

use super::{AppModel, AppMsg, View};

const FORMAT_VERSION: u8 = 1;
const MAX_ENTRIES: usize = 20;
const MAX_QUERY_CHARS: usize = 160;
const MAX_FILE_BYTES: usize = 16 * 1024;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct History {
    entries: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u8,
    entries: Vec<String>,
}

impl History {
    pub(super) fn load() -> Self {
        let path = path();
        let Ok(data) = crate::private_storage::read_to_string(&path, MAX_FILE_BYTES) else {
            return Self::default();
        };
        Self::from_data(&data)
    }

    fn from_data(data: &str) -> Self {
        let Ok(stored) = serde_json::from_str::<Stored>(data) else {
            return Self::default();
        };
        if stored.version != FORMAT_VERSION {
            return Self::default();
        }

        let mut history = Self::default();
        for entry in stored.entries.into_iter().rev() {
            history.record(&entry);
        }
        history
    }

    pub(super) fn entries(&self) -> &[String] {
        &self.entries
    }

    pub(super) fn record(&mut self, query: &str) -> bool {
        let Some(query) = normalized(query) else {
            return false;
        };
        if self.entries.first() == Some(&query) {
            return false;
        }

        let folded = query.to_lowercase();
        self.entries.retain(|entry| entry.to_lowercase() != folded);
        self.entries.insert(0, query);
        self.entries.truncate(MAX_ENTRIES);
        true
    }

    pub(super) fn remove(&mut self, query: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry != query);
        before != self.entries.len()
    }

    pub(super) fn clear(&mut self) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        self.entries.clear();
        true
    }

    pub(super) fn save(&self) {
        let path = path();
        let Some(dir) = path.parent() else {
            return;
        };
        let Ok(data) = serde_json::to_vec(&Stored {
            version: FORMAT_VERSION,
            entries: self.entries.clone(),
        }) else {
            return;
        };
        if data.len() > MAX_FILE_BYTES
            || crate::private_storage::ensure_dir(dir).is_err()
            || crate::private_storage::write(&path, &data).is_err()
        {
            tracing::warn!("could not save search history");
        }
    }
}

fn path() -> std::path::PathBuf {
    gtk::glib::user_cache_dir()
        .join("jamelade")
        .join("search-history.json")
}

fn normalized(query: &str) -> Option<String> {
    let mut result = String::new();
    let mut space = false;
    for character in query.trim().chars() {
        if character.is_whitespace() {
            space = !result.is_empty();
            continue;
        }
        if character.is_control() {
            continue;
        }
        if space {
            result.push(' ');
            space = false;
        }
        result.push(character);
        if result.chars().count() == MAX_QUERY_CHARS {
            break;
        }
    }
    (!result.is_empty()).then_some(result)
}

pub(super) struct Controls {
    root: gtk::Box,
    panel: gtk::Box,
    pub(super) entry: gtk::SearchEntry,
    wipe_button: gtk::Button,
    list: gtk::ListBox,
    catalog: Cell<bool>,
    live: Cell<bool>,
    has_entries: Cell<bool>,
}

impl Controls {
    pub(super) fn new(sender: &ComponentSender<AppModel>, history: &History) -> Self {
        let root = gtk::Box::builder()
            .hexpand(true)
            .spacing(0)
            .css_classes(["linked"])
            .build();
        let entry = gtk::SearchEntry::builder()
            .hexpand(true)
            .max_width_chars(60)
            .build();
        {
            let sender = sender.clone();
            entry.connect_search_changed(move |entry| {
                sender.input(AppMsg::SearchChanged(entry.text().into()));
            });
        }
        {
            let sender = sender.clone();
            entry.connect_stop_search(move |entry| {
                entry.set_text("");
                sender.input(AppMsg::SearchChanged(String::new()));
                sender.input(AppMsg::ShowSearch(false));
            });
        }

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(320)
            .min_content_width(420)
            .propagate_natural_height(true)
            .child(&list)
            .build();
        let heading = gtk::Label::builder()
            .label(crate::i18n::tr("Recent searches"))
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        let hint = gtk::Label::builder()
            .label(crate::i18n::tr("Right-click an entry to remove it"))
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .build();
        let panel = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(24)
            .halign(gtk::Align::Center)
            .build();
        panel.append(&heading);
        panel.append(&scroll);
        panel.append(&hint);

        let wipe_button = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(crate::i18n::tr("Clear search history"))
            .build();
        wipe_button.add_css_class("flat");
        {
            let sender = sender.clone();
            wipe_button.connect_clicked(move |_| sender.input(AppMsg::ClearSearchHistory));
        }

        root.append(&entry);
        root.append(&wipe_button);

        let controls = Self {
            root,
            panel,
            entry,
            wipe_button,
            list,
            catalog: Cell::new(false),
            live: Cell::new(false),
            has_entries: Cell::new(false),
        };
        controls.sync(history, sender);
        controls
    }

    pub(super) fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub(super) fn panel(&self) -> &gtk::Box {
        &self.panel
    }

    pub(super) fn sync_context(&self, view: View, live: bool) {
        self.catalog.set(view == View::Search);
        self.live.set(live);
        self.entry.set_sensitive(live);
        self.entry.set_placeholder_text(Some(match view {
            View::Explore | View::Lyrics | View::Search => crate::i18n::tr("Search Apple Music"),
            View::Songs => "Search your library",
            View::Albums => "Search albums",
            View::Artists => "Search artists",
            View::Playlists => "Search playlists",
        }));
        self.update_buttons();
    }

    pub(super) fn sync(&self, history: &History, sender: &ComponentSender<AppModel>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for query in history.entries() {
            let label = gtk::Label::builder()
                .label(query)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            let button = gtk::Button::builder()
                .child(&label)
                .hexpand(true)
                .tooltip_text(crate::i18n::tr("Click to search; right-click to remove"))
                .build();
            button.add_css_class("flat");
            {
                let sender = sender.clone();
                let query = query.clone();
                button.connect_clicked(move |_| {
                    sender.input(AppMsg::UseSearchHistory(query.clone()));
                });
            }
            {
                let sender = sender.clone();
                let query = query.clone();
                let click = gtk::GestureClick::new();
                click.set_button(3);
                click.connect_pressed(move |gesture, _, _, _| {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    sender.input(AppMsg::RemoveSearchHistory(query.clone()));
                });
                button.add_controller(click);
            }
            self.list.append(&button);
        }
        self.has_entries.set(!history.entries().is_empty());
        self.update_buttons();
    }

    fn update_buttons(&self) {
        let catalog = self.catalog.get();
        let has_entries = self.has_entries.get();
        self.panel.set_visible(catalog && has_entries);
        self.wipe_button.set_visible(catalog && has_entries);
        self.wipe_button.set_sensitive(self.live.get());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_unique_queries_win_and_history_is_bounded() {
        let mut history = History::default();
        for index in 0..MAX_ENTRIES + 5 {
            history.record(&format!("query {index}"));
        }
        assert_eq!(history.entries().len(), MAX_ENTRIES);
        assert_eq!(history.entries()[0], format!("query {}", MAX_ENTRIES + 4));

        history.record("QUERY 10");
        assert_eq!(history.entries()[0], "QUERY 10");
        assert_eq!(
            history
                .entries()
                .iter()
                .filter(|entry| entry.to_lowercase() == "query 10")
                .count(),
            1
        );
    }

    #[test]
    fn stored_history_is_versioned_sanitized_and_bounded() {
        let entries: Vec<String> = (0..MAX_ENTRIES + 5)
            .map(|index| format!("  search\n{index}  "))
            .collect();
        let data = serde_json::to_string(&Stored {
            version: FORMAT_VERSION,
            entries,
        })
        .unwrap();
        let history = History::from_data(&data);
        assert_eq!(history.entries().len(), MAX_ENTRIES);
        assert!(history.entries().iter().all(|entry| !entry.contains('\n')));
        assert!(
            History::from_data(r#"{"version":99,"entries":["no"]}"#)
                .entries()
                .is_empty()
        );
        assert!(History::from_data("not json").entries().is_empty());
    }

    #[test]
    fn individual_and_complete_removal_are_explicit() {
        let mut history = History::default();
        history.record("one");
        history.record("two");
        assert!(history.remove("one"));
        assert_eq!(history.entries(), &["two"]);
        assert!(history.clear());
        assert!(!history.clear());
    }
}
