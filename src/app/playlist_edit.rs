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
        if self.playlists.is_empty() {
            self.toast("Create a playlist first, then add this song");
            return;
        }
        let names: Vec<&str> = self
            .playlists
            .iter()
            .map(|playlist| playlist.name.as_str())
            .collect();
        let ids: Vec<String> = self
            .playlists
            .iter()
            .map(|playlist| playlist.id.clone())
            .collect();
        let chooser = adw::ComboRow::builder()
            .title("Playlist")
            .model(&gtk::StringList::new(&names))
            .selected(0)
            .build();
        let group = adw::PreferencesGroup::new();
        group.add(&chooser);

        let dialog = adw::AlertDialog::new(
            Some("Add to Playlist"),
            Some("The song is appended to the selected Apple Music playlist."),
        );
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
}
