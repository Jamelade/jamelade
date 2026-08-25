// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! User-approved Wayland/global shortcuts through XDG Desktop Portal.
//!
//! Jamelade never grabs keys itself. The desktop chooses and stores bindings;
//! this process receives only one of four fixed action identifiers.

use futures_util::StreamExt;
use relm4::ComponentSender;

use super::{AppModel, CommandMsg};

pub(super) fn start(sender: &ComponentSender<AppModel>) -> tokio::sync::watch::Sender<bool> {
    let (stop, mut stopped) = tokio::sync::watch::channel(false);
    sender.command(move |out, shutdown| {
        shutdown
            .register(async move {
                use ashpd::desktop::CreateSessionOptions;
                use ashpd::desktop::global_shortcuts::{
                    BindShortcutsOptions, GlobalShortcuts, NewShortcut,
                };

                let ready = async {
                    let portal = GlobalShortcuts::new().await?;
                    let activated = portal.receive_activated().await?;
                    let session = portal
                        .create_session(CreateSessionOptions::default())
                        .await?;
                    let shortcuts = [
                        NewShortcut::new("play-pause", "Play or pause Jamelade"),
                        NewShortcut::new("next", "Next Jamelade track"),
                        NewShortcut::new("previous", "Previous Jamelade track"),
                        NewShortcut::new("lyrics", "Show Jamelade lyrics"),
                    ];
                    let request = portal
                        .bind_shortcuts(&session, &shortcuts, None, BindShortcutsOptions::default())
                        .await?;
                    let _ = request.response()?;
                    Ok::<_, ashpd::Error>((portal, session, activated))
                };

                // Turning the preference off must also dismiss an in-flight
                // portal setup request instead of leaving a desktop prompt
                // alive until the user answers it.
                let result = tokio::select! {
                    result = ready => Some(result),
                    changed = stopped.changed() => {
                        if changed.is_err() || *stopped.borrow() {
                            None
                        } else {
                            return;
                        }
                    }
                };
                let Some(result) = result else {
                    return;
                };

                let (_portal, _session, mut activated) = match result {
                    Ok(ready) => ready,
                    Err(error) => {
                        let _ = out.send(CommandMsg::GlobalShortcutsReady(Err(error.to_string())));
                        return;
                    }
                };
                if out.send(CommandMsg::GlobalShortcutsReady(Ok(()))).is_err() {
                    return;
                }
                loop {
                    tokio::select! {
                        signal = activated.next() => {
                            let Some(signal) = signal else { break };
                            let id = signal.shortcut_id();
                            if matches!(id, "play-pause" | "next" | "previous" | "lyrics")
                                && out.send(CommandMsg::GlobalShortcut(id.to_owned())).is_err()
                            {
                                break;
                            }
                        }
                        changed = stopped.changed() => {
                            if changed.is_err() || *stopped.borrow() {
                                break;
                            }
                        }
                    }
                }
            })
            .drop_on_shutdown()
    });
    stop
}
