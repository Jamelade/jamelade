// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later

//! Locate, spawn and supervise the Electron child.
//!
//! This is the piece Dockyard and Pitwall never needed. Both talked to
//! something that already existed (a socket, an API); Slipmat *owns a process*.
//! So the module's job is lifetime: start it, read its stdout forever, notice
//! when it dies, and let `app/mod.rs` restart it (ARCHITECTURE.md rule 6).
//!
//! ## Ownership note (Rust, for a React brain)
//!
//! The child's stdin and stdout are two halves that need to live in different
//! places: stdout is read by a background task that runs for as long as the
//! process does, while stdin is written to from `update()` in response to
//! clicks. We can't hand both to one owner without making every send `await` a
//! lock, so we split them:
//!
//!   - stdout is *moved* into a spawned tokio task that owns it outright;
//!   - stdin is *moved* into a second task fed by a bounded channel.
//!
//! `Handle` then holds only a channel sender, which is cheap to clone and
//! `Send` — that's why the UI can keep one in the model and clone it into
//! closures without a `Mutex` anywhere. The channel *is* the synchronisation.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, anyhow};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};

use super::protocol::{ApiMethod, ApiResponse, Command, Event};

/// What the reader task pushes up to `app/mod.rs`.
#[derive(Debug)]
pub enum Incoming {
    Event(Event),
    /// A line we couldn't parse. Kept as a distinct case rather than silently
    /// dropped — it usually means preload.js and protocol.rs drifted.
    Unparsed,
    /// The process exited. Always the last message on the channel.
    Died(String),
}

/// The most commands of one kind Slipmat will send in a second.
///
/// A ceiling, not a budget, and deliberately generous.
///
/// The number has to sit above anything a person can produce and far below what
/// hurts. The upper bound on human input is the pointer's event rate: GTK emits
/// `value-changed` per motion event, so dragging a slider on a 165Hz display
/// can plausibly reach a couple of hundred a second. A ceiling that clipped a
/// real drag would be worse than the bug it guards against — so this is set
/// clear of that, not snug against it.
///
/// The failure it catches is nothing like a drag: a runaway `update()` managed
/// **5,721 dispatches** before the desktop stopped responding (#37), and reached
/// this ceiling in the first fraction of a second.
const MAX_PER_SECOND: u32 = 250;
/// Backpressure between the GTK model, pipe tasks and the remote page.
/// Generous for ordinary bursts, finite so a compromised renderer or an
/// accidental loop cannot turn RAM into an implicit event queue.
const EVENT_CHANNEL_CAPACITY: usize = 64;
const COMMAND_CHANNEL_CAPACITY: usize = 256;
const MAX_EVENT_LINE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DIAGNOSTIC_LINE_BYTES: usize = 16 * 1024;
const MAX_BROKER_PENDING: usize = 64;
const MAX_BROKER_PATH_BYTES: usize = 4 * 1024;
const BROKER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(35);

type PendingBroker = std::collections::HashMap<u64, oneshot::Sender<ApiReply>>;

/// A bounded, credential-free Apple Music response returned by Chromium.
/// Debug output deliberately contains no response data.
pub struct ApiReply {
    pub status: u16,
    pub body: String,
}

impl std::fmt::Debug for ApiReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiReply")
            .field("status", &self.status)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    #[error("invalid Apple Music broker request")]
    InvalidRequest,
    #[error("Apple Music browser broker is busy")]
    Busy,
    #[error("Apple Music browser broker is unavailable")]
    Unavailable,
    #[error("Apple Music browser request timed out")]
    Timeout,
}

/// Cloneable request half of the sidecar. It accepts only a relative path and
/// one of two typed methods; origins, credentials, headers and bodies remain
/// entirely inside Chromium.
#[derive(Clone)]
pub struct Broker {
    tx: mpsc::Sender<Command>,
    pending: std::sync::Arc<AsyncMutex<PendingBroker>>,
    next_id: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl std::fmt::Debug for Broker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Broker").finish_non_exhaustive()
    }
}

fn valid_broker_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_BROKER_PATH_BYTES
        && path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains("://")
        && !path.contains('\\')
        && !path.contains('#')
        && !path.chars().any(char::is_control)
}

impl Broker {
    pub async fn request(&self, method: ApiMethod, path: String) -> Result<ApiReply, BrokerError> {
        if !valid_broker_path(&path) {
            return Err(BrokerError::InvalidRequest);
        }

        let request_id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if request_id == 0 {
            return Err(BrokerError::Unavailable);
        }

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            if pending.len() >= MAX_BROKER_PENDING {
                return Err(BrokerError::Busy);
            }
            pending.insert(request_id, tx);
        }

        let command = Command::ApiRequest {
            request_id,
            method,
            path,
        };
        if self.tx.try_send(command).is_err() {
            self.pending.lock().await.remove(&request_id);
            return Err(BrokerError::Unavailable);
        }

        match tokio::time::timeout(BROKER_TIMEOUT, rx).await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(_)) => Err(BrokerError::Unavailable),
            Err(_) => {
                self.pending.lock().await.remove(&request_id);
                Err(BrokerError::Timeout)
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BoundedLine {
    Data(Vec<u8>),
    TooLong,
}

/// Read one line without allowing a child process to choose our allocation.
/// Tokio's convenient `lines()` grows until a newline, which makes a damaged
/// or compromised sidecar an unbounded-memory input even though valid JSON is
/// capped at the Electron boundary.
async fn read_bounded_line<R>(
    reader: &mut R,
    maximum: usize,
) -> std::io::Result<Option<BoundedLine>>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    let mut too_long = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if line.is_empty() && !too_long {
                Ok(None)
            } else if too_long {
                Ok(Some(BoundedLine::TooLong))
            } else {
                Ok(Some(BoundedLine::Data(line)))
            };
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let data_len = newline.unwrap_or(available.len());
        if !too_long {
            let room = maximum.saturating_sub(line.len());
            let copy = data_len.min(room);
            line.extend_from_slice(&available[..copy]);
            too_long = data_len > room;
        }
        let consumed = data_len + usize::from(newline.is_some());
        reader.consume(consumed);

        if newline.is_some() {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(Some(if too_long {
                BoundedLine::TooLong
            } else {
                BoundedLine::Data(line)
            }));
        }
    }
}

/// A cheap, cloneable handle for sending commands to the sidecar.
#[derive(Debug, Clone)]
pub struct Handle {
    tx: mpsc::Sender<Command>,
    broker: Broker,
    /// Per-command-kind rate window, shared by every clone of this handle.
    ///
    /// `Arc<Mutex<_>>` rather than `Rc<RefCell<_>>`: the handle is delivered to
    /// the model through `CommandMsg::Spawned`, which crosses a thread, so it
    /// has to be `Send`. Sends themselves all happen on the GTK thread, so the
    /// lock is uncontended in practice.
    rate: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<&'static str, Window>>>,
    /// One warning per saturation episode, not one per dropped command.
    queue_warned: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Debug)]
struct Window {
    started: std::time::Instant,
    sent: u32,
    /// So a storm says so once rather than once per dropped command — the
    /// logging *is* the amplifier this exists to stop.
    warned: bool,
}

impl Handle {
    pub fn broker(&self) -> Broker {
        self.broker.clone()
    }

    /// Fire-and-forget, up to a ceiling.
    ///
    /// A closed channel means the child already died; the `Died` message is
    /// already on its way, so dropping there is correct rather than an error the
    /// UI has to handle twice.
    ///
    /// The ceiling is the other half. Rule 6 says a dead sidecar must not
    /// present as a healthy player; this is the same argument pointed the other
    /// way — **a runaway client must not be able to take the session with it.**
    /// A two-way binding on the volume button once cycled at a few thousand
    /// commands, each one an NDJSON write, a journald record and a D-Bus
    /// property change, and the machine had to be power-cycled. Nothing between
    /// `update()` and the desktop pushed back.
    pub fn send(&self, cmd: Command) {
        if !self.allow(cmd.name()) {
            return;
        }
        match self.tx.try_send(cmd) {
            Ok(()) => self
                .queue_warned
                .store(false, std::sync::atomic::Ordering::Relaxed),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::debug!("sidecar command dropped: channel closed (child is gone)");
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                if !self
                    .queue_warned
                    .swap(true, std::sync::atomic::Ordering::Relaxed)
                {
                    tracing::warn!(
                        capacity = COMMAND_CHANNEL_CAPACITY,
                        "sidecar command queue full; dropping command"
                    );
                }
            }
        }
    }

    /// Whether this command is within the ceiling for its kind.
    fn allow(&self, name: &'static str) -> bool {
        let now = std::time::Instant::now();
        // A poisoned lock would mean a panic while holding it — nothing here
        // panics, and refusing to send commands because of it would be worse
        // than the storm.
        let Ok(mut rate) = self.rate.lock() else {
            return true;
        };
        let window = rate.entry(name).or_insert(Window {
            started: now,
            sent: 0,
            warned: false,
        });

        if now.duration_since(window.started) >= std::time::Duration::from_secs(1) {
            window.started = now;
            window.sent = 0;
            window.warned = false;
        }

        window.sent += 1;
        if window.sent <= MAX_PER_SECOND {
            return true;
        }

        if !window.warned {
            window.warned = true;
            tracing::error!(
                cmd = name,
                ceiling = MAX_PER_SECOND,
                "command storm: dropping the rest of this second — something is looping"
            );
        }
        false
    }
}

/// Find the sidecar directory: an explicit override, the per-user install, the
/// system install, then the dev tree.
/// Say so when an installed sidecar is about to shadow the one beside the code.
///
/// **This is the trap ARCHITECTURE.md warns about, made audible.** `locate` prefers
/// an installed sidecar over the build tree, so once anything has been
/// installed, `cargo run` runs fresh Rust against stale JavaScript and says
/// nothing. It fails in the most misleading way available: the command goes
/// out, the optimistic UI updates, and only MusicKit disagrees.
///
/// It cost an afternoon on `removeFromLibrary` and then a whole test round on
/// `moveInQueue` — fourteen `unknown-command` errors read as a broken feature
/// rather than a stale file. A build has a `sidecar/` next to its manifest and
/// an installed copy does not, so the two are distinguishable, and the line
/// costs nothing on a real install because the check cannot fire there.
#[cfg(debug_assertions)]
fn warn_if_shadowing_a_build_tree(_chosen: &Path) {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sidecar");
    if dev.join("main.js").is_file() {
        tracing::warn!(
            "an installed sidecar is shadowing this build tree — \
             run with JAMELADE_SIDECAR=$PWD/sidecar, or `make install-sidecar`"
        );
    }
}

#[cfg(not(debug_assertions))]
fn warn_if_shadowing_a_build_tree(_chosen: &Path) {}

pub fn locate() -> Result<PathBuf> {
    let mut tried = Vec::new();

    if let Ok(dir) = std::env::var("JAMELADE_SIDECAR") {
        let p = PathBuf::from(dir);
        if p.join("main.js").is_file() {
            return Ok(p);
        }
        tried.push(p);
    }

    // Per-user first, then system-wide. `XDG_DATA_DIRS` is what makes a
    // packaged install work at all: `make install` puts the sidecar under
    // `~/.local/share`, but a distribution package puts it in
    // `/usr/share/jamelade/sidecar`, which nothing here used to look at.
    for data in dirs_data_home().into_iter().chain(dirs_data_dirs()) {
        let p = data.join("jamelade/sidecar");
        if p.join("main.js").is_file() {
            warn_if_shadowing_a_build_tree(&p);
            return Ok(p);
        }
        tried.push(p);
    }

    // The dev tree, and only in a dev build: `CARGO_MANIFEST_DIR` is where the
    // binary was compiled, which for a package is a build root that will not
    // exist on the machine running it. Baking it into a release binary is both
    // useless and what makes `makepkg` warn about a reference to `$srcdir`.
    #[cfg(debug_assertions)]
    {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sidecar");
        if dev.join("main.js").is_file() {
            return Ok(dev);
        }
        tried.push(dev);
    }

    let _ = tried;
    Err(anyhow!(
        "sidecar not found. Reinstall Jamelade (developers: run `make sidecar`)."
    ))
}

/// `$XDG_DATA_HOME`, else `~/.local/share`. Small enough not to warrant a crate.
/// The system data directories, in preference order. Defaults to the values the
/// XDG spec mandates when the variable is unset, which is the common case.
fn dirs_data_dirs() -> Vec<PathBuf> {
    let raw = match std::env::var("XDG_DATA_DIRS") {
        Ok(x) if !x.is_empty() => x,
        _ => "/usr/local/share:/usr/share".to_owned(),
    };
    raw.split(':')
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn dirs_data_home() -> Option<PathBuf> {
    if let Ok(x) = std::env::var("XDG_DATA_HOME")
        && !x.is_empty()
    {
        return Some(PathBuf::from(x));
    }
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".local/share"))
}

/// The Electron binary inside the sidecar's `node_modules`.
///
/// We deliberately use `electron/dist/electron` rather than `.bin/electron`:
/// the latter is a Node shim, which adds a process between us and the child and
/// can print to stdout — and stdout is protocol (ARCHITECTURE.md).
fn electron_binary(sidecar: &Path) -> Result<PathBuf> {
    let direct = sidecar.join("node_modules/electron/dist/electron");
    if direct.is_file() {
        return Ok(direct);
    }
    Err(anyhow!(
        "Electron is not installed in Jamelade's sidecar. Reinstall Jamelade \
         (developers: run `make sidecar`)."
    ))
}

/// Give Electron an app-specific root before its own JavaScript runs.
///
/// `app.setPath()` in main.js remains the second line of defence, but Electron
/// chooses its default profile before evaluating that script. Supplying the
/// Chromium switch at process creation ensures no cookies or browser state can
/// land in the generic profile. Electron may still leave an empty `Electron`
/// directory during bootstrap; it is not used as a profile.
fn electron_user_data_dir() -> Result<PathBuf> {
    let base = match std::env::var("XDG_CONFIG_HOME") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(std::env::var("HOME").context("HOME is not set")?).join(".config"),
    };
    let path = base.join(crate::SIDECAR_PROFILE_NAME);
    crate::private_storage::ensure_dir(&path)
        .context("creating Jamelade's private browser directory")?;
    Ok(path)
}

/// Start the sidecar. Returns a handle for commands and a receiver of events.
///
/// The receiver ends with exactly one `Incoming::Died`, which is `app/mod.rs`'s cue
/// to restart with backoff.
pub fn spawn() -> Result<(Handle, mpsc::Receiver<Incoming>)> {
    let dir = locate()?;
    let bin = electron_binary(&dir)?;
    let user_data = electron_user_data_dir()?;
    tracing::info!("spawning sidecar");

    let mut child = TokioCommand::new(&bin)
        .arg(format!("--user-data-dir={}", user_data.display()))
        .arg(".")
        .env("JAMELADE_SIDECAR_IDENTITY", crate::SIDECAR_IDENTITY)
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // **Piped, not inherited, and that is not about tidiness.**
        //
        // Inherited, Chromium holds the real terminal — and its children can
        // outlive us by a few milliseconds. Bytes landing after the shell has
        // taken the terminal back interleave with the shell's own setup: on
        // fish 4.8 and ghostty that truncated the kitty-keyboard negotiation
        // and left the terminal echoing `^[[27u` for every Escape and arrow
        // key until it was cleared. Confirmed by `cargo run 2>&1 | cat`, which
        // gives Chromium no terminal and does not break.
        //
        // Reading it ourselves also puts Chromium's noise behind `RUST_LOG`
        // and gives it a timestamp, which inheriting never could.
        .stderr(Stdio::piped())
        // Electron re-executes itself for its zygote/GPU processes; killing the
        // parent on drop keeps a crashed run from leaving Chromium behind.
        .kill_on_drop(true)
        .spawn()
        .context("failed to start the bundled Electron sidecar")?;

    let stdout = child.stdout.take().context("child stdout was not piped")?;
    let mut stdin = child.stdin.take().context("child stdin was not piped")?;
    let stderr = child.stderr.take().context("child stderr was not piped")?;

    // Logger task — owns stderr, and outlives nothing: when the pipe closes
    // this ends, which is what keeps Chromium's parting words off a terminal
    // somebody else now owns.
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr);
        while let Ok(Some(line)) = read_bounded_line(&mut lines, MAX_DIAGNOSTIC_LINE_BYTES).await {
            let BoundedLine::Data(line) = line else {
                tracing::debug!("chromium emitted an oversized diagnostic line");
                continue;
            };
            let Ok(line) = std::str::from_utf8(&line) else {
                tracing::debug!("chromium emitted non-UTF-8 diagnostic output");
                continue;
            };
            // Our own `log()` in main.js prefixes its lines; everything else is
            // Chromium talking to itself. The first is worth seeing by default,
            // the second only when something is being diagnosed.
            match line.strip_prefix("[sidecar] ") {
                // This exact, data-free marker means the hostname gate refused
                // a request. Keep hostnames and URLs suppressed, but make the
                // fact visible at the default warning level so a missing Apple
                // CDN cannot masquerade as a generic MusicKit failure again.
                Some("blocked non-Apple network request") => {
                    tracing::warn!("sidecar blocked a non-Apple network request")
                }
                Some(msg) => tracing::info!(%msg, "sidecar"),
                None if line.trim().is_empty() => {}
                // Browser diagnostics can contain URLs and page state. Their
                // contents are deliberately suppressed even when debug logging
                // is enabled; the typed sidecar protocol carries useful errors.
                None => tracing::debug!("chromium emitted diagnostic output"),
            }
        }
    });

    let (evt_tx, evt_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(COMMAND_CHANNEL_CAPACITY);
    let broker_pending: std::sync::Arc<AsyncMutex<PendingBroker>> = Default::default();
    let broker = Broker {
        tx: cmd_tx.clone(),
        pending: broker_pending.clone(),
        next_id: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
    };

    // Writer task — owns stdin.
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            let mut line = match serde_json::to_vec(&cmd) {
                Ok(v) => v,
                Err(err) => {
                    tracing::error!(?err, "failed to serialise command");
                    continue;
                }
            };
            line.push(b'\n');
            if let Err(err) = stdin.write_all(&line).await {
                tracing::warn!(?err, "sidecar stdin closed");
                break;
            }
            if let Err(err) = stdin.flush().await {
                tracing::warn!(?err, "sidecar stdin flush failed");
                break;
            }
        }
    });

    // Reader task — owns stdout, and owns waiting on the child so the exit
    // status is reported on the same channel, strictly after the last event.
    let died_tx = evt_tx.clone();
    let reader_pending = broker_pending.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout);
        loop {
            match read_bounded_line(&mut lines, MAX_EVENT_LINE_BYTES).await {
                Ok(Some(BoundedLine::Data(line))) => {
                    if line.iter().all(u8::is_ascii_whitespace) {
                        continue;
                    }
                    let msg = match serde_json::from_slice::<Event>(&line) {
                        Ok(Event::ApiResponse(ApiResponse {
                            request_id,
                            status,
                            body,
                        })) => {
                            if let Some(waiter) = reader_pending.lock().await.remove(&request_id) {
                                let _ = waiter.send(ApiReply { status, body });
                            } else {
                                tracing::debug!(request_id, "late or unknown broker response");
                            }
                            continue;
                        }
                        Ok(ev) => Incoming::Event(ev),
                        Err(err) => {
                            // Never log or retain the raw line: a token event
                            // whose schema drifted is precisely what failed to
                            // parse, and the raw JSON contains both credentials.
                            tracing::warn!(?err, "unparsed sidecar event");
                            Incoming::Unparsed
                        }
                    };
                    if evt_tx.send(msg).await.is_err() {
                        break; // app is gone
                    }
                }
                Ok(Some(BoundedLine::TooLong)) => {
                    tracing::warn!("oversized sidecar event refused");
                    if evt_tx.send(Incoming::Unparsed).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break, // EOF: the child closed stdout
                Err(err) => {
                    tracing::warn!(?err, "sidecar stdout read failed");
                    break;
                }
            }
        }

        // Dropping the senders wakes every outstanding caller as unavailable.
        reader_pending.lock().await.clear();
        let reason = match child.wait().await {
            Ok(status) => format!("sidecar exited: {status}"),
            Err(err) => format!("sidecar wait failed: {err}"),
        };
        let _ = died_tx.send(Incoming::Died(reason)).await;
    });

    Ok((
        Handle {
            tx: cmd_tx,
            broker,
            rate: Default::default(),
            queue_warned: Default::default(),
        },
        evt_rx,
    ))
}

/// Backoff for supervised restarts (rule 6): 1s, 2s, 4s, 8s, capped at 30s.
/// Capped rather than unbounded because a laptop that wakes from suspend
/// should recover promptly, not sit in a 20-minute backoff.
pub fn restart_delay(attempt: u32) -> std::time::Duration {
    let secs = 1u64 << attempt.min(5);
    std::time::Duration::from_secs(secs.min(30))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A handle with no child on the other end. The channel is what `send`
    /// writes to; for the ceiling only the counting matters.
    fn handle() -> (Handle, mpsc::Receiver<Command>) {
        let (tx, rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let pending = Default::default();
        (
            Handle {
                tx: tx.clone(),
                broker: Broker {
                    tx,
                    pending,
                    next_id: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
                },
                rate: Default::default(),
                queue_warned: Default::default(),
            },
            rx,
        )
    }

    #[test]
    fn broker_paths_are_relative_and_bounded() {
        assert!(valid_broker_path("/catalog/us/albums/1?include=tracks"));
        assert!(valid_broker_path("/catalog/us/search?term=hello%20world"));
        assert!(!valid_broker_path("https://example.com/collect"));
        assert!(!valid_broker_path("//example.com/collect"));
        assert!(!valid_broker_path("/catalog/us/search#fragment"));
        assert!(!valid_broker_path("/catalog/us/search\nsecret"));
        assert!(!valid_broker_path(&format!(
            "/{}",
            "x".repeat(MAX_BROKER_PATH_BYTES)
        )));
    }

    #[test]
    fn oversized_child_lines_are_discarded_without_losing_the_next_event() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let input = b"ok\n12345\nnext\n";
            let mut reader = BufReader::new(&input[..]);
            assert_eq!(
                read_bounded_line(&mut reader, 4).await.unwrap(),
                Some(BoundedLine::Data(b"ok".to_vec()))
            );
            assert_eq!(
                read_bounded_line(&mut reader, 4).await.unwrap(),
                Some(BoundedLine::TooLong)
            );
            assert_eq!(
                read_bounded_line(&mut reader, 4).await.unwrap(),
                Some(BoundedLine::Data(b"next".to_vec()))
            );
            assert_eq!(read_bounded_line(&mut reader, 4).await.unwrap(), None);
        });
    }

    #[test]
    fn a_command_storm_is_cut_off_at_the_ceiling() {
        // The failure this exists to stop: a loop in `update()` emitted 5,721
        // commands, each an NDJSON write, a journald record and a D-Bus
        // property change, and the desktop stopped responding (#37).
        let (h, mut rx) = handle();
        for _ in 0..5_000 {
            h.send(Command::SetVolume { volume: 0.5 });
        }
        let mut delivered = 0;
        while rx.try_recv().is_ok() {
            delivered += 1;
        }
        assert_eq!(
            delivered, MAX_PER_SECOND as usize,
            "the ceiling should have cut this off"
        );
    }

    #[test]
    fn the_ceiling_is_per_command_kind() {
        // One runaway command must not silence the rest. A volume loop that
        // also blocked `pause` would leave the user unable to stop the noise.
        let (h, mut rx) = handle();
        for _ in 0..5_000 {
            h.send(Command::SetVolume { volume: 0.5 });
        }
        h.send(Command::Pause);
        let mut pauses = 0;
        while let Ok(cmd) = rx.try_recv() {
            if matches!(cmd, Command::Pause) {
                pauses += 1;
            }
        }
        assert_eq!(pauses, 1, "pause was collateral damage");
    }

    #[test]
    fn ordinary_use_never_reaches_it() {
        // The ceiling must sit above the fastest thing a person can do, which
        // is a pointer drag at the display's refresh rate. Clipping that would
        // be worse than the bug it guards against.
        let (h, mut rx) = handle();
        // 200 in a second is already beyond a 165Hz pointer dragging flat out.
        for _ in 0..200 {
            h.send(Command::SetVolume { volume: 0.5 });
        }
        let mut delivered = 0;
        while rx.try_recv().is_ok() {
            delivered += 1;
        }
        assert_eq!(delivered, 200, "a real drag must not be clipped");
    }

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(restart_delay(0).as_secs(), 1);
        assert_eq!(restart_delay(1).as_secs(), 2);
        assert_eq!(restart_delay(3).as_secs(), 8);
        assert_eq!(restart_delay(5).as_secs(), 30);
        assert_eq!(restart_delay(99).as_secs(), 30, "must stay bounded");
    }

    #[test]
    fn a_missing_electron_names_the_fix() {
        // ARCHITECTURE.md: errors name the fix. "Electron not installed" on its own
        // sends you to a search engine; the command to run does not.
        // (Deliberately not testing `locate()` via env vars — `set_var` is
        // process-global and tests run in parallel threads.)
        let err = electron_binary(Path::new("/nonexistent/slipmat")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("make sidecar"), "unhelpful error: {msg}");
        assert!(
            !msg.contains("/nonexistent"),
            "path leaked into error: {msg}"
        );
    }
}

#[cfg(test)]
mod data_dirs_tests {
    use super::*;

    #[test]
    fn the_system_data_dirs_default_to_what_xdg_mandates() {
        // A packaged install lands in one of these. If this ever returns
        // nothing, `/usr/share/jamelade/sidecar` becomes unreachable and every
        // distribution package silently stops working.
        //
        // Read from the environment, so this asserts on the shape rather than
        // on the values: a test that mutates the environment is a test that
        // breaks whichever other test runs beside it.
        let dirs = dirs_data_dirs();
        assert!(!dirs.is_empty(), "there must always be somewhere to look");
        assert!(dirs.iter().all(|p| p.is_absolute()));
    }
}
