// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use relm4::adw::prelude::*;
use relm4::{ComponentSender, adw, gtk};

use super::{AppModel, AppMsg};

impl AppModel {
    pub(super) fn show_listenbrainz_setup(
        &self,
        sender: &ComponentSender<Self>,
        parent: &adw::ApplicationWindow,
    ) {
        let token = gtk::PasswordEntry::builder()
            .placeholder_text("ListenBrainz user token")
            .show_peek_icon(true)
            .activates_default(true)
            .build();
        let dialog = adw::AlertDialog::new(
            Some("Connect ListenBrainz"),
            Some(
                "After half a song (or four minutes), Jamelade sends its title, artist, album, duration and start time to ListenBrainz. Your Apple account, identifiers and lyrics are never sent.",
            ),
        );
        dialog.set_extra_child(Some(&token));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("connect", "Save & Enable");
        dialog.set_response_appearance("connect", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("connect"));
        dialog.set_close_response("cancel");
        dialog.set_response_enabled("connect", false);
        {
            let dialog = dialog.clone();
            token.connect_changed(move |entry| {
                dialog.set_response_enabled("connect", entry.text().trim().len() >= 16);
            });
        }
        {
            let sender = sender.clone();
            let token = token.clone();
            dialog.connect_response(None, move |_, response| {
                if response == "connect" {
                    let value = token.text().to_string();
                    token.set_text("");
                    sender.input(AppMsg::EnableListenBrainz(value));
                }
            });
        }
        dialog.present(Some(parent));
        token.grab_focus();
    }
}
