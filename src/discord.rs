// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Optional, local-only Discord Rich Presence.
//!
//! This module never talks to Discord's network. When the user explicitly
//! enables the preference it opens Discord's local Unix socket and sends only
//! the visible song title, artist, album and selected Jamkin. Apple ids,
//! credentials, artwork URLs, playlist names and lyrics never enter this type.

use std::cell::{Cell, RefCell};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::companion::Companion;

const FRAME_MAX: usize = 64 * 1024;
const TEXT_MAX: usize = 128;
const RETRY_AFTER: Duration = Duration::from_secs(8);
/// Re-publish infrequently so a Vesktop/Discord restart cannot leave a dead
/// stream looking connected forever. This is local IPC only and retains the
/// original stable playback timestamps.
const HEARTBEAT_AFTER: Duration = Duration::from_secs(15);
const CONNECT_BUDGET: Duration = Duration::from_secs(2);
const SOCKET_IO_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Activity {
    title: String,
    state: String,
    started_at: Option<u64>,
    ends_at: Option<u64>,
}

enum Message {
    Changed,
    Shutdown,
}

/// Small main-thread handle. The bounded channel prevents either a broken
/// Discord client or a stalled socket from accumulating listening history.
pub struct Presence {
    sender: Option<SyncSender<Message>>,
    worker: Option<JoinHandle<()>>,
    enabled: Cell<bool>,
    /// The privacy switch cannot be dropped merely because the bounded event
    /// queue is full. The worker checks this authoritative flag whenever it
    /// wakes, even if the best-effort wake-up message was coalesced.
    shared_enabled: Arc<AtomicBool>,
    /// Likewise, the channel never owns a listening-history queue. Every wake
    /// reads this single newest value, so a full channel cannot delay a clear
    /// behind older songs.
    shared_activity: Arc<Mutex<Option<Activity>>>,
    current: RefCell<Option<Activity>>,
}

impl Presence {
    pub fn available() -> bool {
        application_id().is_some()
    }

    pub fn new(enabled: bool) -> Self {
        let Some(client_id) = application_id() else {
            return Self {
                sender: None,
                worker: None,
                enabled: Cell::new(false),
                shared_enabled: Arc::new(AtomicBool::new(false)),
                shared_activity: Arc::new(Mutex::new(None)),
                current: RefCell::new(None),
            };
        };

        let (sender, receiver) = mpsc::sync_channel(8);
        let shared_enabled = Arc::new(AtomicBool::new(false));
        let worker_enabled = Arc::clone(&shared_enabled);
        let shared_activity = Arc::new(Mutex::new(None));
        let worker_activity = Arc::clone(&shared_activity);
        let worker = std::thread::Builder::new()
            .name("jamelade-discord".into())
            .spawn(move || worker(receiver, client_id, worker_enabled, worker_activity))
            .ok();
        let sender = worker.as_ref().map(|_| sender);
        let presence = Self {
            sender,
            worker,
            enabled: Cell::new(false),
            shared_enabled,
            shared_activity,
            current: RefCell::new(None),
        };
        presence.set_enabled(enabled);
        presence
    }

    /// Turn disclosure on or off. No socket is even probed while off.
    pub fn set_enabled(&self, enabled: bool) -> bool {
        let enabled = enabled && self.sender.is_some();
        self.shared_enabled.store(enabled, Ordering::Release);
        if self.enabled.replace(enabled) == enabled {
            return enabled;
        }
        if !enabled {
            self.current.borrow_mut().take();
            if let Ok(mut current) = self.shared_activity.lock() {
                current.take();
            }
        }
        self.send(Message::Changed);
        enabled
    }

    /// Diff on the main thread so the playback clock cannot flood the worker.
    pub fn update(
        &self,
        track: Option<(&str, &str, &str)>,
        companion: Companion,
        playing: bool,
        position_ms: u64,
        duration_ms: u64,
    ) {
        if !self.enabled.get() {
            return;
        }
        let display = track.map(|(title, artist, album)| {
            let title = display_text(title);
            let context = match (display_text(artist), display_text(album)) {
                (artist, album) if !artist.is_empty() && !album.is_empty() => {
                    format!("{artist} — {album} · {} is listening", companion.label())
                }
                (artist, _) if !artist.is_empty() => {
                    format!("{artist} · {} is listening", companion.label())
                }
                (_, album) if !album.is_empty() => {
                    format!("{album} · {} is listening", companion.label())
                }
                _ => format!("{} is listening", companion.label()),
            };
            (title, display_text(&context))
        });
        let timed = playing && duration_ms > 0 && position_ms < duration_ms;
        if display.as_ref().is_some_and(|(title, state)| {
            self.current.borrow().as_ref().is_some_and(|current| {
                current.title == *title
                    && current.state == *state
                    && current.started_at.is_some() == timed
            })
        }) {
            // Keep the original stable timestamp rather than recalculating it
            // on every 250ms playback tick.
            return;
        }
        let activity = display.map(|(title, state)| {
            let (started_at, ends_at) = timestamps(playing, position_ms, duration_ms);
            Activity {
                title,
                state,
                started_at,
                ends_at,
            }
        });
        if *self.current.borrow() == activity {
            return;
        }
        *self.current.borrow_mut() = activity.clone();
        if let Ok(mut latest) = self.shared_activity.lock() {
            *latest = activity;
        }
        self.send(Message::Changed);
    }

    fn send(&self, message: Message) {
        let Some(sender) = &self.sender else { return };
        // Track changes are rare and the queue is deliberately bounded. If a
        // wedged Discord client fills it, dropping an update is safer than
        // freezing GTK or retaining an unbounded listening history.
        let _ = sender.try_send(message);
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            // A blocking send is safe here: the worker owns only eight slots
            // and socket operations have short timeouts. It lets Shutdown
            // clear the activity before the process disappears.
            let _ = sender.send(Message::Shutdown);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn application_id() -> Option<String> {
    option_env!("JAMELADE_DISCORD_APPLICATION_ID")
        .filter(|id| valid_application_id(id))
        .map(str::to_owned)
        .or_else(|| {
            std::env::var("JAMELADE_DISCORD_APPLICATION_ID")
                .ok()
                .filter(|id| valid_application_id(id))
        })
}

fn valid_application_id(id: &str) -> bool {
    (5..=32).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_digit())
}

fn display_text(value: &str) -> String {
    let mut shown = String::new();
    for ch in value.trim().chars().filter(|ch| !ch.is_control()) {
        if shown.len() + ch.len_utf8() > TEXT_MAX {
            break;
        }
        shown.push(ch);
    }
    shown
}

fn timestamps(playing: bool, position_ms: u64, duration_ms: u64) -> (Option<u64>, Option<u64>) {
    if !playing || duration_ms == 0 || position_ms >= duration_ms {
        return (None, None);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let started = now.saturating_sub(position_ms / 1_000);
    (
        Some(started),
        Some(started.saturating_add(duration_ms / 1_000)),
    )
}

fn worker(
    receiver: Receiver<Message>,
    client_id: String,
    shared_enabled: Arc<AtomicBool>,
    shared_activity: Arc<Mutex<Option<Activity>>>,
) {
    let mut enabled = false;
    let mut activity: Option<Activity> = None;
    let mut connection: Option<UnixStream> = None;
    loop {
        let message = if enabled {
            let wait = if connection.is_some() {
                HEARTBEAT_AFTER
            } else {
                RETRY_AFTER
            };
            match receiver.recv_timeout(wait) {
                Ok(message) => Some(message),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => Some(Message::Shutdown),
            }
        } else {
            match receiver.recv() {
                Ok(message) => Some(message),
                Err(_) => Some(Message::Shutdown),
            }
        };

        match message {
            Some(Message::Changed) => {}
            Some(Message::Shutdown) => {
                clear(&mut connection);
                return;
            }
            // A Discord-compatible client can replace its socket while this
            // process remains alive. Re-publishing detects the stale stream;
            // `publish` then reconnects immediately without waiting for the
            // next song or leaking any additional metadata.
            None => {}
        }

        // Coalesce everything already queued. Only the newest activity matters
        // and keeping older values would itself be unnecessary history.
        while let Ok(message) = receiver.try_recv() {
            match message {
                Message::Changed => {}
                Message::Shutdown => {
                    clear(&mut connection);
                    return;
                }
            }
        }
        let allowed = shared_enabled.load(Ordering::Acquire);
        if enabled && !allowed {
            clear(&mut connection);
            connection = None;
            activity = None;
        }
        enabled = allowed;
        if !enabled {
            continue;
        }
        if let Ok(latest) = shared_activity.lock()
            && *latest != activity
        {
            activity = latest.clone();
        }

        // Every enabled wake-up is either a coalesced state change or the
        // bounded heartbeat, so there is exactly one publish attempt here.
        publish(&mut connection, &client_id, activity.as_ref());
    }
}

/// Publish the newest single activity. If an existing stream went stale,
/// reconnect once immediately. If no client is running, the empty connection
/// selects the worker's shorter bounded retry timer on its next iteration.
fn publish(connection: &mut Option<UnixStream>, client_id: &str, activity: Option<&Activity>) {
    let _ = publish_with(connection, activity, || connect(client_id));
}

fn publish_with<F>(
    connection: &mut Option<UnixStream>,
    activity: Option<&Activity>,
    mut reconnect: F,
) -> bool
where
    F: FnMut() -> io::Result<UnixStream>,
{
    let retry_stale = connection.is_some();
    if connection.is_none() {
        *connection = reconnect().ok();
    }
    if connection
        .as_mut()
        .is_some_and(|socket| set_activity(socket, activity).is_ok())
    {
        return true;
    }
    *connection = None;

    if !retry_stale {
        return false;
    }
    *connection = reconnect().ok();
    if !connection
        .as_mut()
        .is_some_and(|socket| set_activity(socket, activity).is_ok())
    {
        *connection = None;
        false
    } else {
        true
    }
}

fn socket_candidates() -> Vec<PathBuf> {
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) else {
        return Vec::new();
    };
    if !runtime.is_absolute() || !runtime.is_dir() {
        return Vec::new();
    }
    let mut roots = vec![runtime.clone()];
    roots.extend([
        runtime.join("app/com.discordapp.Discord"),
        runtime.join("app/dev.vencord.Vesktop"),
        runtime.join("app/org.armcord.ArmCord"),
    ]);
    roots
        .into_iter()
        .flat_map(|root| (0..10).map(move |slot| root.join(format!("discord-ipc-{slot}"))))
        .collect()
}

fn connect(client_id: &str) -> io::Result<UnixStream> {
    let started = Instant::now();
    for path in socket_candidates() {
        let Some(remaining) = CONNECT_BUDGET.checked_sub(started.elapsed()) else {
            break;
        };
        if !safe_socket(&path) {
            continue;
        }
        let Ok(mut socket) = UnixStream::connect(&path) else {
            continue;
        };
        let timeout = remaining.min(SOCKET_IO_TIMEOUT);
        socket.set_read_timeout(Some(timeout))?;
        socket.set_write_timeout(Some(timeout))?;
        write_frame(&mut socket, 0, &json!({ "v": 1, "client_id": client_id }))?;
        let (opcode, reply) = read_frame(&mut socket)?;
        if opcode == 1 && reply.get("evt").and_then(Value::as_str) == Some("READY") {
            return Ok(socket);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotConnected,
        "Discord IPC is unavailable",
    ))
}

fn safe_socket(path: &Path) -> bool {
    // A symlink is unnecessary for Discord's documented IPC layout and makes
    // pathname validation ambiguous, so reject it rather than canonicalizing
    // through it. The socket must also belong to the owner of XDG_RUNTIME_DIR.
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() {
        return false;
    }
    let Some(runtime_path) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) else {
        return false;
    };
    let Ok(runtime) = runtime_path.canonicalize() else {
        return false;
    };
    let Ok(runtime_metadata) = fs::metadata(&runtime) else {
        return false;
    };
    // XDG_RUNTIME_DIR and the socket must both belong to this process's user.
    // Merely matching each other is insufficient if the environment was
    // accidentally pointed at another user's runtime directory.
    // SAFETY: `geteuid` has no pointer arguments or preconditions and only
    // reads the kernel's credentials for this process.
    let effective_uid = unsafe { libc::geteuid() };
    if runtime_metadata.uid() != effective_uid || metadata.uid() != runtime_metadata.uid() {
        return false;
    }
    path.canonicalize()
        .is_ok_and(|socket| canonical_socket_is_allowed(&runtime_path, &runtime, path, &socket))
}

fn canonical_socket_is_allowed(
    runtime_path: &Path,
    runtime: &Path,
    visible_socket: &Path,
    canonical_socket: &Path,
) -> bool {
    if canonical_socket.starts_with(runtime) {
        return true;
    }

    // Flatpak exposes an explicitly granted runtime path as a bind mount. Its
    // visible name remains `$XDG_RUNTIME_DIR/...`, while `canonicalize()`
    // deliberately reveals the brokered backing path below `/run/flatpak`.
    // Accept only the exact relative path Flatpak mapped; a different client,
    // slot, or arbitrary location remains rejected. The caller has already
    // established that both the runtime directory and socket belong to this
    // process's effective user and that the final component is a real socket,
    // not a symlink.
    visible_socket
        .strip_prefix(runtime_path)
        .is_ok_and(|relative| canonical_socket == Path::new("/run/flatpak").join(relative))
}

fn set_activity(socket: &mut UnixStream, activity: Option<&Activity>) -> io::Result<()> {
    let activity = activity.map(|activity| {
        let mut value = json!({
            "details": activity.title,
            "state": activity.state,
            "instance": false,
        });
        if let (Some(start), Some(end)) = (activity.started_at, activity.ends_at) {
            value["timestamps"] = json!({ "start": start, "end": end });
        }
        value
    });
    let payload = json!({
        "cmd": "SET_ACTIVITY",
        "args": { "pid": std::process::id(), "activity": activity },
        "nonce": nonce(),
    });
    write_frame(socket, 1, &payload)?;
    // Discord answers every command. Draining and validating that bounded
    // reply prevents a long listening session from filling the socket buffer
    // with one unread response per track.
    let (opcode, reply) = read_frame(socket)?;
    if opcode == 1
        && reply.get("cmd").and_then(Value::as_str) == Some("SET_ACTIVITY")
        && reply.get("evt").and_then(Value::as_str) != Some("ERROR")
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Discord rejected the activity",
        ))
    }
}

fn clear(connection: &mut Option<UnixStream>) {
    if let Some(socket) = connection.as_mut() {
        let _ = set_activity(socket, None);
    }
}

fn nonce() -> String {
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("jamelade-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn write_frame(socket: &mut UnixStream, opcode: u32, value: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(value)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid Discord payload"))?;
    if body.len() > FRAME_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Discord payload is too large",
        ));
    }
    socket.write_all(&opcode.to_le_bytes())?;
    socket.write_all(&(body.len() as u32).to_le_bytes())?;
    socket.write_all(&body)
}

fn read_frame(socket: &mut UnixStream) -> io::Result<(u32, Value)> {
    let mut header = [0_u8; 8];
    socket.read_exact(&mut header)?;
    let opcode = u32::from_le_bytes(header[..4].try_into().unwrap_or_default());
    let length = u32::from_le_bytes(header[4..].try_into().unwrap_or_default()) as usize;
    if length > FRAME_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Discord response is too large",
        ));
    }
    let mut body = vec![0; length];
    socket.read_exact(&mut body)?;
    let value = serde_json::from_slice(&body)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid Discord response"))?;
    Ok((opcode, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_text_drops_controls_and_is_bounded() {
        let text = format!("  hello\n{}  ", "x".repeat(200));
        let shown = display_text(&text);
        assert!(!shown.contains('\n'));
        assert_eq!(shown.len(), TEXT_MAX);
        assert!(display_text(&"🍓".repeat(100)).len() <= TEXT_MAX);
    }

    #[test]
    fn paused_activity_has_no_timestamps() {
        assert_eq!(timestamps(false, 12_000, 40_000), (None, None));
        assert_eq!(timestamps(true, 40_000, 40_000), (None, None));
    }

    #[test]
    fn discord_application_ids_are_public_numeric_identifiers() {
        assert!(valid_application_id("12345"));
        assert!(valid_application_id("12345678901234567890123456789012"));
        assert!(!valid_application_id(""));
        assert!(!valid_application_id("1234"));
        assert!(!valid_application_id("12345-secret"));
        assert!(!valid_application_id("123456789012345678901234567890123"));
    }

    #[test]
    fn canonical_socket_accepts_only_the_exact_flatpak_runtime_bridge() {
        let runtime = Path::new("/run/user/1000");
        let visible = runtime.join("app/com.discordapp.Discord/discord-ipc-0");
        let vesktop = runtime.join("app/dev.vencord.Vesktop/discord-ipc-0");

        assert!(canonical_socket_is_allowed(
            runtime,
            runtime,
            &visible,
            Path::new("/run/flatpak/app/com.discordapp.Discord/discord-ipc-0"),
        ));
        assert!(canonical_socket_is_allowed(
            runtime,
            runtime,
            &runtime.join("discord-ipc-0"),
            Path::new("/run/flatpak/discord-ipc-0"),
        ));
        assert!(canonical_socket_is_allowed(
            runtime,
            runtime,
            &vesktop,
            Path::new("/run/flatpak/app/dev.vencord.Vesktop/discord-ipc-0"),
        ));
        assert!(!canonical_socket_is_allowed(
            runtime,
            runtime,
            &visible,
            Path::new("/run/flatpak/app/dev.vencord.Vesktop/discord-ipc-0"),
        ));
        assert!(!canonical_socket_is_allowed(
            runtime,
            runtime,
            &visible,
            Path::new("/tmp/discord-ipc-0"),
        ));
    }

    #[test]
    fn oversized_outgoing_frames_are_refused_before_writing() {
        let (mut client, _server) = UnixStream::pair().unwrap();
        let error =
            write_frame(&mut client, 1, &json!({ "x": "x".repeat(FRAME_MAX) })).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_stale_stream_reconnects_and_republishes_without_a_track_change() {
        let (stale, stale_peer) = UnixStream::pair().unwrap();
        drop(stale_peer);

        let (replacement, mut replacement_peer) = UnixStream::pair().unwrap();
        replacement
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let server = std::thread::spawn(move || {
            let (opcode, request) = read_frame(&mut replacement_peer).unwrap();
            assert_eq!(opcode, 1);
            assert_eq!(request["cmd"], "SET_ACTIVITY");
            write_frame(
                &mut replacement_peer,
                1,
                &json!({ "cmd": "SET_ACTIVITY", "data": {} }),
            )
            .unwrap();
        });

        let activity = Activity {
            title: "A Song".into(),
            state: "An Artist · JamBun is listening".into(),
            started_at: None,
            ends_at: None,
        };
        let mut connection = Some(stale);
        let mut replacement = Some(replacement);
        assert!(publish_with(&mut connection, Some(&activity), || {
            replacement.take().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotConnected, "replacement already used")
            })
        }));
        assert!(connection.is_some());
        server.join().unwrap();
    }

    #[test]
    fn activity_frame_contains_only_the_disclosed_display_fields() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let peer = std::thread::spawn(move || {
            let (opcode, request) = read_frame(&mut server).unwrap();
            assert_eq!(opcode, 1);
            assert_eq!(request["cmd"], "SET_ACTIVITY");
            let activity = &request["args"]["activity"];
            assert_eq!(activity["details"], "A Song");
            assert_eq!(
                activity["state"],
                "An Artist — An Album · JamPam is listening"
            );
            let serialized = serde_json::to_string(&request).unwrap();
            for forbidden in [
                "token",
                "cookie",
                "lyrics",
                "playlist",
                "artwork",
                "catalog_id",
                "library_id",
            ] {
                assert!(!serialized.to_ascii_lowercase().contains(forbidden));
            }
            write_frame(
                &mut server,
                1,
                &json!({ "cmd": "SET_ACTIVITY", "evt": null, "data": {} }),
            )
            .unwrap();
        });
        let activity = Activity {
            title: "A Song".into(),
            state: "An Artist — An Album · JamPam is listening".into(),
            started_at: Some(10),
            ends_at: Some(20),
        };
        set_activity(&mut client, Some(&activity)).unwrap();
        peer.join().unwrap();
    }
}
