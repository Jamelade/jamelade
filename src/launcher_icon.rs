// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Launcher-icon replacement through a narrow host helper or desktop portal.
//!
//! A Flatpak cannot safely rewrite its exported `.desktop` file. The optional
//! same-user helper exposes only `SetIcon(JamBun|JamPam|JamJoe)` and can touch
//! only Jamelade's stable launcher. When it is absent, the desktop-controlled
//! Dynamic Launcher portal remains the no-extra-component fallback.

use anyhow::{Context, Result, anyhow};
use ashpd::desktop::Icon;
use ashpd::desktop::dynamic_launcher::{DynamicLauncherProxy, PrepareInstallOptions};

use crate::companion::Companion;

pub const DESKTOP_FILE_ID: &str = "io.github.Jamelade.Jamelade.Launcher.desktop";
pub const PREFERENCE_HELP: &str = "With the optional icon helper, the existing Jamelade launcher \
    updates directly. Without it, approve KDE's “Add Application” dialog, restart Jamelade, and \
    re-pin the launcher if the dock keeps its old icon.";
pub const CONFIRM_HELP: &str =
    "Changing the launcher icon… approve the desktop dialog if one appears";
pub const HELPER_CHANGED_HELP: &str =
    "Icon changed. The app menu and dock may take a moment to refresh.";
pub const PORTAL_CHANGED_HELP: &str = "Icon changed. Restart Jamelade to update the app menu; \
    re-pin it if the dock keeps the old icon.";

const HELPER_BUS: &str = "io.github.Jamelade.IconHelper";
const HELPER_PATH: &str = "/io/github/Jamelade/IconHelper";
const HELPER_INTERFACE: &str = "io.github.Jamelade.IconHelper";

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    Helper,
    Portal,
}

pub async fn install(companion: Companion) -> Result<InstallMethod> {
    if install_with_helper(companion).await.is_ok() {
        return Ok(InstallMethod::Helper);
    }
    install_with_portal(companion).await?;
    Ok(InstallMethod::Portal)
}

async fn install_with_helper(companion: Companion) -> Result<()> {
    let connection = ashpd::zbus::Connection::session()
        .await
        .context("the session bus is unavailable")?;
    let proxy = ashpd::zbus::Proxy::new(&connection, HELPER_BUS, HELPER_PATH, HELPER_INTERFACE)
        .await
        .context("the icon helper is unavailable")?;
    let _: () = proxy
        .call("SetIcon", &(companion.label(),))
        .await
        .context("the icon helper rejected the change")?;
    Ok(())
}

async fn install_with_portal(companion: Companion) -> Result<()> {
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
        assert!(PREFERENCE_HELP.contains("restart Jamelade"));
        assert!(PREFERENCE_HELP.contains("re-pin"));
        assert!(PREFERENCE_HELP.contains("optional icon helper"));
        assert!(PORTAL_CHANGED_HELP.contains("Restart Jamelade"));
        assert!(!HELPER_CHANGED_HELP.contains("Restart"));
    }
}
