// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Files which describe a person's library or listening should not inherit a
//! permissive umask. Keep app-owned directories user-only and data files at
//! mode 0600, including files created by an earlier version.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
pub fn ensure_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    fs::create_dir_all(path)?;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let dir = options.open(path)?;
    if !dir.metadata()?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private data directory is not a directory",
        ));
    }
    // Apply the mode to the descriptor we validated, not to a pathname that
    // could be swapped for a symlink between checking and chmod.
    dir.set_permissions(fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
pub fn ensure_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

/// Read an app-owned file without following a final symlink or trusting its
/// on-disk length.
pub fn read_bytes(path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // App-owned state must be the file we intended to open, not a symlink
        // planted between a metadata check and `open()`. O_CLOEXEC also keeps
        // private state out of the Electron child's descriptor table.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }

    let file = options.open(path)?;
    // Inspect the opened inode, not the path. This closes the ordinary
    // metadata/open race even on filesystems where a directory entry changes
    // underneath us.
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > maximum as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private data file exceeded its safe size",
        ));
    }

    let mut bytes = Vec::with_capacity((metadata.len() as usize).min(maximum));
    file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private data file exceeded its safe size",
        ));
    }
    Ok(bytes)
}

/// Read an app-owned UTF-8 file without trusting its on-disk length.
pub fn read_to_string(path: &Path, maximum: usize) -> io::Result<String> {
    String::from_utf8(read_bytes(path, maximum)?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "private data file is not UTF-8"))
}

fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("private-data");
    let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    path.with_file_name(format!(
        ".{name}.{}.{}.{now}.tmp",
        std::process::id(),
        nonce
    ))
}

pub fn write(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }

    // Write a new inode and atomically replace the destination. Besides
    // avoiding truncated caches after a crash, this refuses to follow a
    // pre-existing symlink at the destination into some unrelated user file.
    let temporary = temporary_path(path);
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut created = false;
    let result = (|| {
        let mut file = options.open(&temporary)?;
        created = true;
        file.write_all(contents.as_ref())?;
        file.sync_all()?;
        drop(file);
        // The new inode was created as 0600. Renaming preserves that mode and
        // avoids a second pathname operation after the atomic replacement.
        fs::rename(&temporary, path)
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn private_writes_tighten_both_new_and_existing_paths() {
        let unique = format!(
            "slipmat-private-storage-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        let file = dir.join("library.json");

        write(&file, b"first").unwrap();
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        write(&file, b"second").unwrap();
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn private_writes_replace_a_symlink_instead_of_following_it() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!(
            "jamelade-private-symlink-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("unrelated.txt");
        let private = dir.join("settings.ini");
        fs::write(&target, b"keep me").unwrap();
        symlink(&target, &private).unwrap();

        write(&private, b"private").unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"keep me");
        assert_eq!(fs::read(&private).unwrap(), b"private");
        assert!(
            !fs::symlink_metadata(&private)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn private_reads_have_a_hard_size_limit() {
        let dir = std::env::temp_dir().join(format!(
            "jamelade-private-read-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("session.json");
        fs::write(&file, b"12345").unwrap();
        assert_eq!(read_to_string(&file, 5).unwrap(), "12345");
        assert!(read_to_string(&file, 4).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn private_reads_refuse_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!(
            "jamelade-private-read-symlink-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("unrelated.txt");
        let private = dir.join("settings.ini");
        fs::write(&target, b"do not read through me").unwrap();
        symlink(&target, &private).unwrap();

        assert!(read_to_string(&private, 1024).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn private_directories_refuse_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!(
            "jamelade-private-dir-symlink-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("real-directory");
        let private = dir.join("jamelade");
        fs::create_dir(&target).unwrap();
        symlink(&target, &private).unwrap();

        assert!(ensure_dir(&private).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
