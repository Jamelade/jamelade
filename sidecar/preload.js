// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

// This preload runs in Electron's isolated, sandboxed world. The Apple page
// receives one capability only: sending a fixed set of bounded player events.
// It never receives ipcRenderer, Node.js, filesystem, shell or process access.
const { contextBridge, ipcRenderer } = require('electron')

const MAX_EVENT_BYTES = 4 * 1024 * 1024
const MAX_TOKEN_LENGTH = 16 * 1024
const MAX_EVENT_STRING = 4096
const MAX_EVENT_DEPTH = 8
const MAX_EVENT_NODES = 50_000
const MAX_EVENT_ARRAY = 4096
const MAX_EVENT_KEYS = 256
const MAX_EVENTS_PER_SECOND = 60
const EVENTS = new Set([
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
  'playbackState',
  'position',
  'queue',
  'tokens',
  'volume',
])

function validToken(value, minimum) {
  return typeof value === 'string'
    && value.length >= minimum
    && value.length <= MAX_TOKEN_LENGTH
    && value === value.trim()
    && !/[\r\n]/.test(value)
}

function isPlainObject(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
}

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

function allowed(event, payload) {
  if (!EVENTS.has(event)
    || !isPlainObject(payload)
    || !isBoundedJson(payload, event === 'tokens' ? MAX_TOKEN_LENGTH : MAX_EVENT_STRING)) {
    return false
  }
  if (event === 'tokens') {
    if (!validToken(payload.developerToken, 32)) return false
    if (payload.musicUserToken !== null
      && payload.musicUserToken !== undefined
      && !validToken(payload.musicUserToken, 32)) return false
    if (typeof payload.storefront !== 'string' || !/^[a-z]{2}$/.test(payload.storefront)) {
      return false
    }
    if (typeof payload.authorized !== 'boolean') return false
  }
  if (event === 'authorization-reflected'
    && typeof payload.authorized !== 'boolean') return false
  if ('detail' in payload && typeof payload.detail !== 'string') return false
  if ('code' in payload
    && (typeof payload.code !== 'string'
      || !/^[A-Za-z0-9_-]{1,80}$/.test(payload.code))) return false
  if ('cmd' in payload
    && (typeof payload.cmd !== 'string'
      || !/^[A-Za-z][A-Za-z0-9_-]{0,63}$/.test(payload.cmd))) return false
  try {
    return new TextEncoder().encode(JSON.stringify({ event, ...payload })).length <= MAX_EVENT_BYTES
  } catch {
    return false
  }
}

let rateWindow = { started: Date.now(), count: 0 }

function withinRate() {
  const now = Date.now()
  if (now - rateWindow.started >= 1000) {
    rateWindow = { started: now, count: 0 }
  }
  rateWindow.count += 1
  return rateWindow.count <= MAX_EVENTS_PER_SECOND
}

const PLAYER_ORIGIN = location.protocol === 'https:' && location.hostname === 'music.apple.com'

if (PLAYER_ORIGIN) {
  contextBridge.exposeInMainWorld('slipmatBridge', Object.freeze({
    emit(event, payload = {}) {
      if (allowed(event, payload) && withinRate()) {
        ipcRenderer.send('slipmat:event', { event, ...payload })
      }
    },
  }))
}

function postToPage(type, payload = null) {
  if (!PLAYER_ORIGIN) return
  window.postMessage({ source: 'slipmat-preload', type, payload }, location.origin)
}

ipcRenderer.on('slipmat:command', (_event, command) => {
  postToPage('command', command)
})

ipcRenderer.on('slipmat:wire', () => {
  postToPage('wire')
})
