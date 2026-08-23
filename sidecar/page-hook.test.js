// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')

const source = fs.readFileSync(path.join(__dirname, 'page-hook.js'), 'utf8')

function harness() {
  const events = []
  const windowListeners = new Map()
  const musicListeners = new Map()
  const music = {
    developerToken: `eyJ${'d'.repeat(64)}`,
    musicUserToken: 'u'.repeat(64),
    isAuthorized: true,
    // The exact mismatch seen when Apple's root page redirects to /us/ while
    // the signed-in account belongs to another storefront.
    storefrontId: 'us',
    storefrontCountryCode: 'de',
    playbackState: 3,
    queue: { items: [], position: 0 },
    volume: 1,
    shuffleMode: 0,
    repeatMode: 0,
    addEventListener(name, callback) {
      musicListeners.set(name, callback)
    },
    async setQueue() {},
  }
  const location = {
    origin: 'https://music.apple.com',
    pathname: '/us/new',
  }
  const window = {
    MusicKit: {
      version: 'test',
      getInstance: () => music,
    },
    slipmatBridge: {
      emit(event, payload) {
        events.push({ event, ...payload })
      },
    },
    addEventListener(name, callback) {
      windowListeners.set(name, callback)
    },
  }
  window.window = window

  vm.runInContext(source, vm.createContext({
    window,
    location,
    document: { readyState: 'complete' },
    setTimeout,
    clearTimeout,
  }))

  return {
    events,
    music,
    musicListeners,
    send(command) {
      windowListeners.get('message')({
        source: window,
        origin: location.origin,
        data: {
          source: 'slipmat-preload',
          type: 'command',
          payload: command,
        },
      })
    },
  }
}

test('the signed-in account storefront wins over the page storefront', () => {
  const { events, music } = harness()
  assert.equal(music.storefrontId, 'de')
  assert.equal(events.find((event) => event.event === 'tokens').storefront, 'de')
})

test('a storefront arriving after cookie reflection is re-harvested', () => {
  const { events, music, musicListeners } = harness()
  music.storefrontCountryCode = 'fr'
  musicListeners.get('storefrontCountryCodeDidChange')()

  assert.equal(music.storefrontId, 'fr')
  assert.equal(events.filter((event) => event.event === 'tokens').at(-1).storefront, 'fr')
})

test('completed auth reflection requests one credential-free player refresh', () => {
  const { events, musicListeners } = harness()
  musicListeners.get('authReflectionDidComplete')()

  const reflected = events.filter((event) => event.event === 'authorization-reflected')
  assert.deepEqual(reflected, [{ event: 'authorization-reflected', authorized: true }])
})

test('a storefront mismatch is repaired and retried exactly once', async () => {
  const app = harness()
  app.music.storefrontId = 'us'
  let attempts = 0
  app.music.setQueue = async () => {
    attempts += 1
    if (attempts === 1) {
      const error = new Error('[mk-007] CONTENT_EQUIVALENT')
      error.reason = 'CONTENT_EQUIVALENT'
      throw error
    }
  }
  const before = app.events.length

  app.send({ cmd: 'setQueue', songs: ['1440857781'], startPosition: 0 })
  await new Promise((resolve) => setImmediate(resolve))

  const commandEvents = app.events.slice(before)
  assert.equal(attempts, 2)
  assert.equal(app.music.storefrontId, 'de')
  assert.equal(commandEvents.some((event) => event.event === 'cmd-done'), true)
  assert.equal(commandEvents.some((event) => event.event === 'error'), false)
})
