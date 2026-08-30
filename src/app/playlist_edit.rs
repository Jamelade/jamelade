// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Small, explicit dialogs for Apple's documented playlist writes.
//!
//! Jamelade can create playlists and append catalog songs. Apple does not
//! publish equivalent endpoints for rename, delete, removal, or reordering,
//! so this module does not pretend those operations are available.

use relm4::adw::prelude::*;
use relm4::{ComponentSender, adw, gtk};

use super::{AppModel, AppMsg};

impl AppModel {
    pub(super) fn show_create_playlist(
        &self,
        sender: &ComponentSender<Self>,
        parent: &adw::ApplicationWindow,
    ) {
        let name = gtk::Entry::builder()
            .placeholder_text("Playlist name")
            .max_length(200)
            .activates_default(true)
            .build();
        let description = gtk::Entry::builder()
            .placeholder_text("Description (optional)")
            .max_length(1_000)
            .build();
        let fields = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(12)
            .build();
        fields.append(&name);
        fields.append(&description);

        let dialog = adw::AlertDialog::new(
            Some("Create Playlist"),
            Some("Creates an empty playlist in your Apple Music library."),
        );
        dialog.add_css_class("jamelade-themed-dialog");
        dialog.set_extra_child(Some(&fields));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("create", "Create");
        dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("create"));
        dialog.set_close_response("cancel");
        dialog.set_response_enabled("create", false);
        {
            let dialog = dialog.clone();
            name.connect_changed(move |entry| {
                dialog.set_response_enabled("create", !entry.text().trim().is_empty());
            });
        }
        {
            let sender = sender.clone();
            let name = name.clone();
            let description = description.clone();
            dialog.connect_response(None, move |_, response| {
                if response == "create" {
                    let name = name.text().trim().to_owned();
                    if !name.is_empty() {
                        sender.input(AppMsg::CreatePlaylist {
                            name,
                            description: description.text().trim().to_owned(),
                        });
                    }
                }
            });
        }
        dialog.present(Some(parent));
        name.grab_focus();
    }

    pub(super) fn show_add_to_playlist(
        &self,
        catalog_id: String,
        sender: &ComponentSender<Self>,
        parent: &adw::ApplicationWindow,
    ) {
        let editable: Vec<_> = self
            .playlists
            .iter()
            .filter(|playlist| playlist.can_edit)
            .collect();
        if editable.is_empty() {
            self.toast("Create an editable playlist first, then add this song");
            return;
        }
        let names: Vec<&str> = editable
            .iter()
            .map(|playlist| playlist.name.as_str())
            .collect();
        let ids: Vec<String> = editable
            .iter()
            .map(|playlist| playlist.id.clone())
            .collect();
        let chooser = adw::ComboRow::builder()
            .title("Playlist")
            .model(&gtk::StringList::new(&names))
            .selected(0)
            .build();
        chooser.add_css_class("preferences-value-row");
        let group = adw::PreferencesGroup::new();
        group.add_css_class("preferences-surface-group");
        group.add(&chooser);

        let dialog = adw::AlertDialog::new(
            Some("Add to Playlist"),
            Some("The song is appended to the selected Apple Music playlist."),
        );
        dialog.add_css_class("jamelade-themed-dialog");
        dialog.set_extra_child(Some(&group));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("add", "Add");
        dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("add"));
        dialog.set_close_response("cancel");
        {
            let sender = sender.clone();
            dialog.connect_response(None, move |_, response| {
                if response != "add" {
                    return;
                }
                let Some(playlist_id) = ids.get(chooser.selected() as usize).cloned() else {
                    return;
                };
                sender.input(AppMsg::AddTrackToPlaylist {
                    playlist_id,
                    catalog_id: catalog_id.clone(),
                });
            });
        }
        dialog.present(Some(parent));
    }

    /// Apple's append endpoint returns only an acceptance status, so it cannot
    /// carry the native apps' duplicate warning. Jamelade performs a bounded
    /// read first and asks explicitly before sending a second copy.
    pub(super) fn confirm_duplicate_playlist(
        &self,
        playlist_id: String,
        catalog_id: String,
        sender: &ComponentSender<Self>,
        parent: &adw::ApplicationWindow,
    ) {
        let dialog = adw::AlertDialog::new(
            Some("Song Already in Playlist"),
            Some("This playlist already contains the song. Add another copy?"),
        );
        dialog.add_css_class("jamelade-themed-dialog");
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("add", "Add Anyway");
        dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let sender = sender.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "add" {
                sender.input(AppMsg::AppendTrackToPlaylist {
                    playlist_id: playlist_id.clone(),
                    catalog_id: catalog_id.clone(),
                });
            }
        });
        dialog.present(Some(parent));
    }
}
