// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Narrow host-side launcher writer for the sandboxed Jamelade Flatpak.
//!
//! Its D-Bus surface accepts exactly one of three Jamkin names. It has no
//! network client, accepts no paths or image bytes, and can replace only
//! Jamelade's stable launcher entry.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use zbus::{blocking::Connection, interface};

const BUS_NAME: &str = "io.github.Jamelade.IconHelper";
const OBJECT_PATH: &str = "/io/github/Jamelade/IconHelper";
const LAUNCHER: &str = "io.github.Jamelade.Jamelade.Launcher.desktop";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Jamkin {
    JamBun,
    JamPam,
    JamJoe,
}

impl Jamkin {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "JamBun" => Some(Self::JamBun),
            "JamPam" => Some(Self::JamPam),
            "JamJoe" => Some(Self::JamJoe),
            _ => None,
        }
    }

    fn icon_name(self) -> &'static str {
        match self {
            Self::JamBun => "io.github.Jamelade.Jamelade.jambun",
            Self::JamPam => "io.github.Jamelade.Jamelade.jampam",
            Self::JamJoe => "io.github.Jamelade.Jamelade.jamjoe",
        }
    }
}

struct IconHelper {
    finished: mpsc::Sender<()>,
}

#[interface(name = "io.github.Jamelade.IconHelper")]
impl IconHelper {
    fn set_icon(&self, name: &str) -> zbus::fdo::Result<()> {
        let jamkin = Jamkin::parse(name)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("unknown Jamkin".into()))?;
        install_launcher(jamkin)
            .map_err(|_| zbus::fdo::Error::Failed("could not update Jamelade's launcher".into()))?;
        let _ = self.finished.send(());
        Ok(())
    }
}

fn data_home() -> io::Result<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(value);
        return path
            .is_absolute()
            .then_some(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "XDG_DATA_HOME is relative"));
    }
    let home = PathBuf::from(
        std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is unavailable"))?,
    );
    home.is_absolute()
        .then(|| home.join(".local/share"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "HOME is relative"))
}

fn desktop_entry(jamkin: Jamkin) -> String {
    format!(
        "[Desktop Entry]\n\
Type=Application\n\
Name=Jamelade\n\
GenericName=Music Player\n\
Comment=Play your Apple Music library natively on Linux\n\
Exec=flatpak run io.github.Jamelade.Jamelade\n\
Icon={}\n\
Terminal=false\n\
Categories=AudioVideo;Audio;Player;\n\
Keywords=Music;Apple;Player;Audio;Playlist;Streaming;\n\
StartupNotify=true\n\
StartupWMClass=io.github.Jamelade.Jamelade.Launcher\n",
        jamkin.icon_name()
    )
}

fn install_launcher(jamkin: Jamkin) -> io::Result<()> {
    let applications = data_home()?.join("applications");
    fs::create_dir_all(&applications)?;
    atomic_write(
        &applications.join(LAUNCHER),
        desktop_entry(jamkin).as_bytes(),
    )
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "launcher has no parent"))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid launcher name"))?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::set_permissions(&temporary, std::os::unix::fs::PermissionsExt::from_mode(0o644))?;
        fs::rename(&temporary, path)?;
        fs::File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new("--service"))
        || std::env::args_os().nth(2).is_some()
    {
        return Err("this program is started through D-Bus activation".into());
    }

    let connection = Connection::session()?;
    let (finished, wait) = mpsc::channel();
    connection
        .object_server()
        .at(OBJECT_PATH, IconHelper { finished })?;
    connection.request_name(BUS_NAME)?;

    // Exit after one request. A short grace lets zbus flush the method reply;
    // an activation with no call also cannot leave a resident helper behind.
    if wait.recv_timeout(Duration::from_secs(30)).is_ok() {
        std::thread::sleep(Duration::from_millis(150));
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Jamelade icon helper: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_three_public_jamkin_names_are_accepted() {
        for name in ["JamBun", "JamPam", "JamJoe"] {
            assert!(Jamkin::parse(name).is_some());
        }
        for name in ["jambun", "Jamila", "../JamJoe", "", "JamBun\nIcon=evil"] {
            assert!(Jamkin::parse(name).is_none());
        }
    }

    #[test]
    fn launcher_has_fixed_execution_and_only_the_icon_varies() {
        let entry = desktop_entry(Jamkin::JamJoe);
        assert!(entry.contains("Exec=flatpak run io.github.Jamelade.Jamelade\n"));
        assert!(entry.contains("Icon=io.github.Jamelade.Jamelade.jamjoe\n"));
        assert_eq!(entry.matches("Exec=").count(), 1);
        assert_eq!(entry.matches("Icon=").count(), 1);
    }
}
