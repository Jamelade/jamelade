// SPDX-FileCopyrightText: 2026 Miguel Rincon
// SPDX-License-Identifier: GPL-3.0-or-later
//
// The page-world hook. This is the ONE fragile surface in Slipmat (ARCHITECTURE.md rule 4):
// it reaches into a page Apple can change without warning.
//
// Rules for editing this file:
//   - Feature-detect everything. Never assume a property exists.
//   - Never scrape the DOM. Only MusicKit.getInstance() and its events.
//     DOM scraping is why wrappers break monthly.
//   - Fail LOUDLY (`hook-failed`) rather than degrade into a dead player.
//   - Keep it small. Every line here is a line that isn't native.

;(function slipmatPageHook() {
'use strict'

if (window.__slipmatPageHookInstalled) return
const bridge = window.slipmatBridge
if (!bridge || typeof bridge.emit !== 'function') return
Object.defineProperty(window, '__slipmatPageHookInstalled', {
  value: true,
  configurable: false,
  enumerable: false,
  writable: false,
})

const READY_TIMEOUT_MS = 60_000
const READY_POLL_MS = 250

let music = null
let tokenTimer = null

const emit = (event, payload = {}) => bridge.emit(event, payload)

// Proof-of-life, sent before anything can go wrong. If Rust sees no
// `hook-boot` at all, the preload is not running and no amount of debugging
// inside it will help — check webPreferences.preload and sandbox instead.
emit('hook-boot', {
  readyState: (typeof document !== 'undefined' && document.readyState) || 'no-document',
})

/** Try a list of accessors and return the first that yields something. */
function pick(...getters) {
  for (const get of getters) {
    try {
      const v = get()
      if (v !== undefined && v !== null && v !== '') return v
    } catch {
      /* keep trying — a throwing getter just means that shape is gone */
    }
  }
  return null
}

function safeDetail(value) {
  return String(value ?? '')
    .replace(/eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g, '<redacted-jwt>')
    .replace(/[A-Za-z0-9+/_=-]{64,}/g, '<redacted-secret>')
    .replace(/[\r\n\t]+/g, ' ')
    .trim()
    .slice(0, 512)
}

// ---------------------------------------------------------------------------
// Readiness
// ---------------------------------------------------------------------------

function getInstance() {
  return pick(() => window.MusicKit && window.MusicKit.getInstance())
}

/// Is MusicKit up? Called BY THE MAIN PROCESS via executeJavaScript, because a
/// timer in here cannot be trusted to run (see the wiring note below).
window.__slipmatReady = () => {
  try {
    return !!(window.MusicKit && window.MusicKit.getInstance())
  } catch {
    return false
  }
}

// ---------------------------------------------------------------------------
// Session state
//
// Credentials never leave this page world. Native code needs only the
// storefront and two booleans; authenticated requests use MusicKit's own API
// below, so there is no reason to copy a developer or Music User Token across
// even the isolated renderer bridge.
// ---------------------------------------------------------------------------

function readSession() {
  if (!music) return null
  const musicUserToken = pick(
    () => music.musicUserToken,
    () => music.api && music.api.userToken,
  )
  // These are not synonyms. `storefrontId` is the catalogue selected by the
  // page URL (music.apple.com currently redirects `/` to `/us/new` here),
  // while `storefrontCountryCode` is the storefront attached to the signed-in
  // Apple Music account. MusicKit refuses full playback when the two differ
  // with CONTENT_EQUIVALENT. Prefer the account and align MusicKit before any
  // queue is built; the page storefront remains the signed-out fallback.
  const accountStorefront = pick(() => music.storefrontCountryCode)
  const storefront = pick(
    () => accountStorefront,
    () => music.storefrontId,
    () => music.api && music.api.storefrontId,
  )
  const normalizedStorefront = String(storefront || 'us').toLowerCase()
  if (pick(() => music.isAuthorized)
    && /^[a-z]{2}$/.test(String(accountStorefront || '').toLowerCase())) {
    try {
      music.storefrontId = String(accountStorefront).toLowerCase()
    } catch {
      // An older MusicKit may expose a read-only storefront. Reporting the
      // account storefront still keeps native API requests correct; a precise
      // command error below remains visible if playback cannot be realigned.
    }
  }
  return {
    storefront: /^[a-z]{2}$/.test(normalizedStorefront) ? normalizedStorefront : 'us',
    authorized: !!pick(() => music.isAuthorized),
    hasUserToken: typeof musicUserToken === 'string' && musicUserToken.length > 0,
  }
}

let lastSession = ''

/// Emit session state only when it actually changes.
///
/// main.js nudges this once a second for the first ten seconds (authorization
/// can settle after MusicKit), and unconditional emitting turned that into
/// ten identical log lines that buried everything else.
function pushSession() {
  const session = readSession()
  if (!session) return null
  const fingerprint = JSON.stringify(session)
  if (fingerprint === lastSession) return session
  lastSession = fingerprint
  emit('session', session)
  return session
}

// ---------------------------------------------------------------------------
// State serialisation — our types stop at the Rust boundary, but keep the
// payload small and stable so player/protocol.rs has a narrow contract.
// ---------------------------------------------------------------------------

const STATES = [
  'none', 'loading', 'playing', 'paused', 'stopped',
  'ended', 'seeking', 'unknown', 'waiting', 'stalled', 'completed',
]
const MAX_QUEUE_ITEMS = 2500
const MAX_API_PATH_BYTES = 4096
const MAX_API_BODY_BYTES = 3 * 1024 * 1024
const API_METHOD_READY_TIMEOUT_MS = 3000
const API_METHOD_READY_POLL_MS = 100

const stateName = (n) => STATES[n] || 'unknown'

function serializeItem(item) {
  if (!item) return null
  return {
    id: pick(() => item.id, () => item.playbackId) || null,
    catalogId: pick(() => item.catalogId, () => item.container && item.container.id) || null,
    title: pick(() => item.title, () => item.attributes && item.attributes.name) || '',
    artist: pick(() => item.artistName, () => item.attributes && item.attributes.artistName) || '',
    album: pick(() => item.albumName, () => item.attributes && item.attributes.albumName) || '',
    durationMs: pick(
      () => item.playbackDuration,
      () => item.attributes && item.attributes.durationInMillis,
    ) || 0,
    trackNumber: pick(() => item.trackNumber, () => item.attributes && item.attributes.trackNumber) || 0,
    // A TEMPLATE url containing {w}/{h}/{f} — Rust substitutes the size it
    // wants and caches to disk, because MPRIS needs a file:// path.
    artworkTemplate: pick(
      () => item.artwork && item.artwork.url,
      () => item.attributes && item.attributes.artwork && item.attributes.artwork.url,
    ) || null,
  }
}

// `reason` is the one thing Rust cannot work out for itself, and the two values
// mean opposite things:
//
//   'items'     the queue was EDITED. MusicKit does not re-index its own
//               position afterwards, so it has to be told where the current
//               track went (#117, #118).
//   'position'  MusicKit moved its own cursor. Correcting that is fighting it —
//               including the pre-advance it does a few hundred ms before every
//               track boundary, which is what makes gapless seamless (#121).
//
// Every caller passes it explicitly. A default here would be a guess at the one
// question this argument exists to answer.
function currentQueue(reason) {
  const items = pick(() => music.queue && music.queue.items) || []
  return {
    reason,
    position: pick(() => music.queue && music.queue.position) ?? 0,
    items: items.slice(0, MAX_QUEUE_ITEMS).map(serializeItem),
  }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

function on(name, fn) {
  try {
    music.addEventListener(name, fn)
  } catch {
    // An event this MusicKit version doesn't know about is survivable; a
    // missing *critical* one shows up as a player that never updates.
    emit('hook-warning', { detail: `no event ${String(name).slice(0, 80)}` })
  }
}

// MusicKit: shuffleMode 0 off / 1 on; repeatMode 0 none / 1 one / 2 all.
function emitModes() {
  emit('modes', {
    shuffle: (pick(() => music.shuffleMode) ?? 0) === 1,
    repeat: ['none', 'one', 'all'][pick(() => music.repeatMode) ?? 0] ?? 'none',
  })
}

function emitVolume() {
  // `?? 1` and not `?? 0`: a MusicKit that cannot tell us is not a silent one,
  // and opening muted because a read failed would be worse than opening loud.
  emit('volume', { volume: pick(() => music.volume) ?? 1 })
}

function supportedPlaybackRate(rate) {
  return Number.isFinite(rate)
    && rate >= 0.5
    && rate <= 2
    && Math.abs(rate * 10 - Math.round(rate * 10)) < 0.000001
}

function emitPlaybackRate() {
  const rate = pick(() => music.playbackRate) ?? 1
  emit('playback-rate', { rate: supportedPlaybackRate(rate) ? rate : 1 })
}

function wireEvents() {
  on('playbackStateDidChange', () =>
    emit('playbackState', { state: stateName(pick(() => music.playbackState) ?? 0) }))

  on('nowPlayingItemDidChange', () =>
    emit('nowPlaying', {
      item: serializeItem(pick(() => music.nowPlayingItem)),
      queue: currentQueue('position'),
    }))

  on('playbackTimeDidChange', () =>
    emit('position', {
      positionMs: Math.round((pick(() => music.currentPlaybackTime) || 0) * 1000),
      durationMs: Math.round((pick(() => music.currentPlaybackDuration) || 0) * 1000),
    }))

  // Shuffle and repeat. Without these the Rust mirror never learns the mode,
  // so its toggle reads false forever and every click sends "on" again.
  on('shuffleModeDidChange', emitModes)
  on('repeatModeDidChange', emitModes)

  // Subscribed separately and reported separately. Collapsing them into one
  // event is what blinded the gapless check: see `currentQueue`.
  on('queueItemsDidChange', () => emit('queue', currentQueue('items')))
  on('queuePositionDidChange', () => emit('queue', currentQueue('position')))

  // **MusicKit owns the volume, and it remembers it.** Measured: mute, quit
  // fully, relaunch — `music.volume` reads 0 before we have sent anything, out
  // of the same session storage that keeps the login. Rust used to assume the
  // opposite and open at 1.0, so the bar showed full volume over silent audio
  // until the first keypress snapped the two together.
  on('playbackVolumeDidChange', () => emitVolume())
  on('playbackRateDidChange', () => emitPlaybackRate())

  on('authorizationStatusDidChange', () => {
    const session = pushSession()
    emit('authorization', { authorized: !!(session && session.authorized) })
  })

  // Cookie restoration can finish after the hook attaches. In that case the
  // account storefront changes without another authorization transition, and
  // failing to re-harvest it leaves the native client on the page's `/us/`
  // fallback for the rest of the run.
  on('storefrontCountryCodeDidChange', () => pushSession())
  on('authReflectionDidComplete', () => {
    const session = pushSession()
    // Main process consumes this locally; it is not forwarded to Rust. A
    // MusicKit object created before sign-in can retain preview-only playback
    // capability even after its token fields update. Rebuilding the document
    // once Apple's own auth reflection is complete creates the full player
    // immediately instead of making the first app restart do it by accident.
    emit('authorization-reflected', { authorized: !!(session && session.authorized) })
  })
}

// ---------------------------------------------------------------------------
// Commands from Rust
//
// Note the absence of any per-track play: rule 3 says MusicKit owns the queue.
// `setQueue` is sent ONCE with the whole list; moving within it is
// changeToMediaAtIndex, never a fresh setQueue.
// ---------------------------------------------------------------------------

// `playNext` and `playLater` are documented on MusicKit v3, but this page ships
// whichever version it likes (rule 4), so feature-detect and fail loudly rather
// than throwing a bare TypeError at somebody who clicked a menu item.
async function enqueue(method, songs) {
  if (typeof music[method] !== 'function') {
    throw new Error(`this MusicKit build has no ${method}`)
  }
  if (!Array.isArray(songs) || songs.length === 0) {
    throw new Error(`${method} called with no songs`)
  }

  const before = pick(() => music.queue?.items?.length) ?? 0
  await music[method]({ songs })
  const after = pick(() => music.queue?.items?.length) ?? 0

  // `queueItemsDidChange` does not fire for playNext/playLater in this
  // MusicKit build, so the mirror would keep showing the old queue and the
  // insert would look like it did nothing. Push it ourselves.
  emit('queue', currentQueue('items'))

  // And say so if the queue genuinely did not grow — silently doing nothing is
  // the failure this project keeps refusing to ship.
  if (after <= before) {
    throw new Error(`${method} did not change the queue (still ${after} items)`)
  }
}

/// Run a library write and treat an empty response body as success.
///
/// These endpoints answer `202 Accepted` with **no content**, and MusicKit's
/// client parses every response as JSON — so success arrives as
/// `SyntaxError: Unexpected end of JSON input`. Rethrowing that would report a
/// write that actually happened as a failure, which is how the first working
/// call looked broken.
///
/// Anything else is a real error and still throws, so `dispatch` reports it.
async function accepted(fn) {
  try {
    return await fn()
  } catch (err) {
    // A library write answers 202 with **no body**, and MusicKit's client parses
    // every response as JSON — so success arrives as a SyntaxError.
    //
    // Matched on the error *type* plus an empty body, not on message text. The
    // text alone would also swallow a genuine failure whose body happened to be
    // truncated or malformed, and report it as an accepted write — which is the
    // exact failure this whole path exists to stop being silent.
    const empty = !err || err.body === undefined || err.body === null || err.body === ''
    if (err instanceof SyntaxError && empty) return null
    throw err
  }
}

/// Run a library write and report its outcome **against the id it was for**.
///
/// `cmd-done` carries only the command name, and this dispatch is async, so two
/// removals can finish out of order — correlating by name lets one command's
/// completion be attributed to another's row. These carry the id so Rust can
/// match exactly.
async function libraryWrite(kind, id, fn) {
  try {
    await accepted(fn)
    emit('library-write', { kind, id, ok: true, detail: '' })
  } catch (err) {
    const detail = safeDetail((err && err.message) || err)
    emit('library-write', { kind, id, ok: false, detail })
  }
}

function validApiId(value) {
  if (typeof value !== 'string' || value.length === 0 || value.length > 512
      || !/^[A-Za-z0-9._~%-]+$/.test(value)) return false
  try {
    decodeURIComponent(value)
    return true
  } catch {
    return false
  }
}

function allowedApiRoute(method, pathname) {
  const parts = pathname.split('/').filter(Boolean)
  if (parts[0] !== 'v1') return false

  if (method === 'post') {
    return pathname === '/v1/me/library' || pathname === '/v1/me/favorites'
  }
  if (method !== 'get') return false

  if (parts[1] === 'me') {
    if (pathname === '/v1/me/recommendations'
        || pathname === '/v1/me/recent/played'
        || pathname === '/v1/me/recent/played/tracks'
        || pathname === '/v1/me/heavy-rotation') return true
    if (parts[2] !== 'library' || parts.length < 4 || parts.length > 6) return false
    const kind = parts[3]
    if (!['songs', 'albums', 'artists', 'playlists'].includes(kind)) return false
    if (parts.length === 4) return true
    if (!validApiId(parts[4])) return false
    if (parts.length === 5) return true
    return (kind === 'playlists' && parts[5] === 'tracks')
      || (kind === 'artists' && ['albums', 'catalog'].includes(parts[5]))
  }

  if (parts[1] !== 'catalog' || !/^[a-z]{2}$/.test(parts[2] || '')) return false
  if (parts.length === 4 && ['search', 'charts'].includes(parts[3])) return true
  const kind = parts[3]
  if (!['songs', 'albums', 'artists', 'playlists'].includes(kind)) return false
  if (parts.length === 4) return true
  if (!validApiId(parts[4])) return false
  if (parts.length === 5) return true
  if (kind === 'songs' && parts.length === 6 && parts[5] === 'syllable-lyrics') return true
  if (kind === 'playlists' && parts.length === 6 && parts[5] === 'tracks') return true
  return kind === 'artists'
    && parts.length === 7
    && parts[5] === 'view'
    && ['top-songs', 'latest-release'].includes(parts[6])
}

function normalizedApiPath(method, path) {
  if (typeof path !== 'string' || path.length === 0 || path.length > MAX_API_PATH_BYTES
      || !path.startsWith('/') || path.startsWith('//') || path.includes('\\')
      || path.includes('#') || /[\r\n\0]/.test(path)) return null
  let parsed
  try {
    parsed = new URL('https://api.music.apple.com/v1' + path)
  } catch {
    return null
  }
  if (parsed.origin !== 'https://api.music.apple.com'
      || !allowedApiRoute(method, parsed.pathname)) return null
  return parsed.pathname + parsed.search
}

function apiStatus(value) {
  const candidates = [value?.status, value?.statusCode, value?.response?.status]
  return candidates.find((status) => Number.isInteger(status) && status >= 100 && status <= 599)
}

function apiPayload(response) {
  if (response && typeof response === 'object'
      && Object.prototype.hasOwnProperty.call(response, 'data')) return response.data
  return response ?? null
}

function emitApiResponse(requestId, status, payload) {
  let body
  try {
    // A successful empty MusicKit POST can return `{ data: undefined }`.
    // JSON.stringify(undefined) is itself undefined, which the hardened event
    // boundary correctly refuses because `body` must be a string. Normalize
    // that one empty representation so the typed broker receives the status
    // instead of timing out after the command already completed.
    body = JSON.stringify(payload)
    if (body === undefined) body = 'null'
  } catch {
    body = ''
    status = 502
  }
  if (new TextEncoder().encode(body).length > MAX_API_BODY_BYTES) {
    body = ''
    status = 502
  }
  emit('api-response', { requestId, status, body })
}

function availableApiCall(method) {
  if (method === 'get') {
    const owner = typeof music.api?.v3?.music === 'function' ? music.api.v3 : music.api
    return typeof owner?.music === 'function' ? owner.music.bind(owner) : null
  }
  if (method === 'post') {
    return typeof music.api?.post === 'function' ? music.api.post.bind(music.api) : null
  }
  return null
}

async function waitForApiCall(method) {
  const deadline = Date.now() + API_METHOD_READY_TIMEOUT_MS
  while (true) {
    const call = availableApiCall(method)
    if (call) return call
    if (Date.now() >= deadline) return null
    await new Promise((resolve) => setTimeout(resolve, API_METHOD_READY_POLL_MS))
  }
}

async function browserApiRequest({ requestId, method, path }) {
  if (!Number.isSafeInteger(requestId) || requestId <= 0) {
    throw new Error('invalid broker request id')
  }
  const normalized = normalizedApiPath(method, path)
  if (!normalized) {
    emitApiResponse(requestId, 400, null)
    return
  }

  try {
    let response
    if (method === 'get') {
      const call = await waitForApiCall(method)
      if (!call) {
        emitApiResponse(requestId, 503, null)
        return
      }
      response = await call(normalized)
    } else if (method === 'post') {
      const call = await waitForApiCall(method)
      if (!call) {
        emitApiResponse(requestId, 503, null)
        return
      }
      response = await accepted(() => call(normalized))
    } else {
      emitApiResponse(requestId, 405, null)
      return
    }
    emitApiResponse(requestId, apiStatus(response) || (method === 'post' ? 202 : 200), apiPayload(response))
  } catch (err) {
    // Error bodies and exception messages can echo private queries. Rust needs
    // only the status class to choose its user-facing recovery path.
    emitApiResponse(requestId, apiStatus(err) || 502, null)
  }
}

function boundedUtf8(value, maximum, allowNewline = false) {
  return typeof value === 'string'
    && new TextEncoder().encode(value).length <= maximum
    && !value.includes('\0')
    && !value.includes('\r')
    && (allowNewline || !value.includes('\n'))
}

function validCatalogSongs(songs, allowEmpty) {
  return Array.isArray(songs)
    && songs.length <= 1000
    && (allowEmpty || songs.length > 0)
    && songs.every((id) => typeof id === 'string' && /^[0-9]{1,32}$/.test(id))
}

function validPlaylistId(id) {
  return typeof id === 'string'
    && id.length > 0
    && id.length <= 512
    && /^[A-Za-z0-9._-]+$/.test(id)
}

function songRelationships(songs) {
  return songs.map((id) => ({ id, type: 'songs' }))
}

async function playlistWrite(requestId, path, body) {
  if (!Number.isSafeInteger(requestId) || requestId <= 0) {
    throw new Error('invalid broker request id')
  }
  try {
    const call = await waitForApiCall('post')
    if (!call) {
      emitApiResponse(requestId, 503, null)
      return
    }
    const response = await accepted(() => call(path, body))
    emitApiResponse(requestId, apiStatus(response) || 202, apiPayload(response))
  } catch (err) {
    emitApiResponse(requestId, apiStatus(err) || 502, null)
  }
}

async function createPlaylist({ requestId, name, description = '', songs = [] }) {
  if (!boundedUtf8(name, 512) || name.trim().length === 0
      || !boundedUtf8(description, 4096, true)
      || !validCatalogSongs(songs, true)) {
    emitApiResponse(requestId, 400, null)
    return
  }
  const body = {
    attributes: { name, description },
    relationships: { tracks: { data: songRelationships(songs) } },
  }
  await playlistWrite(requestId, '/v1/me/library/playlists', body)
}

async function addPlaylistTracks({ requestId, playlistId, songs }) {
  if (!validPlaylistId(playlistId) || !validCatalogSongs(songs, false)) {
    emitApiResponse(requestId, 400, null)
    return
  }
  await playlistWrite(
    requestId,
    '/v1/me/library/playlists/' + encodeURIComponent(playlistId) + '/tracks',
    { data: songRelationships(songs) },
  )
}

const commands = {
  apiRequest: browserApiRequest,
  createPlaylist,
  addPlaylistTracks,
  async setQueue({ songs, startPosition = 0, startPlaying = true, startTimeMs = 0 }) {
    // BOTH keys, deliberately. MusicKit v3's setQueue forwards only
    // `startWith` to the queue descriptor:
    //
    //   startPlaying: e.startPlaying, startTime: e.startTime,
    //   startWith: e.startWith, context: e.context, ...
    //
    // so a lone `startPosition` is silently dropped and playback always begins
    // at index 0 — the queue is correct, just started in the wrong place.
    // Deeper down the descriptor does accept either (`startWith ?? startPosition`),
    // so sending both is harmless and survives whichever layer a future
    // MusicKit build hands the options to.
    // `startTime` is seconds, and it is how a restored queue comes back where
    // it was left. Seeking afterwards does not work while nothing is playing:
    // there is no current item to seek within.
    await music.setQueue({
      songs,
      startWith: startPosition,
      startPosition,
      startPlaying,
      startTime: startTimeMs / 1000,
    })
  },
  async playStation({ station }) {
    // A recommendation id is data, not a path. Bound it here as well as at the
    // typed Rust boundary so a changed Apple page cannot turn one click into a
    // giant queue descriptor.
    if (typeof station !== 'string' || station.length === 0 || station.length > 512) {
      throw new Error('invalid station id')
    }
    await music.setQueue({ station, startPlaying: true })
    if (!music.isPlaying) await music.play()
    // Station queues populate lazily and do not reliably emit an items event
    // immediately after setQueue, just like playNext/playLater.
    emit('queue', currentQueue('items'))
  },
  play: () => music.play(),
  pause: () => music.pause(),
  playPause: () => (music.isPlaying ? music.pause() : music.play()),
  next: () => music.skipToNextItem(),
  previous: () => music.skipToPreviousItem(),
  changeToIndex: ({ index }) => music.changeToMediaAtIndex(index),
  // Move one item within the queue MusicKit already holds.
  //
  // `splice` is undocumented — feature-detected rather than assumed, like
  // `remove` beside it. Its own source gives away the shape:
  //
  //     splice(e, n, d = []) {
  //       return toMediaItems(this.spliceQueueItems(e, n, toQueueItems(d)))
  //     }
  //
  // so it is `Array.prototype.splice` semantics, and the removed items come
  // back as media items ready to be handed straight to the insert.
  // Tell MusicKit where the current track ended up.
  //
  // **A splice does not re-index the position.** Measured: playing index 36,
  // two drags across it, and MusicKit still reports 36 — so `skipToNextItem`
  // advances 36 -> 37 and plays whatever now sits there, which is not the
  // track after the one playing. The queue looks right and playback follows a
  // number that no longer means anything.
  //
  // `position` has a real setter, and `_updatePosition` returns early when the
  // value is unchanged, so this is safe to send after every move.
  syncQueuePosition: ({ index }) => {
    const q = music.queue
    if (!q) throw new Error('no queue to reposition')
    const len = q.items?.length ?? 0
    if (!Number.isInteger(index) || index < 0 || index >= len) {
      throw new Error(`queue position ${index} out of range (queue holds ${len})`)
    }
    q.position = index
  },
  moveInQueue: ({ from, to }) => {
    if (typeof music.queue?.splice !== 'function') {
      throw new Error('this MusicKit build cannot reorder the queue')
    }
    const len = music.queue.items?.length ?? 0
    for (const [name, i] of [['from', from], ['to', to]]) {
      if (!Number.isInteger(i) || i < 0 || i >= len) {
        throw new Error(`queue index ${name}=${i} out of range (queue holds ${len})`)
      }
    }
    if (from === to) return
    const moved = music.queue.splice(from, 1)
    if (!moved || moved.length !== 1) {
      throw new Error(`splice removed ${moved ? moved.length : 0} items, expected 1`)
    }
    music.queue.splice(to, 0, moved)
  },
  removeFromQueue: ({ index }) => {
    // `queue.remove` is not in MusicKit's documented surface, so treat it as
    // load-bearing-but-unowned: check it exists rather than throwing a
    // TypeError at the user, and let the queue event report the real result.
    if (typeof music.queue?.remove !== 'function') {
      throw new Error('this MusicKit build cannot remove queue items')
    }
    // Bounds-check here too. MusicKit answers an out-of-range index with a bare
    // `[mk-007] INVALID_ARGUMENTS`, which says nothing about what was wrong;
    // this at least names the numbers.
    const len = music.queue.items?.length ?? 0
    if (!Number.isInteger(index) || index < 0 || index >= len) {
      throw new Error(`queue index ${index} out of range (queue holds ${len})`)
    }
    music.queue.remove(index)
  },
  // Insert into the queue MusicKit already holds, rather than rebuilding it.
  // A fresh setQueue would restart playback and throw away the gapless buffer,
  // which is the whole reason rule 3 exists — these two are the *only*
  // sanctioned way to grow a queue that is already playing.
  playNext: ({ songs }) => enqueue('playNext', songs),
  playLater: ({ songs }) => enqueue('playLater', songs),
  // Emptying the queue is not one documented call, so try the ways it might
  // be spelled and fall back to the one that always exists (rule 4). Stopping
  // first matters: an empty queue with a track still playing is a player in a
  // state nothing else expects.
  async clearQueue() {
    await music.stop()
    if (typeof music.clearQueue === 'function') {
      await music.clearQueue()
    } else if (typeof music.queue?.splice === 'function') {
      const len = music.queue.items?.length ?? 0
      music.queue.splice(0, len)
    } else {
      await music.setQueue({ songs: [] })
    }
    // `queueItemsDidChange` is not reliable for this either — same as
    // playNext/playLater.
    emit('queue', currentQueue('items'))
    const left = pick(() => music.queue?.items?.length) ?? 0
    if (left > 0) {
      throw new Error(`could not clear the queue (${left} items remain)`)
    }
  },
  seek: ({ positionMs }) => music.seekToTime(positionMs / 1000),
  setVolume: ({ volume }) => {
    music.volume = volume
  },
  setPlaybackRate: ({ rate }) => {
    if (!supportedPlaybackRate(rate)) throw new Error('unsupported playback rate')
    // The current element and the next track: MusicKit exposes both properties.
    music.defaultPlaybackRate = rate
    music.playbackRate = rate
    // Like shuffle/repeat, a programmatic change is not guaranteed to fire the
    // corresponding event in every MusicKit build, so echo the effective value.
    emitPlaybackRate()
  },
  setShuffle: ({ shuffle }) => {
    music.shuffleMode = shuffle ? 1 : 0
    // Echoed explicitly. MusicKit does not reliably fire
    // shuffleModeDidChange for a *programmatic* change, and a mode the Rust
    // side never hears about is a toggle that springs back.
    emitModes()
    // Turning shuffle off restores the queue's original order, so the queue
    // itself has changed even though no item was added or removed.
    emit('queue', currentQueue('items'))
  },
  setRepeat: ({ mode }) => {
    // MusicKit: 0 none, 1 one, 2 all
    music.repeatMode = mode === 'one' ? 1 : mode === 'all' ? 2 : 0
    emitModes()
  },
  // Removing things. **Only MusicKit can do these**, which is why they are here
  // and not in music/client.rs with their add counterparts.
  //
  // Verified against a real account: a direct REST probe of
  // `DELETE /v1/me/favorites?ids[songs]=…` answers `400 Insufficient
  // Permissions` and library removal has no documented endpoint at all. Issued
  // through MusicKit's own client, from the page and its session, both are
  // accepted. See issue #34 for the measurements.
  //
  // Two traps live in these four lines:
  //
  //   * `music.api.music(path, {}, {fetchOptions: {method: 'DELETE'}})`
  //     **silently performs a GET.** The verb helpers are the only way to send
  //     one, and a probe that uses the wrong one reports "Resource Not Found"
  //     for a route that exists.
  //   * Only the *per-resource* path works for the library. The collection
  //     forms fail the same way they do over REST: `?ids[songs]=` gives 405 and
  //     `/songs?ids=` gives 400. Favourites are the other way round — there it
  //     is the query form that works.
  removeFromLibrary: ({ id }) =>
    libraryWrite('remove', id, () =>
      music.api.delete('/v1/me/library/songs/' + encodeURIComponent(id))),
  unfavorite: ({ id }) =>
    libraryWrite('unfavorite', id, () =>
      music.api.delete('/v1/me/favorites?ids[songs]=' + encodeURIComponent(id))),

  authorize: () => music.authorize(),
  unauthorize: () => music.unauthorize(),
  refreshSession: () => pushSession(),
}

async function handleCommand(msg) {
  if (!msg || typeof msg !== 'object' || typeof msg.cmd !== 'string') return
  // Report arrival BEFORE doing anything. If Rust sends a command and no
  // `cmd-recv` comes back, the renderer never ran the handler at all — which
  // is a completely different problem from the command failing, and the two
  // are indistinguishable without this.
  emit('cmd-recv', { cmd: msg.cmd })

  const fn = commands[msg.cmd]
  if (!fn) return emit('error', { code: 'unknown-command', detail: msg.cmd })
  try {
    await fn(msg)
    // Always report completion, not just when an id was supplied: a command
    // that resolves without producing any MusicKit event is the signature of
    // playback being blocked rather than failing.
    emit('cmd-done', {
      cmd: msg.cmd,
      state: pick(() => music.playbackState) ?? -1,
      queueLen: pick(() => music.queue && music.queue.items && music.queue.items.length) ?? -1,
    })
  } catch (caught) {
    let err = caught
    // MusicKit deliberately throws CONTENT_EQUIVALENT when the catalogue
    // storefront and the signed-in account storefront differ. The Apple web
    // UI resolves that internally; direct MusicKit clients have to align the
    // storefront themselves. Repair only this exact, known failure and retry
    // the same command once — no generic retry loop and no skipped track.
    const reason = pick(() => err.reason, () => err.errorCode)
    const accountStorefront = String(pick(() => music.storefrontCountryCode) || '').toLowerCase()
    if (reason === 'CONTENT_EQUIVALENT'
      && /^[a-z]{2}$/.test(accountStorefront)
      && String(pick(() => music.storefrontId) || '').toLowerCase() !== accountStorefront) {
      try {
        music.storefrontId = accountStorefront
        await fn(msg)
        pushSession()
        emit('cmd-done', {
          cmd: msg.cmd,
          state: pick(() => music.playbackState) ?? -1,
          queueLen: pick(() => music.queue && music.queue.items && music.queue.items.length) ?? -1,
        })
        return
      } catch (retryError) {
        err = retryError
      }
    }
    emit('error', {
      code: 'command-failed',
      detail: safeDetail(`${msg.cmd}: ${err && err.message}`),
    })
  }
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

// Wiring is triggered by the MAIN process, not by a timer in here.
//
// A window created with show:false has its renderer frozen by Chromium:
// setTimeout fires once or twice and then stops for good. Neither
// webPreferences.backgroundThrottling nor the --disable-renderer-backgrounding
// family prevents it. The old self-polling loop emitted exactly one tick and
// then went silent for 90 seconds — no hook-ready and no hook-failed, because
// the loop that would have reported either had itself frozen.
//
// So main.js polls window.__slipmatReady() over executeJavaScript, which runs
// regardless of renderer timer state, and sends `slipmat:wire` when MusicKit is
// up. Everything after that point is event-driven, and Chromium does not freeze
// a page that is playing audio — so once playback starts the renderer stays
// awake on its own.
function wire(trigger) {
  if (music) return true // already wired; a duplicate trigger is harmless
  music = getInstance()
  if (!music) return false

  wireEvents()

  // Authorization can settle a beat after MusicKit itself. main.js also
  // re-sends `refreshSession` on a main-process timer, because a renderer
  // timer cannot be relied on here.
  pushSession()

  emit('hook-ready', {
    trigger,
    authorized: !!pick(() => music.isAuthorized),
    version: pick(() => window.MusicKit.version) || 'unknown',
  })

  // **And say what the queue is now.** Rust mirrors the queue and only ever
  // learns of a change from an event, so a queue that goes away without one is
  // a queue Rust keeps believing in.
  //
  // That is exactly what signing out does: the session is cleared, the page
  // reloads, and this preload context is replaced — MusicKit comes back with an
  // empty queue and nothing said so. Rust went on holding the old 519 items,
  // decided a click was "already loaded", and sent `changeToIndex` into a queue
  // that no longer existed. Silent, and wedged until the app restarted (#130).
  //
  // Reported on every wire rather than only after a sign-out, because every
  // path that replaces this context has the same hole: a cross-document
  // navigation, a sidecar restart. Whatever the queue is at the moment the hook
  // attaches, that is the truth, and an empty one is a fact rather than an
  // absence of news.
  emit('queue', currentQueue('items'))

  // Same reasoning one line up, for the same class of fact: whatever the volume
  // is when the hook attaches is the truth, and Rust has no other way to learn
  // it. This is what makes a fresh launch — and a supervised restart — agree
  // with what you can hear.
  emitVolume()
  emitPlaybackRate()
  return true
}

// Two independent triggers, because neither is reliable alone:
//
//   1. The renderer self-poll below. Works when the page is live, and is what
//      succeeds on a normal desktop session.
//   2. main.js probing window.__slipmatReady() over executeJavaScript, which
//      keeps working in situations where the renderer's own timers stall.
//
// Whichever wins calls wire(); the `if (music) return` guard makes the loser a
// no-op. Belt and braces on purpose — this handshake failing silently is the
// worst failure mode the sidecar has.
function handleWire() {
  if (!wire('main-probe') && !music) {
    emit('hook-failed', { detail: 'MusicKit vanished between probe and wire' })
  }
}

window.addEventListener('message', (event) => {
  if (event.source !== window || event.origin !== location.origin) return
  const message = event.data
  if (!message || message.source !== 'slipmat-preload') return
  if (message.type === 'wire') {
    handleWire()
  } else if (message.type === 'command') {
    handleCommand(message.payload)
  }
})

function selfPoll() {
  const deadline = Date.now() + READY_TIMEOUT_MS
  const tick = () => {
    if (wire('self-poll')) return
    if (Date.now() > deadline) return // main.js owns the timeout report
    setTimeout(tick, READY_POLL_MS)
  }
  tick()
}

// Guard on readyState: the preload usually runs before the document parses, but
// on a warm cache it can already be past `loading`, and then DOMContentLoaded
// never fires again.
if (document.readyState === 'loading') {
  window.addEventListener('DOMContentLoaded', selfPoll, { once: true })
} else {
  selfPoll()
}
})()
