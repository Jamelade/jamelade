// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! User-confirmed launcher-icon replacement through the desktop portal.
//!
//! A Flatpak cannot safely rewrite its exported `.desktop` file. Granting it
//! `xdg-data/applications` would also expose other launchers, so Jamelade ships
//! one stable sub-launcher and asks the Dynamic Launcher portal to replace only
//! that entry. The desktop owns the confirmation and the resulting host file.

use anyhow::{Context, Result, anyhow};
use ashpd::desktop::Icon;
use ashpd::desktop::dynamic_launcher::{DynamicLauncherProxy, PrepareInstallOptions};

use crate::companion::Companion;

pub const DESKTOP_FILE_ID: &str = "io.github.Jamelade.Jamelade.Launcher.desktop";
pub const PREFERENCE_HELP: &str = "Choose a Jamkin, then approve the desktop dialog. KDE calls \
    it \"Add Application\"; it updates Jamelade rather than installing another copy. Restart \
    Jamelade afterward.";
pub const CONFIRM_HELP: &str =
    "Choose “Add Application” in the desktop dialog; it updates Jamelade";
pub const CHANGED_HELP: &str = "Icon changed. Restart Jamelade to update the app menu; re-pin it \
    if the dock keeps the old icon.";

const DESKTOP_ENTRY: &str = "[Desktop Entry]\n\
Type=Application\n\
GenericName=Music Player\n\
Comment=Play your Apple Music library natively on Linux\n\
Exec=jamelade\n\
Terminal=false\n\
Categories=AudioVideo;Audio;Player;\n\
Keywords=Music;Apple;Player;Audio;Playlist;Streaming;\n\
StartupNotify=true\n\
StartupWMClass=io.github.Jamelade.Jamelade.Launcher\n";
const MAX_LAUNCHER_ICON_BYTES: usize = 10 * 1024 * 1024;

/// Ask the desktop to install the selected rounded tile. No Apple data,
/// listening history or network request is involved; only the bundled PNG and
/// this fixed launcher template cross the portal boundary.
pub async fn install(companion: Companion) -> Result<()> {
    let path = companion
        .launcher_icon_path()
        .ok_or_else(|| anyhow!("{} launcher artwork is missing", companion.label()))?;
    let bytes = tokio::task::spawn_blocking(move || {
        crate::private_storage::read_bytes(&path, MAX_LAUNCHER_ICON_BYTES)
    })
    .await
    .context("launcher artwork task stopped")?
    .context("could not read launcher artwork")?;

    let proxy = DynamicLauncherProxy::new()
        .await
        .context("the desktop launcher portal is unavailable")?;
    let request = proxy
        .prepare_install(
            None,
            crate::APP_NAME,
            Icon::Bytes(bytes),
            PrepareInstallOptions::default()
                .set_modal(true)
                .set_editable_name(false)
                .set_editable_icon(false),
        )
        .await
        .context("the desktop could not open its launcher confirmation")?;
    let response = request
        .response()
        .context("launcher change was not confirmed")?;
    proxy
        .install(
            response.token(),
            DESKTOP_FILE_ID,
            DESKTOP_ENTRY,
            Default::default(),
        )
        .await
        .context("the desktop could not install the selected launcher")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_template_cannot_override_its_confirmed_name_or_icon() {
        assert!(!DESKTOP_ENTRY.lines().any(|line| line.starts_with("Name=")));
        assert!(!DESKTOP_ENTRY.lines().any(|line| line.starts_with("Icon=")));
        assert!(DESKTOP_ENTRY.contains("Exec=jamelade"));
        assert!(DESKTOP_ENTRY.contains(crate::LAUNCHER_ID));
    }

    #[test]
    fn launcher_is_a_valid_sub_id_of_the_flatpak() {
        assert!(DESKTOP_FILE_ID.starts_with(&format!("{}.", crate::APP_ID)));
        assert!(DESKTOP_FILE_ID.ends_with(".desktop"));
    }

    #[test]
    fn icon_help_explains_the_desktop_confirmation() {
        assert!(PREFERENCE_HELP.contains("Add Application"));
        assert!(PREFERENCE_HELP.contains("rather than installing another copy"));
        assert!(CHANGED_HELP.contains("Restart Jamelade"));
    }
}
