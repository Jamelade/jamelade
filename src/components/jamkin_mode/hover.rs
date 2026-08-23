// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pointer lifetime for the Desktop Jamkin's lyric popover.

use std::rc::Rc;
use std::time::Duration;

use relm4::adw::prelude::*;
use relm4::gtk;

use super::InteractionState;

const LEAVE_GRACE: Duration = Duration::from_millis(120);

pub(super) fn install(
    widget: &impl IsA<gtk::Widget>,
    popover: &gtk::Popover,
    interaction: Rc<InteractionState>,
) {
    let hover = gtk::EventControllerMotion::new();
    {
        let popover = popover.clone();
        let interaction = interaction.clone();
        hover.connect_enter(move |_, _, _| {
            interaction.set_hover(true);
            popover.popup();
        });
    }
    {
        let popover = popover.clone();
        let interaction = interaction.clone();
        let controller = hover.clone();
        hover.connect_leave(move |_| {
            let hover = controller.clone();
            let popover = popover.clone();
            let interaction = interaction.clone();
            gtk::glib::timeout_add_local_once(LEAVE_GRACE, move || {
                let inside = hover.contains_pointer();
                interaction.set_hover(inside);
                if inside {
                    popover.popup();
                } else {
                    popover.popdown();
                }
            });
        });
    }
    {
        let hover = hover.clone();
        let interaction = interaction.clone();
        popover.connect_closed(move |popover| {
            if hover.contains_pointer() {
                interaction.set_hover(true);
                let popover = popover.clone();
                gtk::glib::idle_add_local_once(move || popover.popup());
            }
        });
    }
    widget.add_controller(hover);
}
