// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

const MAX_URL_LENGTH = 4096
const MAX_PROTOCOL_BYTES = 4 * 1024 * 1024
const MAX_API_BODY_BYTES = 3 * 1024 * 1024
const MAX_ERROR_LENGTH = 512
const MAX_EVENT_STRING = 4096
const MAX_EVENT_DEPTH = 8
const MAX_EVENT_NODES = 50_000
const MAX_EVENT_ARRAY = 4096
const MAX_EVENT_KEYS = 256

const STABLE_IDENTITY = Object.freeze({
  appName: 'Jamelade',
  profileName: 'Jamelade',
  desktopName: 'io.github.Jamelade.Jamelade.Launcher.desktop',
  partition: 'jamelade-hardened-memory',
})
const BROKER_TEST_IDENTITY = Object.freeze({
  appName: 'Jamelade Broker Test',
  profileName: 'JameladeBrokerTest',
  desktopName: 'io.github.Jamelade.Jamelade.BrokerTest.Launcher.desktop',
  partition: 'jamelade-broker-test-memory',
})

function runtimeIdentity(value) {
  return value === 'broker-test' ? BROKER_TEST_IDENTITY : STABLE_IDENTITY
}

// Remote content runs with access to Apple credentials.  Flatpak can grant or
// deny networking but cannot express a hostname allowlist, so the Electron
// session enforces the missing half.  These are Apple-operated service/CDN
// domains used by MusicKit, authentication and artwork; domain-boundary checks
// keep lookalikes such as apple.com.evil.example out.
const APPLE_NETWORK_DOMAINS = Object.freeze([
  'apple.com',
  // Apple Music's own CSP names *.applemusic.com as a media source. Some
  // catalogue items are delivered there rather than from *.itunes.apple.com,
  // so omitting it made playback depend on which CDN held a particular song.
  'applemusic.com',
  'mzstatic.com',
  'cdn-apple.com',
  'icloud.com',
  'icloud-content.com',
  'apple-dns.net',
  'itunes.com',
])

const RENDERER_EVENTS = new Set([
  'api-response',
  'authorization',
  'authorization-reflected',
  'cmd-done',
  'cmd-recv',
  'error',
  'hook-boot',
  'hook-failed',
  'hook-ready',
  'hook-warning',
  'library-write',
  'modes',
  'nowPlaying',
  'playback-rate',
  'playbackState',
  'position',
  'queue',
  'session',
  'volume',
])

// Keep this in lockstep with preload.js. The main process repeats the check so
// a preload regression cannot turn ignored JSON fields into a covert path from
// the credential-bearing page to native code.
const RENDERER_EVENT_KEYS = Object.freeze({
  'api-response': ['body', 'event', 'requestId', 'status'],
  authorization: ['authorized', 'event'],
  'authorization-reflected': ['authorized', 'event'],
  'cmd-done': ['cmd', 'event', 'queueLen', 'state'],
  'cmd-recv': ['cmd', 'event'],
  error: ['code', 'detail', 'event'],
  'hook-boot': ['event', 'readyState'],
  'hook-failed': ['detail', 'event'],
  'hook-ready': ['authorized', 'event', 'trigger', 'version'],
  'hook-warning': ['detail', 'event'],
  'library-write': ['detail', 'event', 'id', 'kind', 'ok'],
  modes: ['event', 'repeat', 'shuffle'],
  nowPlaying: ['event', 'item', 'queue'],
  'playback-rate': ['event', 'rate'],
  playbackState: ['event', 'state'],
  position: ['durationMs', 'event', 'positionMs'],
  queue: ['event', 'items', 'position', 'reason'],
  session: ['authorized', 'event', 'hasUserToken', 'storefront'],
  volume: ['event', 'volume'],
})

function hasExactKeys(value, expected) {
  const actual = Object.keys(value).sort()
  return actual.length === expected.length
    && actual.every((key, index) => key === expected[index])
}

function hasDomainBoundary(hostname, domain) {
  return hostname === domain || hostname.endsWith(`.${domain}`)
}

function parseBoundedUrl(raw) {
  if (typeof raw !== 'string' || raw.length === 0 || raw.length > MAX_URL_LENGTH) {
    return null
  }
  try {
    return new URL(raw)
  } catch {
    return null
  }
}

/** Main-frame destinations that may remain inside the privileged player. */
function isTrustedAppleUrl(raw) {
  const parsed = parseBoundedUrl(raw)
  if (!parsed
    || parsed.protocol !== 'https:'
    || parsed.username
    || parsed.password
    || (parsed.port && parsed.port !== '443')) return false
  return hasDomainBoundary(parsed.hostname, 'apple.com')
    || hasDomainBoundary(parsed.hostname, 'mzstatic.com')
}

/** Subresources the credential-bearing Apple session may contact. */
function isAllowedNetworkUrl(raw) {
  const parsed = parseBoundedUrl(raw)
  if (!parsed
    || (parsed.protocol !== 'https:' && parsed.protocol !== 'wss:')
    || parsed.username
    || parsed.password
    || (parsed.port && parsed.port !== '443')) return false
  return APPLE_NETWORK_DOMAINS.some((domain) => hasDomainBoundary(parsed.hostname, domain))
}

function serializedSize(value) {
  try {
    return Buffer.byteLength(JSON.stringify(value), 'utf8')
  } catch {
    return Number.POSITIVE_INFINITY
  }
}

function isPlainObject(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
}

/** Reject pathological structured-clone payloads before JSON/Rust sees them. */
function isBoundedJson(
  value,
  maximumString = MAX_EVENT_STRING,
  state = { nodes: 0 },
  depth = 0,
) {
  state.nodes += 1
  if (state.nodes > MAX_EVENT_NODES || depth > MAX_EVENT_DEPTH) return false
  if (value === null || typeof value === 'boolean') return true
  if (typeof value === 'number') return Number.isFinite(value)
  if (typeof value === 'string') return value.length <= maximumString
  if (Array.isArray(value)) {
    return value.length <= MAX_EVENT_ARRAY
      && value.every((entry) => isBoundedJson(entry, maximumString, state, depth + 1))
  }
  if (!isPlainObject(value)) return false
  const entries = Object.entries(value)
  return entries.length <= MAX_EVENT_KEYS
    && entries.every(([key, entry]) => key.length <= 128
      && isBoundedJson(entry, maximumString, state, depth + 1))
}

/**
 * Treat the remote page as untrusted even though it is Apple-owned. This is a
 * deliberately small structural check at the Electron boundary; Rust performs
 * the typed parse on the other side of the pipe.
 */
function isAllowedRendererEvent(value) {
  if (!isPlainObject(value)
    || !isBoundedJson(
      value,
      value.event === 'api-response' ? MAX_API_BODY_BYTES : MAX_EVENT_STRING,
    )) {
    return false
  }
  if (!RENDERER_EVENTS.has(value.event)) return false
  if (!hasExactKeys(value, RENDERER_EVENT_KEYS[value.event])) return false
  if (serializedSize(value) > MAX_PROTOCOL_BYTES) return false

  if (value.event === 'session') {
    const keys = Object.keys(value)
    if (!keys.every((key) => ['event', 'storefront', 'authorized', 'hasUserToken'].includes(key))) {
      return false
    }
    if (typeof value.storefront !== 'string' || !/^[a-z]{2}$/.test(value.storefront)) {
      return false
    }
    if (typeof value.authorized !== 'boolean'
      || typeof value.hasUserToken !== 'boolean') return false
  }
  if (value.event === 'api-response') {
    const keys = Object.keys(value)
    if (!keys.every((key) => ['event', 'requestId', 'status', 'body'].includes(key))
      || !Number.isSafeInteger(value.requestId) || value.requestId <= 0
      || !Number.isInteger(value.status) || value.status < 100 || value.status > 599
      || typeof value.body !== 'string') return false
  }
  if (value.event === 'authorization-reflected'
    && typeof value.authorized !== 'boolean') return false
  if (value.event === 'playback-rate'
    && (typeof value.rate !== 'number'
      || !Number.isFinite(value.rate)
      || value.rate < 0.5
      || value.rate > 2
      || Math.abs(value.rate * 10 - Math.round(value.rate * 10)) >= 0.000001)) return false

  if ('detail' in value && typeof value.detail !== 'string') return false
  if ('code' in value
    && (typeof value.code !== 'string' || !/^[A-Za-z0-9_-]{1,80}$/.test(value.code))) {
    return false
  }
  if ('cmd' in value
    && (typeof value.cmd !== 'string' || !/^[A-Za-z][A-Za-z0-9_-]{0,63}$/.test(value.cmd))) {
    return false
  }

  return true
}

/** Strip credential-shaped strings before an exception reaches logs or Rust. */
function safeErrorDetail(value) {
  let text = String(value ?? '')
  text = text
    .replace(/\b(?:cookie|set-cookie)\s*[:=][^\r\n]*/gi, '<redacted-cookie-header>')
    .replace(/\b(authorization|music-user-token|media-user-token)\s*[:=]\s*[^\s,;]+/gi, '$1: <redacted>')
    .replace(/([?&](?:access_token|auth|authorization|cookie|key|media-user-token|music-user-token|password|session|token)=)[^&#\s]*/gi, '$1<redacted>')
    .replace(/eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g, '<redacted-jwt>')
    .replace(/[A-Za-z0-9+/_=-]{32,}/g, '<redacted-secret>')
    .replace(/\/(?:home|Users)\/[^/\s]+/g, '/<user-home>')
    .replace(/[A-Za-z]:\\Users\\[^\\\s]+/g, '<user-home>')
    .replace(/[\r\n\t]+/g, ' ')
    .trim()
  return text.slice(0, MAX_ERROR_LENGTH)
}

function mayPersistCookies(encryptionAvailable, backend) {
  if (!encryptionAvailable) return false
  if (process.platform !== 'linux') return true
  return backend === 'gnome_libsecret'
    || backend === 'kwallet'
    || backend === 'kwallet5'
    || backend === 'kwallet6'
}

module.exports = {
  MAX_PROTOCOL_BYTES,
  isAllowedNetworkUrl,
  isAllowedRendererEvent,
  isTrustedAppleUrl,
  mayPersistCookies,
  runtimeIdentity,
  safeErrorDetail,
  serializedSize,
}
