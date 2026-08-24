// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Jamelade sidecar — the invisible half of the app.
//
// This process exists for exactly one reason: Widevine. Apple Music full tracks
// are HLS + Widevine, and on Linux the only CDM that exists ships inside
// Chromium. So we run castLabs Electron, load the real music.apple.com in a
// window that is never shown, and drive MusicKit from Rust over stdio.
//
// The window is shown exactly once — for Apple's own sign-in — and then hides
// forever. Chromium's session and HTTP cache stay memory-only; a separate
// OS-keyring-encrypted vault persists only Apple's cookies, never localStorage,
// IndexedDB, cached requests or responses.
//
// PROTOCOL: newline-delimited JSON.
//   stdin  <- commands from Rust   { id?, cmd, ...args }
//   stdout -> events to Rust       { event, ...payload }
//   stderr -> human logs (Chromium's own noise lands here too)
//
// *** NOTHING may write to stdout except send(). A stray console.log corrupts
// *** the channel and the Rust side will drop the connection. Use log().

const {
  app,
  components,
  BrowserWindow,
  powerSaveBlocker,
  safeStorage,
  session,
} = require('electron')
const fs = require('node:fs')
const path = require('node:path')
const readline = require('node:readline')
const {
  MAX_PROTOCOL_BYTES,
  isAllowedNetworkUrl,
  isAllowedRendererEvent,
  isTrustedAppleUrl,
  mayPersistCookies,
  runtimeIdentity,
  safeErrorDetail,
  serializedSize,
} = require('./security')
const { createCookieVault } = require('./session-vault')

// JAMELADE_SHOW_SIDECAR=1 keeps the window on screen. This is the fastest way
// to tell a frozen renderer from a broken command: if playback works with the
// window visible and not without, the problem is Chromium freezing a page it
// thinks nobody is looking at.
//
// **The env var, not a flag.** `npm run debug` used to pass `--debug`, which
// never reached here: Electron reads it as Node's long-deprecated `--debug`
// and exits before the app starts ("`node --debug` ... are invalid", make
// Error 9). So the one documented tool for isolating an Apple or DRM problem
// from a Rust one did not run at all. The argv check is kept because it costs
// nothing and still works if the flag is passed somewhere Electron ignores it.
const DEBUG =
  process.argv.includes('--debug') || process.env.JAMELADE_SHOW_SIDECAR === '1'

/// Per-command logging. **Off by default, and that is a safety property rather
/// than tidiness.**
///
/// `log()` writes to stderr, which under a .desktop launch is journald, which
/// writes to disk synchronously. One line per command is fine at the handful
/// per minute a person generates — and is the single biggest amplifier when
/// something loops. A two-way binding on the volume button once emitted 5,721
/// commands and the machine had to be power-cycled; those disk writes are a
/// large part of why an app bug became a system one (#37).
///
/// `JAMELADE_SIDECAR_TRACE=1` brings it back for the evening you need it. The
/// protocol events it duplicates — `cmd-recv`, `cmd-done`, `cmd-queued` — are
/// unaffected, and are what ARCHITECTURE.md's "diagnose by layer" actually reads.
const TRACE = process.env.JAMELADE_SIDECAR_TRACE === '1'
const APPLE_MUSIC = 'https://music.apple.com/'
const runtime = runtimeIdentity(process.env.JAMELADE_SIDECAR_IDENTITY)
/// Where the live login lives. No `persist:` prefix: all Chromium-managed
/// storage is memory-only. The encrypted cookie vault is the sole persistence.
const activePartition = runtime.partition
let cookieVault = null
let playerSession = null
let shuttingDown = false
const PAGE_HOOK = fs.readFileSync(path.join(__dirname, 'page-hook.js'), 'utf8')
/// How the window stays out of the way. Set JAMELADE_SIDECAR_WINDOW to override:
///
///   hidden     (default) never mapped. Completely invisible — nothing in the
///              window overview, nothing in the dash. Verified on GNOME/Wayland
///              with playback left running for a long stretch and still
///              responding afterwards.
///   concealed  mapped but 1x1, transparent and click-through. Kept as an
///              escape hatch in case a compositor does freeze the renderer of
///              an unmapped window; the cost is a speck in the overview.
///
/// Note the --disable-renderer-backgrounding family below was already in place
/// when `hidden` was verified, so those switches may well be what makes it
/// viable. Do not remove them and assume this still works.
const WINDOW_MODE = process.env.JAMELADE_SIDECAR_WINDOW || 'hidden'

const READY_TIMEOUT_MS = 60_000
const PROBE_INTERVAL_MS = 500
/// How many times to re-ask for session state after wiring. Authorization can
/// settle just after MusicKit; ten seconds is plenty and then it stops.
const SESSION_NUDGES = 10
const MAX_PENDING_COMMANDS = 128
const MAX_RENDERER_EVENTS_PER_SECOND = 60

// These four switches are what make a permanently-hidden window actually work.
// All must be set before app.whenReady(). Do not remove any of them without
// re-running the standalone sidecar test — each one was added to fix an
// observed, silent failure.
//
//   autoplay-policy
//     Chromium refuses to start audio until a page has "user activation" — a
//     real click inside it. Our window is hidden and driven entirely over IPC,
//     so it NEVER receives one, and MusicKit's play() resolves without
//     producing sound.
//
//   disable-renderer-backgrounding / disable-background-timer-throttling /
//   disable-backgrounding-occluded-windows
//     A window created with show:false counts as hidden AND occluded, and
//     Chromium will freeze such a renderer: timers stop firing and the page
//     stops making progress. webPreferences.backgroundThrottling alone does
//     NOT cover it. These three are almost certainly what lets the sidecar run
//     entirely unmapped (WINDOW_MODE=hidden), which is verified working — so
//     treat them as load-bearing, not as leftovers from a fixed bug.
// Identity. Without these the sidecar shows up in the dash and window list as
// a separate app called "jamelade-sidecar" (from package.json's name), with a
// generic icon. Pointing it at Jamelade's own desktop entry makes the shell
// treat any window it does show as part of Jamelade rather than a stray second
// app. `userData` is explicit too: when Electron is launched against a script
// rather than a packaged archive it otherwise falls back to `~/.config/Electron`,
// which could mix the encrypted cookie vault and Widevine component state with
// an unrelated raw Electron app on a native install. Must all happen before
// app.whenReady().
app.setName(runtime.appName)
app.setPath('userData', path.join(app.getPath('appData'), runtime.profileName))
if (process.platform === 'linux') {
  app.setDesktopName(runtime.desktopName)
  // Chromium otherwise silently falls back to `basic_text` when a keyring is
  // unavailable. We detect that after app readiness and use an ephemeral
  // partition instead of writing weakly protected Apple cookies to disk.
  app.commandLine.appendSwitch('password-store', 'gnome-libsecret')
}

// Chromium publishes its OWN MPRIS player as soon as a page plays media, and
// grabs the hardware media keys with it. Jamelade exports MPRIS itself (see
// src/mpris.rs), so leaving these on gives the shell two identical "Jamelade"
// players — the visible symptom — and lets an invisible Chromium win the race
// for Play/Pause on the keyboard.
//
// MediaSessionService is the MPRIS bridge; HardwareMediaKeyHandling is the key
// grab. Neither has anything to do with decoding audio, so disabling them costs
// nothing and leaves exactly one player on the bus: ours.
app.commandLine.appendSwitch(
  'disable-features',
  'MediaSessionService,HardwareMediaKeyHandling',
)

app.commandLine.appendSwitch('autoplay-policy', 'no-user-gesture-required')
// No GPU process. The window is never mapped (WINDOW_MODE=hidden), so nothing
// this renderer draws is ever seen — and it was paying for compositing anyway.
//
// Measured while playing, PSS across the whole tree:
//
//              total    renderer   gpu     cpu
//   before     856 MB   399 MB     106 MB  24.8%
//   after      587 MB   169 MB      60 MB   7.1%
//
// The renderer is where most of it went: without a GPU process it stops holding
// the raster and texture buffers that back a surface nobody looks at. 269 MB and
// most of the sidecar's idle CPU, for a picture that is never presented.
//
// Audio is unaffected — Widevine on Linux decrypts in software and this is an
// audio-only client. If Jamelade ever plays video, revisit this first.
app.commandLine.appendSwitch('disable-gpu')
app.commandLine.appendSwitch('disable-renderer-backgrounding')
app.commandLine.appendSwitch('disable-background-timer-throttling')
app.commandLine.appendSwitch('disable-backgrounding-occluded-windows')

let win = null
/** Queued commands that arrived before the hook was ready. */
let pending = []
let hookReady = false
let suspensionBlocker = null
let probeTimer = null
let sessionTimer = null
let loginRefreshTimer = null
let loginRefreshPending = false
let loginRefreshInFlight = false
let rendererEventWindow = { started: Date.now(), count: 0 }
const blockedLogAt = new Map()

// ---------------------------------------------------------------------------
// Protocol
// ---------------------------------------------------------------------------

function send(msg) {
  let encoded
  try {
    encoded = JSON.stringify(msg)
  } catch {
    encoded = '{"event":"error","code":"protocol-encode","detail":"Sidecar event could not be encoded."}'
  }
  if (Buffer.byteLength(encoded, 'utf8') > MAX_PROTOCOL_BYTES) {
    encoded = '{"event":"error","code":"protocol-too-large","detail":"Sidecar event exceeded the safe size limit."}'
  }
  process.stdout.write(encoded + '\n')
}

function log(...args) {
  // stderr, never stdout — see the header.
  process.stderr.write('[sidecar] ' + args.join(' ') + '\n')
}

function logBlocked(kind) {
  const now = Date.now()
  const previous = blockedLogAt.get(kind) || 0
  if (now - previous < 5000) return
  blockedLogAt.set(kind, now)
  log(kind)
}

function rendererEventWithinRate() {
  const now = Date.now()
  if (now - rendererEventWindow.started >= 1000) {
    rendererEventWindow = { started: now, count: 0 }
  }
  rendererEventWindow.count += 1
  return rendererEventWindow.count <= MAX_RENDERER_EVENTS_PER_SECOND
}

function fail(code, detail) {
  const safe = safeErrorDetail(detail)
  // The typed code is enough for persistent diagnostics. Even redacted remote
  // exception text can contain private URLs, paths or media identifiers.
  log('ERROR', code)
  send({ event: 'error', code, detail: safe })
}

function isMainMusicPage(url) {
  try {
    const parsed = new URL(url)
    return parsed.protocol === 'https:' && parsed.hostname === 'music.apple.com'
  } catch {
    return false
  }
}

async function installPageHook() {
  if (!win || win.isDestroyed() || !isMainMusicPage(win.webContents.getURL())) return
  try {
    await win.webContents.executeJavaScript(PAGE_HOOK)
  } catch (err) {
    fail('hook-injection-failed', err && err.message)
  }
}

function configureNavigation(contents, playerWindow) {
  const leave = (event, url) => {
    if (isTrustedAppleUrl(url)) return
    event.preventDefault()
    logBlocked('blocked navigation outside the Apple player')
  }
  contents.on('will-navigate', leave)
  contents.on('will-redirect', leave)

  contents.setWindowOpenHandler(({ url }) => {
    // Popups exist only for an explicit, visible sign-in. A compromised page
    // playing invisibly in the background must not manufacture browser
    // windows, even to another nominally trusted Apple origin.
    if (!isTrustedAppleUrl(url) || !win || !win.isVisible()) {
      logBlocked('blocked unexpected popup')
      return { action: 'deny' }
    }

    // Apple occasionally uses a popup for authentication. It receives no
    // player bridge and retains the same sandboxed, non-Node security posture.
    return {
      action: 'allow',
      overrideBrowserWindowOptions: {
        autoHideMenuBar: true,
        webPreferences: {
          preload: path.join(__dirname, 'auth-preload.js'),
          partition: activePartition,
          contextIsolation: true,
          nodeIntegration: false,
          sandbox: true,
          webSecurity: true,
          allowRunningInsecureContent: false,
          devTools: DEBUG,
          spellcheck: false,
          webviewTag: false,
          enableWebSQL: false,
          navigateOnDragDrop: false,
          safeDialogs: true,
        },
      },
    }
  })

  if (playerWindow) {
    contents.on('did-finish-load', () => installPageHook())
  }
}

function configurePlayerSession() {
  // Playback and sign-in need ordinary HTTPS, not capture, location, MIDI,
  // notifications or any future web permission Electron adds.
  playerSession.setPermissionCheckHandler(() => false)
  playerSession.setPermissionRequestHandler((_wc, _permission, cb) => cb(false))

  // The renderer carries live Apple cookies and tokens. A compromised Apple
  // page must not be able to beacon them or listening data to an arbitrary
  // third-party origin via fetch, images, WebSockets or redirects.
  playerSession.webRequest.onBeforeRequest(
    { urls: ['<all_urls>'] },
    (details, callback) => {
      const allowed = isAllowedNetworkUrl(details.url)
      if (!allowed) logBlocked('blocked non-Apple network request')
      callback({ cancel: !allowed })
    },
  )

  // Nothing in an invisible audio engine should ever download a file to the
  // host. This also closes a disk-fill route from compromised page content.
  playerSession.on('will-download', (event) => event.preventDefault())
}

function chooseCookiePartition() {
  let backend = 'unknown'
  let encryptionAvailable = false
  try {
    encryptionAvailable = safeStorage.isEncryptionAvailable()
    if (process.platform === 'linux') backend = safeStorage.getSelectedStorageBackend()
  } catch {
    encryptionAvailable = false
  }
  const persistent = mayPersistCookies(encryptionAvailable, backend)
  send({ event: 'storage-mode', persistent })
  return persistent
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

async function createWindow() {
  win = new BrowserWindow({
    show: DEBUG,
    width: 1100,
    height: 760,
    // Constructor-time, not just setSkipTaskbar() — some shells only honour it
    // at map time, and by then the window has already been listed.
    skipTaskbar: true,
    // No menu bar, no chrome — on the rare occasion this is visible it is
    // Apple's login and nothing else.
    autoHideMenuBar: true,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      partition: activePartition,

      // The remote Apple page never shares a JavaScript world with Electron.
      // A narrow contextBridge in preload.js carries bounded player events.
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webSecurity: true,
      allowRunningInsecureContent: false,
      devTools: DEBUG,
      spellcheck: false,
      webviewTag: false,
      enableWebSQL: false,
      navigateOnDragDrop: false,
      safeDialogs: true,

      // CRITICAL: Chromium throttles timers and media in windows it believes
      // are hidden. Our window is *always* hidden. Without this, playback
      // stutters or stops the moment focus moves elsewhere.
      backgroundThrottling: false,
    },
  })

  configureNavigation(win.webContents, true)
  win.webContents.on('did-create-window', (child) => {
    configureNavigation(child.webContents, false)
  })

  // Signing in navigates the page, which tears down the old preload context
  // and builds a new one. Until that new hook reports in, `music` is null over
  // there — so commands must go back to the pending queue rather than being
  // forwarded into a context that will throw. Without this, every command sent
  // between sign-in and the new hook-ready is silently lost.
  // ONLY a real cross-document navigation in the main frame invalidates the
  // hook. `did-start-loading` is the wrong event and was a serious bug: it
  // fires for SPA route changes and subresource loads too, so on
  // music.apple.com it flipped hookReady to false within seconds and it never
  // came back — `hook-ready` is emitted once per document, not once per load.
  // Every command from Rust after that was queued forever, with no error
  // anywhere. Symptom: refreshSession (sent straight from main.js, bypassing
  // dispatch) kept working while setQueue vanished.
  win.webContents.on('did-start-navigation', (...args) => {
    // Electron has changed this signature across versions: older releases pass
    // (event, url, isInPlace, isMainFrame, …), newer ones a single details
    // object. Accept both rather than silently reading undefined.
    const first = args[0]
    const d =
      first && typeof first === 'object' && 'isMainFrame' in first
        ? first
        : { isMainFrame: args[3], isSameDocument: args[2] }

    if (d.isMainFrame && !d.isSameDocument) {
      log('main-frame navigation — hook invalidated, re-probing')
      hookReady = false
      // Re-arm the probe. The new document gets a fresh preload which will
      // self-poll, but the probe is the backstop that guarantees a re-wire —
      // otherwise an invalidated hook could never recover and every later
      // command would queue forever.
      probeForMusicKit()
    }
  })

  win.on('close', (e) => {
    // Closing the login window must not kill playback; Rust owns our lifetime.
    e.preventDefault()
    conceal()
    send({ event: 'window-hidden' })
  })

  await win.loadURL(APPLE_MUSIC)
  await installPageHook()
  log('loaded', APPLE_MUSIC)
  if (!DEBUG) conceal()
  probeForMusicKit()
}

/// Put the window away. See WINDOW_MODE.
///
/// Default is a plain hide — genuinely unmapped, so it appears nowhere in the
/// shell. `concealed` is the fallback: mapped but 1x1, transparent and
/// click-through, for a compositor that freezes unmapped renderers.
function conceal() {
  // Tell the OS this process must not be suspended. On its own this does not
  // stop Chromium's per-page freezing, but without it a laptop on battery can
  // suspend the whole sidecar mid-track.
  if (suspensionBlocker === null) {
    suspensionBlocker = powerSaveBlocker.start('prevent-app-suspension')
  }

  if (WINDOW_MODE === 'hidden') {
    // Truly invisible: nothing in the overview, nothing in the dash.
    win.hide()
    log('window mode: hidden (not mapped)')
    // Asked of Electron rather than inferred: the switch does not appear in the
    // main process's argv (`appendSwitch` sets Chromium's internal command
    // line), and the child processes that *do* carry it are transient enough
    // that reading /proc races them. An empty value here means the cap is not
    // in force and the cache is unbounded again.
    log('HTTP cache: disabled')
    return
  }

  win.setOpacity(0)
  win.setIgnoreMouseEvents(true)
  win.setSkipTaskbar(true)
  win.setSize(1, 1)
  win.showInactive()
  log('window mode: concealed (mapped, 1x1, transparent)')
}

/// The inverse, for Apple's sign-in — the one time the user sees this window.
function reveal() {
  // Only a login explicitly opened in this process may request the one-time
  // post-auth refresh below. Restored sessions arrive authorized at startup
  // and must not enter a reload loop.
  loginRefreshPending = true
  win.setOpacity(1)
  win.setIgnoreMouseEvents(false)
  win.setSkipTaskbar(false)
  win.setSize(1100, 760)
  win.center()
  win.show()
  win.focus()
}

function scheduleLoginPlayerRefresh() {
  if (!loginRefreshPending || loginRefreshInFlight || loginRefreshTimer) return
  // `authReflectionDidComplete` normally wins first. This is a bounded
  // compatibility fallback for a future MusicKit build that still yields a
  // valid user token but drops or renames that event.
  loginRefreshTimer = setTimeout(() => {
    loginRefreshTimer = null
    refreshPlayerAfterLogin()
  }, 1_500)
}

async function refreshPlayerAfterLogin() {
  if (!loginRefreshPending || loginRefreshInFlight || !win || win.isDestroyed()) return
  loginRefreshPending = false
  loginRefreshInFlight = true
  clearTimeout(loginRefreshTimer)
  loginRefreshTimer = null
  hookReady = false
  // Do not leave the freshly authorized Apple page visible if MusicKit emits
  // reflection before its separate authorization event. Token delivery has
  // already told Rust the account is ready; this window has finished its one
  // visible job.
  conceal()

  // Persist the completed session before replacing the document. The live
  // in-memory cookie store is already authoritative, so a vault failure must
  // not strand playback in preview mode.
  if (cookieVault) {
    try {
      await cookieVault.flush()
    } catch {
      log('cookie vault flush after sign-in failed')
    }
  }

  try {
    await win.loadURL(APPLE_MUSIC)
    await installPageHook()
    probeForMusicKit()
    log('player session refreshed after sign-in')
  } catch (err) {
    fail('post-login-refresh-failed', err && err.message)
  } finally {
    loginRefreshInFlight = false
  }
}

/// Poll the renderer until MusicKit exists, then tell the preload to wire up.
///
/// This runs in the MAIN process on purpose. A show:false window has its
/// renderer frozen by Chromium — a setTimeout loop inside the page fires once
/// and then stops — so readiness cannot be detected from in there.
/// executeJavaScript still runs, so we drive it from out here.
function probeForMusicKit() {
  // Probes and session nudges are module-level and always cleared first.
  // They used to be per-call locals, so every re-probe (one per main-frame
  // navigation) leaked another probe AND another 10-shot session nudger — which
  // is why refreshSession arrived several times a second, forever, instead of
  // ten times at startup.
  clearInterval(probeTimer)
  clearInterval(sessionTimer)

  const deadline = Date.now() + READY_TIMEOUT_MS
  let wired = false

  probeTimer = setInterval(async () => {
    if (wired || !win || win.isDestroyed()) return clearInterval(probeTimer)

    // Deadline is checked BEFORE the await on purpose. If the renderer is
    // frozen, executeJavaScript never settles — and a deadline check placed
    // after the await would then be unreachable, which is exactly how the
    // freeze first presented: no hook-ready, no hook-failed, no error at all.
    if (Date.now() > deadline) {
      clearInterval(probeTimer)
      return send({
        event: 'hook-failed',
        detail: 'MusicKit never appeared on music.apple.com',
      })
    }

    // Electron defers executeJavaScript until the page stops loading, and it
    // implements that by attaching a `did-stop-loading` listener per call. So
    // probing a still-loading document queues one listener per tick and trips
    // "MaxListenersExceededWarning: 11 did-stop-loading listeners added".
    //
    // Skipping the tick is the fix rather than raising maxListeners: there is
    // nothing to find on a document that has not finished loading, so those
    // calls were never going to answer anything. Deliberately checked *after*
    // the deadline above, so a page that never finishes still times out and
    // reports `hook-failed` instead of probing silently forever.
    if (win.webContents.isLoadingMainFrame()) return

    let ready = false
    try {
      ready = await win.webContents.executeJavaScript(
        'window.__slipmatReady ? window.__slipmatReady() : false',
      )
    } catch {
      log('MusicKit readiness probe failed')
    }

    if (ready) {
      wired = true
      clearInterval(probeTimer)
      win.webContents.send('slipmat:wire')
      // Authorization can settle slightly after MusicKit, so nudge a few
      // times. `sessionTimer` is module-level and cleared at the top of this
      // function — a `const` here shadowed it, so the nudger was unstoppable
      // and every re-probe added another one.
      let nudges = 0
      sessionTimer = setInterval(() => {
        if (++nudges > SESSION_NUDGES || !win || win.isDestroyed()) {
          return clearInterval(sessionTimer)
        }
        win.webContents.send('slipmat:command', { cmd: 'refreshSession' })
      }, 1000)
    }
  }, PROBE_INTERVAL_MS)
}

// ---------------------------------------------------------------------------
// Commands from Rust
// ---------------------------------------------------------------------------

/// Actually sign out: drop Apple's session, not just MusicKit's token.
///
/// `music.unauthorize()` was the whole of sign-out, and it is a MusicKit call —
/// it clears the Music User Token and nothing else. The login itself is an
/// ordinary browser session, so it survived, and signing back in silently
/// reused it. The hardened build keeps the live session in memory and saves an
/// encrypted cookie vault, but both still have to be cleared here.
///
/// Best-effort and unordered on purpose. `unauthorize` is a courtesy to
/// MusicKit — clearing the storage underneath it is what actually ends the
/// session, so nothing here waits on the renderer, which may be mid-navigation
/// or gone.
async function signOut() {
  // Commands belong to the account/session that queued them. Carrying a
  // pre-login queue across sign-out could replay old catalog ids after another
  // account signs in, leaking listening context and performing stale actions.
  pending = []
  if (win && !win.isDestroyed()) {
    try {
      win.webContents.send('slipmat:command', { cmd: 'unauthorize' })
    } catch {
      log('unauthorize could not be delivered')
    }
  }

  if (cookieVault) cookieVault.suspend()
  try {
    // Cookies, every web storage that can hold an identity, and cached HTTP
    // responses that could contain library data. The Widevine CDM is outside
    // this partition, so it is untouched and does not re-download.
    await playerSession.clearStorageData({
      storages: [
        'cookies',
        'filesystem',
        'localstorage',
        'sessionstorage',
        'indexdb',
        'websql',
        'serviceworkers',
        'cachestorage',
      ],
    })
    await playerSession.clearCache()
    if (cookieVault) await cookieVault.clearSaved()
    log('session cleared')
  } catch (err) {
    // Say so rather than reporting a sign-out that did not happen — this is the
    // failure the whole function exists to stop being silent.
    log('clearing the session failed')
    send({ event: 'error', code: 'sign-out-failed', detail: safeErrorDetail(err) })
    return
  } finally {
    if (cookieVault) cookieVault.resume()
  }

  // Reload so the next sign-in starts from a document that never saw the old
  // account. Without this the page keeps running with its in-memory MusicKit
  // instance and looks signed in until something forces a navigation.
  if (win && !win.isDestroyed()) {
    hookReady = false
    try {
      await win.loadURL(APPLE_MUSIC)
      probeForMusicKit()
    } catch {
      log('reload after sign-out failed')
    }
  }
  send({ event: 'signed-out' })
}

async function shutdown() {
  if (shuttingDown) return
  shuttingDown = true
  if (cookieVault) {
    try {
      await cookieVault.flush()
    } catch {
      log('cookie vault flush failed')
    }
  }
  app.exit(0)
}

function dispatch(msg) {
  if (!msg || typeof msg !== 'object' || typeof msg.cmd !== 'string') {
    throw new Error('command must be an object with a string cmd')
  }
  // Handled here, not in the page.
  switch (msg.cmd) {
    case 'showLogin':
      if (win) reveal()
      return
    case 'hide':
      // Always conceal(), never win.hide() directly — conceal() is what
      // honours WINDOW_MODE.
      if (win) conceal()
      return
    case 'quit':
      shutdown()
      return
    case 'signOut':
      // Main process, not the page: `session.clearStorageData` is a
      // main-process API, and the page cannot delete the cookies that keep it
      // logged in. That is precisely why sign-out used to leave them behind.
      signOut()
      return
  }

  if (!hookReady) {
    // Never queue silently. A command that is waiting is indistinguishable
    // from one that was dropped unless it says so, and that ambiguity cost
    // three debugging rounds.
    if (pending.length >= MAX_PENDING_COMMANDS) {
      send({ event: 'error', code: 'command-queue-full', detail: 'Playback command queue is full.' })
      return
    }
    pending.push(msg)
    log('queued (hook not ready):', msg.cmd, 'depth=', pending.length)
    send({ event: 'cmd-queued', cmd: msg.cmd, depth: pending.length })
    return
  }
  // `visible` is the diagnostic that matters when a command produces no
  // sound and no error: a window Chromium considers hidden has a frozen
  // renderer that will never run the handler.
  if (TRACE) {
    log('dispatch', msg.cmd, 'visible=', win.isVisible(), 'crashed=', win.webContents.isCrashed())
  }
  win.webContents.send('slipmat:command', msg)
}

function drainPending() {
  const queued = pending
  pending = []
  for (const msg of queued) win.webContents.send('slipmat:command', msg)
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

app.whenReady().then(async () => {
  const persistent = chooseCookiePartition()
  // `cache: false` is part of the privacy boundary. A Chromium HTTP cache can
  // retain request URLs and first-party responses after a crash even when its
  // cookies and web storage are memory-only. Jamelade already has a bounded,
  // app-owned artwork cache and Linux Widevine cannot reuse encrypted HLS data
  // across sessions, so there is no useful persistent browser cache to keep.
  playerSession = session.fromPartition(activePartition, { cache: false })
  try {
    // Remove anything left by releases that used a disk cache. This does not
    // touch the separately managed Widevine component or the encrypted vault.
    await playerSession.clearCache()
  } catch {
    log('legacy HTTP cache could not be cleared')
  }
  configurePlayerSession()
  if (persistent) {
    cookieVault = createCookieVault({
      safeStorage,
      cookieStore: playerSession.cookies,
      filePath: path.join(app.getPath('userData'), 'apple-session.vault'),
      onError: () => log('cookie vault operation failed'),
    })
    await cookieVault.restore()
    cookieVault.watch()
  }
  try {
    // castLabs ECS: the Widevine CDM arrives through Chromium's component
    // updater, so this can take a moment on first run and needs network.
    // Creating a window before it resolves means EME is simply absent.
    await components.whenReady()
    // Component status can include an install path. Readiness is the only fact
    // Rust needs, and keeping paths out avoids putting the account name in
    // journald on native installs.
    log('widevine ready')
    send({ event: 'widevine-ready' })
  } catch (err) {
    fail('widevine-unavailable', err)
    // No CDM means no playback, ever. Say so and exit rather than pretend.
    app.exit(1)
    return
  }

  // The renderer talks back through the same channel name in both directions.
  const { ipcMain } = require('electron')

  ipcMain.on('slipmat:event', (event, ev) => {
    const trustedSender = win
      && event.sender === win.webContents
      && event.senderFrame === win.webContents.mainFrame
      && isMainMusicPage(event.senderFrame.url)
    if (!trustedSender || !isAllowedRendererEvent(ev) || serializedSize(ev) > MAX_PROTOCOL_BYTES) {
      logBlocked('blocked invalid renderer event')
      return
    }
    if (!rendererEventWithinRate()) {
      logBlocked('blocked renderer event flood')
      return
    }
    if (ev.event === 'hook-ready') {
      hookReady = true
      drainPending()
    }
    if (ev.event === 'session'
      && ev.authorized
      && ev.hasUserToken) {
      scheduleLoginPlayerRefresh()
    }
    if (ev.event === 'session') {
      // The page exposes only this credential-free projection. Rust receives
      // it unchanged; authenticated API calls stay inside the browser broker.
      send(ev)
      return
    }
    if (ev.event === 'authorization-reflected') {
      if (ev.authorized) refreshPlayerAfterLogin()
      // This is an internal lifecycle signal. Rust intentionally has no event
      // variant for it and never needs to see it or any authentication detail.
      return
    }
    // Error text is remote-controlled. Redact it once more at the privileged
    // boundary before it can reach Rust, journald or a support report.
    send(typeof ev.detail === 'string'
      ? { ...ev, detail: safeErrorDetail(ev.detail) }
      : ev)
  })

  await createWindow()

  const rl = readline.createInterface({ input: process.stdin })
  rl.on('line', (line) => {
    if (!line.trim()) return
    if (Buffer.byteLength(line, 'utf8') > MAX_PROTOCOL_BYTES) {
      fail('bad-command', 'Command exceeded the safe size limit.')
      return
    }
    let msg
    try {
      msg = JSON.parse(line)
    } catch (err) {
      fail('bad-command', err)
      return
    }
    try {
      dispatch(msg)
    } catch (err) {
      fail('dispatch-failed', err)
    }
  })

  // Rust closing our stdin is the shutdown signal.
  rl.on('close', () => shutdown())

  send({ event: 'ready', debug: DEBUG })
})

// The whole point is to outlive a closed window.
app.on('window-all-closed', () => {})

process.on('uncaughtException', (err) => fail('uncaught', err && err.stack))
