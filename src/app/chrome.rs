// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! The window's furniture: the primary menu's actions and accelerators, and the
//! three dialogs behind them.
//!
//! All built imperatively rather than in `view!`, because they are presented on
//! demand and own no state of their own — every change one of them makes goes
//! straight back through an `AppMsg`, so the reducer stays the only writer.

use relm4::adw::prelude::*;
use relm4::{ComponentSender, adw, gtk};

use super::{AppModel, AppMsg};
use crate::companion::Companion;
use crate::components::jamkin_mode::JamkinMode;
use crate::settings::{JamkinQuality, Theme};
use crate::style::Accent;

impl super::AppModel {
    /// Point the sort popover at the section now showing.
    ///
    /// Both halves are needed. The **menu** changes because the keys differ per
    /// section, and the **action states** change because each section remembers
    /// its own choice — without the second, the radio dot would sit on whatever
    /// the last section chose and lie about the list underneath it.
    ///
    /// Artists get no key list at all: a library artist carries only a name, so
    /// there is nothing to choose between and the popover is just the direction
    /// toggle.
    pub(super) fn sync_sort_menu(&self, button: &gtk::MenuButton) {
        use gtk::prelude::ToVariant;
        let sort = self.sorts.get(self.view);

        let menu = gtk::gio::Menu::new();
        let direction = gtk::gio::Menu::new();
        direction.append(Some("_Reverse Order"), Some("sort.reverse"));
        menu.append_section(None, &direction);
        if super::SortBy::for_view(self.view).len() > 1 {
            menu.prepend_section(None, &sort_keys_menu(self.view));
        }
        button.set_menu_model(Some(&menu));

        if let Some((by, reverse)) = &self.sort_actions {
            by.set_state(&sort.by.id().to_variant());
            reverse.set_state(&sort.reversed.to_variant());
        }
    }
}

/// The radio list in the sort popover, for whatever section is showing.
///
/// Rebuilt on every section change rather than filtered from one fixed list,
/// because the keys are not a subset of each other: a playlist has no artist,
/// an album has a date added and a song does not. See `SortBy::for_view` for
/// which are honest where — it is a measurement, not a preference.
pub(super) fn sort_keys_menu(view: super::View) -> gtk::gio::Menu {
    use gtk::prelude::ToVariant;
    let keys = gtk::gio::Menu::new();
    for option in super::SortBy::for_view(view) {
        let item = gtk::gio::MenuItem::new(Some(option.label()), None);
        item.set_action_and_target_value(Some("sort.by"), Some(&option.id().to_variant()));
        keys.append_item(&item);
    }
    keys
}

// The primary menu's action group. GTK menu items invoke `GAction`s by name;
// each of these bridges to an `AppMsg` so the reducer stays the only place
// state changes.
relm4::new_action_group!(AppMenuActionGroup, "win");
relm4::new_stateless_action!(PreferencesAction, AppMenuActionGroup, "preferences");
relm4::new_stateless_action!(NewPlaylistAction, AppMenuActionGroup, "new-playlist");
relm4::new_stateless_action!(ShortcutsAction, AppMenuActionGroup, "shortcuts");
relm4::new_stateless_action!(AboutAction, AppMenuActionGroup, "about");
relm4::new_stateless_action!(PlayPauseAction, AppMenuActionGroup, "play-pause");
relm4::new_stateless_action!(NextAction, AppMenuActionGroup, "next");
relm4::new_stateless_action!(PreviousAction, AppMenuActionGroup, "previous");
relm4::new_stateless_action!(VolumeUpAction, AppMenuActionGroup, "volume-up");
relm4::new_stateless_action!(VolumeDownAction, AppMenuActionGroup, "volume-down");
relm4::new_stateless_action!(CloseWindowAction, AppMenuActionGroup, "close-window");
relm4::new_stateless_action!(ToggleQueueAction, AppMenuActionGroup, "toggle-queue");
relm4::new_stateless_action!(ToggleSidebarAction, AppMenuActionGroup, "toggle-sidebar");
relm4::new_stateless_action!(SignOutAction, AppMenuActionGroup, "sign-out");
relm4::new_stateless_action!(FocusSearchAction, AppMenuActionGroup, "focus-search");
relm4::new_stateless_action!(SupportAction, AppMenuActionGroup, "support");

/// Wire the primary menu's actions to messages, with their accelerators.
pub(super) fn register_actions(
    window: &adw::ApplicationWindow,
    sender: &ComponentSender<AppModel>,
) {
    use relm4::actions::{AccelsPlus, RelmAction, RelmActionGroup};

    let mut group = RelmActionGroup::<AppMenuActionGroup>::new();

    let s = sender.clone();
    group.add_action(RelmAction::<PreferencesAction>::new_stateless(move |_| {
        s.input(AppMsg::ShowPreferences)
    }));
    let s = sender.clone();
    group.add_action(RelmAction::<NewPlaylistAction>::new_stateless(move |_| {
        s.input(AppMsg::ShowCreatePlaylist)
    }));
    let s = sender.clone();
    group.add_action(RelmAction::<ShortcutsAction>::new_stateless(move |_| {
        s.input(AppMsg::ShowShortcuts)
    }));
    let s = sender.clone();
    group.add_action(RelmAction::<SignOutAction>::new_stateless(move |_| {
        s.input(AppMsg::SignOut)
    }));
    let s = sender.clone();
    group.add_action(RelmAction::<AboutAction>::new_stateless(move |_| {
        s.input(AppMsg::ShowAbout)
    }));
    // **Application-scoped, not window-scoped.** A `win.` action resolves
    // through whatever currently holds focus, and the first-run gate is an
    // `adw::Dialog` presented into the window's own dialog host — so the one
    // moment a user most needs a way out is the moment that scope is least
    // certain. `app.quit` is reachable from any focus scope, and is the GNOME
    // convention besides.
    //
    // It matters more than it looks: an `adw::Dialog` with `can_close(false)`
    // also blocks the window's close request, so while the gate is up the title
    // bar button does nothing either. Between that and Quit missing from the
    // primary menu, a signed-out app had no visible way to exit at all.
    let app = relm4::main_application();
    let quit = gtk::gio::SimpleAction::new("quit", None);
    quit.connect_activate(|_, _| crate::notify::quit_cleanly());
    app.add_action(&quit);

    // Transport, so the app answers the keyboard even when the bar does not
    // have focus. Media keys already arrive over MPRIS; these are the
    // in-window equivalents.
    let s = sender.clone();
    group.add_action(RelmAction::<PlayPauseAction>::new_stateless(move |_| {
        s.input(AppMsg::PlayPause)
    }));
    let s = sender.clone();
    group.add_action(RelmAction::<NextAction>::new_stateless(move |_| {
        s.input(AppMsg::Next)
    }));
    let s = sender.clone();
    group.add_action(RelmAction::<PreviousAction>::new_stateless(move |_| {
        s.input(AppMsg::Previous)
    }));
    let s = sender.clone();
    group.add_action(RelmAction::<VolumeUpAction>::new_stateless(move |_| {
        s.input(AppMsg::VolumeUp)
    }));
    let s = sender.clone();
    group.add_action(RelmAction::<VolumeDownAction>::new_stateless(move |_| {
        s.input(AppMsg::VolumeDown)
    }));
    // The keyboard equivalent of the close button, and deliberately the *same*
    // message — so it inherits the same two meanings: hide and keep playing when
    // something is loaded, quit when nothing is. A `Ctrl`+`W` that quit outright
    // while the close button did not would be the worse kind of surprise.
    let s = sender.clone();
    group.add_action(RelmAction::<CloseWindowAction>::new_stateless(move |_| {
        s.input(AppMsg::WindowCloseRequested)
    }));

    let s = sender.clone();
    group.add_action(RelmAction::<ToggleQueueAction>::new_stateless(move |_| {
        s.input(AppMsg::ToggleQueue)
    }));
    let s = sender.clone();
    group.add_action(RelmAction::<ToggleSidebarAction>::new_stateless(
        move |_| s.input(AppMsg::ToggleSidebar),
    ));
    let s = sender.clone();
    group.add_action(RelmAction::<FocusSearchAction>::new_stateless(move |_| {
        s.input(AppMsg::FocusSearch)
    }));
    let s = sender.clone();
    group.add_action(RelmAction::<SupportAction>::new_stateless(move |_| {
        s.input(AppMsg::OpenSupport)
    }));

    app.set_accelerators_for_action::<PreferencesAction>(&["<Control>comma"]);
    app.set_accelerators_for_action::<ShortcutsAction>(&["<Control>question"]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accelerators_for_action::<CloseWindowAction>(&["<Control>w"]);
    app.set_accelerators_for_action::<PlayPauseAction>(&["<Control>k"]);
    app.set_accelerators_for_action::<NextAction>(&["<Control>Right"]);
    app.set_accelerators_for_action::<PreviousAction>(&["<Control>Left"]);
    app.set_accelerators_for_action::<VolumeUpAction>(&["<Control>Up"]);
    app.set_accelerators_for_action::<VolumeDownAction>(&["<Control>Down"]);
    app.set_accelerators_for_action::<ToggleQueueAction>(&["<Control>u"]);
    // F9 is the GNOME convention for showing and hiding a sidebar.
    app.set_accelerators_for_action::<ToggleSidebarAction>(&["F9"]);
    app.set_accelerators_for_action::<FocusSearchAction>(&["<Control>f"]);

    group.register_for_widget(window);
}

/// Check an icon name against the theme, falling back if it is missing.
///
/// A name that does not exist renders as nothing at all — silently, with no
/// warning — which is how `music-note-single-symbolic` shipped as an invisible
/// icon.
pub(super) fn icon(name: &'static str) -> &'static str {
    let present = gtk::gdk::Display::default()
        .map(|display| gtk::IconTheme::for_display(&display))
        .is_some_and(|theme| theme.has_icon(name));
    if present {
        name
    } else {
        tracing::warn!(icon = name, "icon missing from the theme; falling back");
        "audio-x-generic-symbolic"
    }
}

/// Put a bundled Jamkin beside its Preferences row. Missing artwork is a
/// cosmetic packaging error, so the row remains usable and simply hides it.
fn set_companion_preview(picture: &gtk::Picture, companion: Companion) {
    let paintable = companion.image_path().and_then(|path| {
        gtk::gdk::Texture::from_filename(&path)
            .map(|texture| texture.upcast::<gtk::gdk::Paintable>())
            .inspect_err(|_| {
                tracing::warn!("could not load Jamkin preview");
            })
            .ok()
    });
    picture.set_paintable(paintable.as_ref());
    picture.set_tooltip_text(Some(companion.label()));
    picture.set_visible(paintable.is_some());
}

/// Preview the exact rounded square that the desktop portal will install.
fn set_launcher_preview(picture: &gtk::Picture, companion: Companion) {
    let paintable = companion.launcher_icon_path().and_then(|path| {
        gtk::gdk::Texture::from_filename(&path)
            .map(|texture| texture.upcast::<gtk::gdk::Paintable>())
            .inspect_err(|_| {
                tracing::warn!("could not load launcher preview");
            })
            .ok()
    });
    picture.set_paintable(paintable.as_ref());
    picture.set_tooltip_text(Some(&format!("{} app icon", companion.label())));
    picture.set_visible(paintable.is_some());
}

pub(super) fn show_about(parent: &adw::ApplicationWindow) {
    let about = adw::AboutDialog::builder()
        .application_name(crate::APP_NAME)
        .application_icon(crate::APP_ID)
        .developer_name("Miguel Rincon and Jamelade contributors")
        .version(env!("CARGO_PKG_VERSION"))
        .license_type(gtk::License::Gpl30)
        .website("https://github.com/Jamelade/jamelade")
        .issue_url("https://github.com/Jamelade/jamelade/issues")
        // The primary menu carries this too. Both, because the menu is where
        // it is *seen* and About is where somebody goes looking for it.
        .support_url(SUPPORT_URL)
        .comments(
            "A native Linux desktop client for Apple Music.\n\n\
             Playback runs through Apple's own MusicKit player using Google's \
             Widevine CDM, in a hidden helper process. Jamelade is an unofficial \
             community fork of Slipmat and requires an active Apple Music \
             subscription and an internet connection.",
        )
        .build();
    about.present(Some(parent));
}

/// Where to say thank you.
pub(super) const SUPPORT_URL: &str = "https://ko-fi.com/miguelrincon";

/// Open the support page in the user's browser.
///
/// `GtkUriLauncher` rather than `gio::AppInfo`: inside a Flatpak it goes
/// through the OpenURI portal, which is the only route out of the sandbox —
/// and it is the same call whether sandboxed or not, so there is nothing to
/// branch on.
pub(super) fn open_support(parent: &adw::ApplicationWindow) {
    gtk::UriLauncher::new(SUPPORT_URL).launch(
        Some(parent),
        gtk::gio::Cancellable::NONE,
        |result| {
            // Nothing to recover: if no browser answered, a toast telling them
            // so would be one more thing that cannot open a browser either.
            if let Err(err) = result {
                tracing::warn!(?err, "could not open the support page");
            }
        },
    );
}

pub(super) fn show_shortcuts(parent: &adw::ApplicationWindow) {
    // Built by hand rather than from a .ui file: it is a dozen lines either
    // way, and this keeps the strings next to the code that implements them.
    let dialog = adw::ShortcutsDialog::new();

    let playback = adw::ShortcutsSection::new(Some("Playback"));
    for (title, accel) in [
        ("Play or pause", "<Control>k"),
        ("Next track", "<Control>Right"),
        ("Previous track", "<Control>Left"),
        ("Volume up", "<Control>Up"),
        ("Volume down", "<Control>Down"),
    ] {
        playback.add(adw::ShortcutsItem::new(title, accel));
    }

    let general = adw::ShortcutsSection::new(Some("General"));
    for (title, accel) in [
        ("Search", "<Control>f"),
        ("Close the window", "<Control>w"),
        ("Toggle the sidebar", "F9"),
        ("Toggle the queue", "<Control>u"),
        ("Preferences", "<Control>comma"),
        ("Keyboard shortcuts", "<Control>question"),
        ("Quit", "<Control>q"),
    ] {
        general.add(adw::ShortcutsItem::new(title, accel));
    }

    dialog.add(playback);
    dialog.add(general);
    dialog.present(Some(parent));
}

pub(super) fn show_credits(
    parent: &adw::ApplicationWindow,
    credits: &[crate::music::client::SongCredit],
) {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(20)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    if credits.is_empty() {
        body.append(
            &adw::StatusPage::builder()
                .icon_name("avatar-default-symbolic")
                .title("No credits supplied")
                .description("Apple Music did not return credits for this recording.")
                .build(),
        );
    } else {
        for credit in credits {
            let role = gtk::Label::builder()
                .xalign(0.0)
                .wrap(true)
                .css_classes(["title-4", "accent"])
                .build();
            role.set_label(&credit.role);
            let names = gtk::Label::builder()
                .xalign(0.0)
                .wrap(true)
                .selectable(true)
                .build();
            names.set_label(&credit.names.join(", "));
            body.append(&role);
            body.append(&names);
        }
    }

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&body)
            .build(),
    ));
    let dialog = adw::Dialog::builder()
        .title("Song Credits")
        .content_width(480)
        .content_height(520)
        .child(&toolbar)
        .build();
    dialog.present(Some(parent));
}

impl AppModel {
    /// The first-run gate.
    ///
    /// A modal that cannot be dismissed, rather than a page behind a usable
    /// window. Signed out, every control in the app is a control that cannot
    /// work: the sidebar sections fire library loads, the search box queries a
    /// catalog that answers 403, and the transport talks to a player with no
    /// session. Leaving them reachable meant a 403 per second against Apple —
    /// blocking is not a nicety here, it is the correct behaviour.
    ///
    /// Dismissed from `update` the moment the sidecar reports an authorized
    /// session, never by the user.
    pub(super) fn present_onboarding(
        &self,
        sender: &ComponentSender<Self>,
        parent: &adw::ApplicationWindow,
    ) -> adw::Dialog {
        let page = adw::StatusPage::builder()
            .icon_name(crate::APP_ID)
            .title("Welcome to Jamelade")
            .description(
                "Jamelade plays your Apple Music library as a native Linux app. \
                 It needs an active Apple Music subscription.",
            )
            .build();

        let button = gtk::Button::builder()
            .label("Sign In to Apple Music")
            .halign(gtk::Align::Center)
            .css_classes(["suggested-action", "pill"])
            .build();
        {
            let sender = sender.clone();
            button.connect_clicked(move |_| sender.input(AppMsg::SignIn));
        }

        // Said before the button is pressed, not after. A browser window
        // opening out of a native app is alarming when it is a surprise, and
        // this is the one moment Slipmat cannot hide the web engine.
        let note = gtk::Label::builder()
            .label(
                "Apple's own sign-in page opens in a separate window, including \
                 two-factor if your account uses it. It closes for good once you're in.",
            )
            .justify(gtk::Justification::Center)
            .wrap(true)
            .max_width_chars(46)
            .css_classes(["caption", "dim-label"])
            .build();

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .halign(gtk::Align::Center)
            .spacing(18)
            .build();
        column.append(&button);
        column.append(&note);
        page.set_child(Some(&column));

        // A way out of the app, on the one screen that otherwise has none:
        // `can_close(false)` stops the window's own close button too, so
        // without this the gate is a dead end for anyone who does not want to
        // sign in right now.
        //
        // In the corner rather than under the call to action. Below Sign In it
        // sat in the reading order as if it were the second step, and it is not
        // a step at all — it is the way out. Flat, and not destructive:
        // quitting is ordinary, and red would imply it discards something.
        let quit = gtk::Button::builder()
            .label("Quit")
            .css_classes(["flat"])
            .build();
        quit.connect_clicked(|_| crate::notify::quit_cleanly());

        // The bar exists only to hold that button — the dialog cannot be
        // closed, so there are no window controls to show and no title to
        // repeat above the status page's own.
        let header = adw::HeaderBar::builder()
            .show_start_title_buttons(false)
            .show_end_title_buttons(false)
            .css_classes(["flat"])
            .build();
        header.set_title_widget(Some(&gtk::Label::new(None)));
        header.pack_end(&quit);

        let view = adw::ToolbarView::builder().content(&page).build();
        view.add_top_bar(&header);

        // **Width only.** A fixed `content_height` was what made this scroll:
        // `adw::StatusPage` puts its content in a scrolled window, so any
        // height smaller than the natural one produces a scrollbar — and 420
        // was smaller, on a dialog with a heading, two short paragraphs and a
        // button. Left unset, the dialog takes the height its content asks for
        // and there is nothing to scroll.
        let dialog = adw::Dialog::builder()
            .child(&view)
            .content_width(480)
            // No escape, no click-outside: there is nothing behind this worth
            // reaching until there is a session.
            .can_close(false)
            .build();

        // Ctrl+Q, again, because the gate swallows the application one.
        //
        // Moving the action from `win.quit` to `app.quit` was not enough:
        // tested by hand with the gate up, the Quit button works — so
        // `main_application().quit()` is fine — while the accelerator never
        // arrives. A modal `adw::Dialog` holds the focus, and the application
        // shortcut does not survive that, whatever the scope of the action
        // behind it.
        //
        // So the dialog carries its own, local to it and its children, which is
        // exactly where the key is going. A `CallbackAction` rather than a
        // `NamedAction`: nothing to resolve by name, so there is no second
        // lookup that can fail the same quiet way the first one did.
        // **Capture, not bubble.** A bubbling controller runs on the way back
        // up, which is after anything nearer the focus has had its chance to
        // stop the event — and something is stopping it, or the application
        // accelerator would have worked. Capture runs on the way *down* from
        // the dialog, before its own children see the key, so nothing gets to
        // swallow it first. Safe here only because the gate holds no text
        // input: on a dialog with an entry, capturing Ctrl+Q would take it away
        // from the entry, which is why this is not the default anywhere else.
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_scope(gtk::ShortcutScope::Local);
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
        shortcuts.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("<Control>q"),
            Some(gtk::CallbackAction::new(|_, _| {
                crate::notify::quit_cleanly();
                gtk::glib::Propagation::Stop
            })),
        ));
        dialog.add_controller(shortcuts);

        dialog.present(Some(parent));
        dialog
    }

    /// Ask before signing out.
    ///
    /// Destructive and not obviously reversible from the user's side: it drops
    /// Apple's session, so getting back in means the login window again, with
    /// whatever two-factor prompt that involves. Worth a question.
    pub(super) fn confirm_sign_out(
        &self,
        sender: &ComponentSender<Self>,
        parent: &adw::ApplicationWindow,
    ) {
        let dialog = adw::AlertDialog::new(
            Some("Sign out of Apple Music?"),
            Some(
                "Jamelade will forget this session. Signing back in opens Apple's \
                 login window again.",
            ),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("sign-out", "Sign Out");
        dialog.set_response_appearance("sign-out", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let sender = sender.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "sign-out" {
                sender.input(AppMsg::SignOutConfirmed);
            }
        });
        dialog.present(Some(parent));
    }

    /// Preferences: appearance, and the track-change notification.
    ///
    /// Built imperatively rather than in `view!` because it is presented on
    /// demand and owns no state of its own — every change goes straight back
    /// through `AppMsg` so the reducer stays the only writer.
    pub(super) fn show_preferences(
        &self,
        sender: &ComponentSender<Self>,
        parent: &adw::ApplicationWindow,
    ) {
        let dialog = adw::PreferencesDialog::new();
        let page = adw::PreferencesPage::new();

        let appearance = adw::PreferencesGroup::builder()
            .title(crate::i18n::tr("Appearance"))
            .build();
        let language_names: Vec<&str> = crate::i18n::Language::ALL
            .iter()
            .map(|language| language.label())
            .collect();
        let language = adw::ComboRow::builder()
            .title(crate::i18n::tr("Language"))
            .subtitle("Applies after restarting Jamelade")
            .model(&gtk::StringList::new(&language_names))
            .selected(self.settings.language.index())
            .build();
        {
            let sender = sender.clone();
            language.connect_selected_notify(move |row| {
                sender.input(AppMsg::SetLanguage(row.selected()));
            });
        }
        appearance.add(&language);
        let theme_names: Vec<&str> = Theme::ALL.iter().map(|theme| theme.label()).collect();
        let theme = adw::ComboRow::builder()
            .title(crate::i18n::tr("Theme"))
            .model(&gtk::StringList::new(&theme_names))
            .selected(self.settings.theme.index())
            .build();
        {
            let sender = sender.clone();
            theme.connect_selected_notify(move |row| {
                sender.input(AppMsg::SetTheme(row.selected()));
            });
        }
        appearance.add(&theme);

        let names: Vec<&str> = Accent::ALL.iter().map(|a| a.label()).collect();
        let accent = adw::ComboRow::builder()
            .title(crate::i18n::tr("Accent Colour"))
            .model(&gtk::StringList::new(&names))
            .selected(Accent::parse(&self.settings.accent).index())
            .build();
        {
            let sender = sender.clone();
            accent.connect_selected_notify(move |row| {
                sender.input(AppMsg::SetAccent(Accent::from_index(row.selected())));
            });
        }
        appearance.add(&accent);

        // One switch for both parts of album-aware glass: the cover and its
        // extracted colours. Off, named themes own every surface and stock
        // Light/Dark fall back to their quiet Jamkin-accented material.
        let backdrop = adw::SwitchRow::builder()
            .title(crate::i18n::tr("Album Liquid Glass"))
            .subtitle("Blend the current cover and its colours through the window")
            .active(self.settings.player_backdrop)
            .build();
        {
            let sender = sender.clone();
            backdrop.connect_active_notify(move |row| {
                sender.input(AppMsg::SetPlayerBackdrop(row.is_active()));
            });
        }
        appearance.add(&backdrop);

        let glass_row = adw::ActionRow::builder()
            .title(crate::i18n::tr("Transparency &amp; Blur"))
            .subtitle("Higher values reveal more album art; 100 is fully clear")
            .build();
        let glass = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 5.0);
        glass.set_value(f64::from(self.settings.glass_strength));
        glass.set_digits(0);
        glass.set_draw_value(true);
        glass.set_value_pos(gtk::PositionType::Right);
        glass.set_width_request(190);
        glass.set_valign(gtk::Align::Center);
        glass.set_tooltip_text(Some("Subtle glass to fully clear at 100"));
        {
            let sender = sender.clone();
            glass.connect_value_changed(move |scale| {
                // Five-point steps keep dragging smooth while avoiding a flood
                // of persisted settings and redundant blurred cache variants.
                let strength = ((scale.value() / 5.0).round() * 5.0).clamp(0.0, 100.0) as u8;
                sender.input(AppMsg::SetGlassStrength(strength));
            });
        }
        glass_row.add_suffix(&glass);
        appearance.add(&glass_row);

        let lyric_colour_row = adw::ActionRow::builder()
            .title(crate::i18n::tr("Nearby Lyric Colour"))
            .subtitle("Balances Jamkin colour across the previous and upcoming lines")
            .build();
        let lyric_colour = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 5.0);
        lyric_colour.set_value(f64::from(self.settings.lyrics_accent_strength));
        lyric_colour.set_digits(0);
        lyric_colour.set_draw_value(true);
        lyric_colour.set_value_pos(gtk::PositionType::Right);
        lyric_colour.set_width_request(190);
        lyric_colour.set_valign(gtk::Align::Center);
        lyric_colour.set_tooltip_text(Some("Neutral to colourful nearby lyrics"));
        {
            let sender = sender.clone();
            lyric_colour.connect_value_changed(move |scale| {
                let strength = ((scale.value() / 5.0).round() * 5.0).clamp(0.0, 100.0) as u8;
                sender.input(AppMsg::SetLyricsAccentStrength(strength));
            });
        }
        lyric_colour_row.add_suffix(&lyric_colour);
        appearance.add(&lyric_colour_row);

        let lyric_size_row = adw::ActionRow::builder()
            .title(crate::i18n::tr("Lyric Text Size"))
            .subtitle("Scales the full lyrics view and Desktop Jamkin bubble")
            .build();
        let lyric_size = gtk::Scale::with_range(
            gtk::Orientation::Horizontal,
            f64::from(crate::settings::MIN_LYRICS_FONT_SCALE),
            f64::from(crate::settings::MAX_LYRICS_FONT_SCALE),
            5.0,
        );
        lyric_size.set_value(f64::from(self.settings.lyrics_font_scale));
        lyric_size.set_digits(0);
        lyric_size.set_draw_value(true);
        lyric_size.set_value_pos(gtk::PositionType::Right);
        lyric_size.set_width_request(190);
        lyric_size.set_valign(gtk::Align::Center);
        lyric_size.set_tooltip_text(Some("Lyric text size as a percentage"));
        {
            let sender = sender.clone();
            lyric_size.connect_value_changed(move |scale| {
                let percent = ((scale.value() / 5.0).round() * 5.0) as u8;
                sender.input(AppMsg::SetLyricsFontScale(percent));
            });
        }
        lyric_size_row.add_suffix(&lyric_size);
        appearance.add(&lyric_size_row);

        let jamkin = adw::PreferencesGroup::builder()
            .title(crate::i18n::tr("Jamkin Companion"))
            .description("Appears beside lyrics; Match Jamkin also follows its palette")
            .build();
        let companion_names: Vec<&str> = Companion::ALL.iter().map(|c| c.label()).collect();
        let companion = adw::ComboRow::builder()
            .title(crate::i18n::tr("Companion"))
            .subtitle(self.settings.companion.personality())
            .model(&gtk::StringList::new(&companion_names))
            .selected(self.settings.companion.index())
            .build();
        let preview = gtk::Picture::builder()
            .width_request(72)
            .height_request(72)
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .margin_top(6)
            .margin_bottom(6)
            .css_classes(["jamkin-portrait"])
            .build();
        set_companion_preview(&preview, self.settings.companion);
        companion.add_prefix(&preview);
        {
            let sender = sender.clone();
            let preview = preview.clone();
            companion.connect_selected_notify(move |row| {
                let selected = Companion::from_index(row.selected());
                row.set_subtitle(selected.personality());
                set_companion_preview(&preview, selected);
                sender.input(AppMsg::SetCompanion(selected));
            });
        }
        jamkin.add(&companion);

        let quality = adw::ComboRow::builder()
            .title(crate::i18n::tr("Jamkin Image Quality"))
            .subtitle(self.settings.jamkin_quality.subtitle())
            .model(&gtk::StringList::new(&[
                "Automatic",
                "High Resolution",
                "Performance",
            ]))
            .selected(self.settings.jamkin_quality.index())
            .build();
        {
            let sender = sender.clone();
            quality.connect_selected_notify(move |row| {
                let selected = JamkinQuality::from_index(row.selected());
                row.set_subtitle(selected.subtitle());
                sender.input(AppMsg::SetJamkinQuality(selected));
            });
        }
        jamkin.add(&quality);

        let reduced_motion = adw::SwitchRow::builder()
            .title(crate::i18n::tr("Reduce Jamkin Motion"))
            .subtitle("Uses a still pose and makes Edge Walk moves instant")
            .active(self.settings.jamkin_reduced_motion)
            .build();
        {
            let sender = sender.clone();
            reduced_motion.connect_active_notify(move |row| {
                sender.input(AppMsg::SetJamkinReducedMotion(row.is_active()));
            });
        }
        jamkin.add(&reduced_motion);

        let launcher_preview = gtk::Picture::builder()
            .width_request(64)
            .height_request(64)
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .margin_top(7)
            .margin_bottom(7)
            .css_classes(["launcher-tile-preview"])
            .build();
        launcher_preview.set_overflow(gtk::Overflow::Hidden);
        set_launcher_preview(&launcher_preview, self.settings.launcher_icon);
        let launcher_icon = adw::ComboRow::builder()
            .title(crate::i18n::tr("App Icon"))
            .subtitle(crate::launcher_icon::PREFERENCE_HELP)
            .subtitle_lines(3)
            .model(&gtk::StringList::new(&companion_names))
            .selected(self.settings.launcher_icon.index())
            .build();
        launcher_icon.add_prefix(&launcher_preview);
        {
            let sender = sender.clone();
            let preview = launcher_preview.clone();
            launcher_icon.connect_selected_notify(move |row| {
                let selected = Companion::from_index(row.selected());
                set_launcher_preview(&preview, selected);
                sender.input(AppMsg::SetLauncherIcon(selected));
            });
        }
        jamkin.add(&launcher_icon);
        let desktop_jamkin = adw::SwitchRow::builder()
            .title(crate::i18n::tr("Desktop Jamkin"))
            .subtitle("Drag to move, hover for the current lyric, click to open Lyrics")
            .active(self.settings.desktop_jamkin)
            .build();
        {
            let sender = sender.clone();
            desktop_jamkin.connect_active_notify(move |row| {
                sender.input(AppMsg::SetDesktopJamkin(row.is_active()));
            });
        }
        jamkin.add(&desktop_jamkin);

        let stay_visible = adw::SwitchRow::builder()
            .title(crate::i18n::tr("Keep Jamkin When Window Closes"))
            .subtitle("Leaves the companion visible while music continues in the background")
            .active(self.settings.desktop_jamkin_stay_visible)
            .build();
        {
            let sender = sender.clone();
            stay_visible.connect_active_notify(move |row| {
                sender.input(AppMsg::SetDesktopJamkinStayVisible(row.is_active()));
            });
        }
        jamkin.add(&stay_visible);

        let size_row = adw::ActionRow::builder()
            .title(crate::i18n::tr("Desktop Jamkin Size"))
            .subtitle("Changes the floating companion immediately")
            .build();
        let size = gtk::Scale::with_range(
            gtk::Orientation::Horizontal,
            f64::from(crate::settings::MIN_DESKTOP_JAMKIN_SIZE),
            f64::from(crate::settings::MAX_DESKTOP_JAMKIN_SIZE),
            1.0,
        );
        size.set_value(f64::from(self.settings.desktop_jamkin_size));
        size.set_digits(0);
        size.set_draw_value(true);
        size.set_value_pos(gtk::PositionType::Right);
        size.set_width_request(190);
        size.set_valign(gtk::Align::Center);
        size.set_tooltip_text(Some("Floating companion size in pixels"));
        {
            let sender = sender.clone();
            size.connect_value_changed(move |scale| {
                let pixels = scale.value().round() as u16;
                sender.input(AppMsg::SetDesktopJamkinSize(pixels));
            });
        }
        size_row.add_suffix(&size);
        jamkin.add(&size_row);

        let opacity_row = adw::ActionRow::builder()
            .title(crate::i18n::tr("Desktop Jamkin Opacity"))
            .subtitle("Changes the sprite only; hover lyrics stay fully readable")
            .build();
        let opacity = gtk::Scale::with_range(
            gtk::Orientation::Horizontal,
            f64::from(crate::settings::MIN_DESKTOP_JAMKIN_OPACITY),
            f64::from(crate::settings::MAX_DESKTOP_JAMKIN_OPACITY),
            5.0,
        );
        opacity.set_value(f64::from(self.settings.desktop_jamkin_opacity));
        opacity.set_digits(0);
        opacity.set_draw_value(true);
        opacity.set_value_pos(gtk::PositionType::Right);
        opacity.set_width_request(190);
        opacity.set_valign(gtk::Align::Center);
        opacity.set_tooltip_text(Some("Floating companion opacity as a percentage"));
        {
            let sender = sender.clone();
            opacity.connect_value_changed(move |scale| {
                let percent = ((scale.value() / 5.0).round() * 5.0) as u8;
                sender.input(AppMsg::SetDesktopJamkinOpacity(percent));
            });
        }
        opacity_row.add_suffix(&opacity);
        jamkin.add(&opacity_row);

        let above_supported = JamkinMode::keep_above_supported();
        let keep_above = adw::SwitchRow::builder()
            .title(crate::i18n::tr("Keep Jamkin Above Other Windows"))
            .subtitle(if above_supported {
                "Draws the Desktop Jamkin over maximized windows"
            } else {
                "Unavailable on this desktop; its window menu may offer Keep Above"
            })
            .active(
                (self.settings.desktop_jamkin_above || self.settings.desktop_jamkin_oled_care)
                    && above_supported,
            )
            .sensitive(above_supported)
            .build();
        jamkin.add(&keep_above);

        let edge_walk = adw::SwitchRow::builder()
            .title(crate::i18n::tr("Edge Walk"))
            .subtitle(if above_supported {
                "Periodically walks screen edges and changes corners to reduce static OLED wear"
            } else {
                "Unavailable here because this desktop cannot position an overlay"
            })
            .active(self.settings.desktop_jamkin_oled_care && above_supported)
            .sensitive(above_supported)
            .build();
        {
            let sender = sender.clone();
            let edge_walk = edge_walk.clone();
            keep_above.connect_active_notify(move |row| {
                if !row.is_active() && edge_walk.is_active() {
                    edge_walk.set_active(false);
                }
                sender.input(AppMsg::SetDesktopJamkinAbove(row.is_active()));
            });
        }
        {
            let sender = sender.clone();
            let keep_above = keep_above.clone();
            edge_walk.connect_active_notify(move |row| {
                if row.is_active() && !keep_above.is_active() {
                    keep_above.set_active(true);
                }
                sender.input(AppMsg::SetDesktopJamkinOledCare(row.is_active()));
            });
        }
        jamkin.add(&edge_walk);

        // No group description. It carried a caveat about notifications needing
        // the app to be installed, which is a **developer's** problem — anyone
        // who has Preferences open from a Flatpak or `make install` is already
        // past it, and pointing at the README from inside a settings dialog is
        // not something a preferences pane should do.
        let notifications = adw::PreferencesGroup::builder()
            .title(crate::i18n::tr("Notifications"))
            .build();
        let notify = adw::SwitchRow::builder()
            .title(crate::i18n::tr("Notify on track change"))
            .subtitle("When a new song starts and Jamelade is not in focus")
            .active(self.settings.notify_track_change)
            .build();
        {
            let sender = sender.clone();
            notify.connect_active_notify(move |row| {
                sender.input(AppMsg::SetNotifyTrackChange(row.is_active()));
            });
        }
        notifications.add(&notify);

        let connections = adw::PreferencesGroup::builder()
            .title(crate::i18n::tr("Connections"))
            .description("Connections are optional and off by default")
            .build();
        let discord_available = crate::discord::Presence::available();
        let discord = adw::SwitchRow::builder()
            .title(crate::i18n::tr("Discord Activity"))
            .subtitle(if discord_available {
                "Shares the current title, artist, album and selected Jamkin with the local Discord app"
            } else {
                "Unavailable until this build has Jamelade's public Discord Application ID"
            })
            .active(self.settings.discord_activity && discord_available)
            .sensitive(discord_available)
            .build();
        {
            let sender = sender.clone();
            discord.connect_active_notify(move |row| {
                sender.input(AppMsg::SetDiscordActivity(row.is_active()));
            });
        }
        connections.add(&discord);

        let shortcuts = adw::SwitchRow::builder()
            .title(crate::i18n::tr("Global Shortcuts"))
            .subtitle(
                "Configure play, next, previous and Lyrics through your desktop's secure portal",
            )
            .active(self.settings.global_shortcuts)
            .build();
        {
            let sender = sender.clone();
            shortcuts.connect_active_notify(move |row| {
                sender.input(if row.is_active() {
                    AppMsg::ConfigureGlobalShortcuts
                } else {
                    AppMsg::DisableGlobalShortcuts
                });
            });
        }
        connections.add(&shortcuts);

        let listenbrainz = adw::ActionRow::builder()
            .title(crate::i18n::tr("ListenBrainz Scrobbling"))
            .subtitle(if self.settings.listenbrainz_scrobbling {
                "Enabled; submits completed-listen metadata to listenbrainz.org"
            } else {
                "Optional; sends title, artist, album, duration and start time when enabled"
            })
            .build();
        let listenbrainz_button = gtk::Button::builder()
            .label(if self.settings.listenbrainz_scrobbling {
                crate::i18n::tr("Disable")
            } else {
                crate::i18n::tr("Set up…")
            })
            .valign(gtk::Align::Center)
            .build();
        {
            let sender = sender.clone();
            let enabled = self.settings.listenbrainz_scrobbling;
            listenbrainz_button.connect_clicked(move |_| {
                sender.input(if enabled {
                    AppMsg::DisableListenBrainz
                } else {
                    AppMsg::ShowListenBrainzSetup
                });
            });
        }
        listenbrainz.add_suffix(&listenbrainz_button);
        listenbrainz.set_activatable_widget(Some(&listenbrainz_button));
        connections.add(&listenbrainz);

        let privacy = adw::PreferencesGroup::builder()
            .title(crate::i18n::tr("Lyrics privacy"))
            .description(
                "Apple Music is tried first through your existing session. Third-party fallbacks are separately opt-in, contacted one at a time, and also see your IP address.",
            )
            .build();
        let apple_lyrics = adw::ActionRow::builder()
            .title(crate::i18n::tr("Lyrics from Apple Music"))
            .subtitle(
                "First choice; sends only the playing song's catalog ID to Apple, which already receives playback requests",
            )
            .build();
        apple_lyrics.add_suffix(
            &gtk::Image::builder()
                .icon_name("emblem-ok-symbolic")
                .tooltip_text("Included with the Apple Music connection")
                .build(),
        );
        privacy.add(&apple_lyrics);
        let lyrics = adw::SwitchRow::builder()
            .title(crate::i18n::tr("Fallback lyrics from LRCLIB"))
            .subtitle(
                "When Apple has no match, sends title, artist, album and duration to lrclib.net; never Apple credentials",
            )
            .active(self.settings.lyrics_enabled)
            .build();
        {
            let sender = sender.clone();
            lyrics.connect_active_notify(move |row| {
                sender.input(AppMsg::SetLyricsEnabled(row.is_active()));
            });
        }
        privacy.add(&lyrics);
        let lyrics_ovh = adw::SwitchRow::builder()
            .title(crate::i18n::tr("Fallback lyrics from Lyrics.ovh"))
            .subtitle(
                "Last resort; sends artist and title to its open-source server, which may consult several lyric sites",
            )
            .active(self.settings.lyrics_ovh_enabled)
            .build();
        {
            let sender = sender.clone();
            lyrics_ovh.connect_active_notify(move |row| {
                sender.input(AppMsg::SetLyricsOvhEnabled(row.is_active()));
            });
        }
        privacy.add(&lyrics_ovh);

        page.add(&appearance);
        page.add(&jamkin);
        page.add(&notifications);
        page.add(&connections);
        page.add(&privacy);
        dialog.add(&page);
        dialog.present(Some(parent));
    }
}
