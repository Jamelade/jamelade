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
  const apiRequests = []
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
    api: {
      v3: {
        async music(path) {
          apiRequests.push({ method: 'get', path })
          return { status: 200, data: { data: [{ id: '123', type: 'albums' }] } }
        },
      },
      async post(path, body) {
        apiRequests.push({ method: 'post', path, ...(body === undefined ? {} : { body }) })
        return { status: 202, data: null }
      },
    },
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
    TextEncoder,
    URL,
  }))

  return {
    events,
    apiRequests,
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

test('the browser broker permits named Apple routes and refuses arbitrary URLs', async () => {
  const app = harness()
  app.send({ cmd: 'apiRequest', requestId: 7, method: 'get', path: '/catalog/de/albums/123?include=tracks' })
  app.send({ cmd: 'apiRequest', requestId: 9, method: 'get', path: '/catalog/de/artists/1234567890?include=albums&extend=editorialNotes' })
  app.send({ cmd: 'apiRequest', requestId: 8, method: 'get', path: '//example.com/collect' })
  await new Promise((resolve) => setImmediate(resolve))

  // Bodies originate in the VM realm; round-trip them so prototype identity
  // cannot make structurally identical payloads compare unequal.
  assert.deepEqual(JSON.parse(JSON.stringify(app.apiRequests)), [
    {
      method: 'get',
      path: '/v1/catalog/de/albums/123?include=tracks',
    },
    {
      method: 'get',
      path: '/v1/catalog/de/artists/1234567890?include=albums&extend=editorialNotes',
    },
  ])
  const accepted = app.events.find((event) => event.event === 'api-response' && event.requestId === 7)
  assert.equal(accepted.status, 200)
  assert.deepEqual(JSON.parse(accepted.body), { data: [{ id: '123', type: 'albums' }] })
  const refused = app.events.find((event) => event.event === 'api-response' && event.requestId === 8)
  assert.equal(refused.status, 400)
})

test('the browser broker waits for MusicKit API startup instead of failing the library', async () => {
  const app = harness()
  const music = app.music.api.v3.music
  delete app.music.api.v3.music

  app.send({
    cmd: 'apiRequest',
    requestId: 10,
    method: 'get',
    path: '/me/library/songs?limit=100&offset=0',
  })
  setTimeout(() => {
    app.music.api.v3.music = music
  }, 40)
  await new Promise((resolve) => setTimeout(resolve, 180))

  assert.deepEqual(app.apiRequests, [{
    method: 'get',
    path: '/v1/me/library/songs?limit=100&offset=0',
  }])
  const response = app.events.find(
    (event) => event.event === 'api-response' && event.requestId === 10,
  )
  assert.equal(response.status, 200)
})

test('playlist writes are typed, bounded, and remain on documented Apple routes', async () => {
  const app = harness()
  app.send({
    cmd: 'createPlaylist',
    requestId: 20,
    name: 'Road trip',
    description: 'Example',
    songs: ['1000000001'],
  })
  app.send({
    cmd: 'addPlaylistTracks',
    requestId: 21,
    playlistId: 'p.example',
    songs: ['1000000002'],
  })
  app.send({
    cmd: 'addPlaylistTracks',
    requestId: 22,
    playlistId: '../../outside',
    songs: ['1000000003'],
  })
  await new Promise((resolve) => setImmediate(resolve))

  assert.deepEqual(JSON.parse(JSON.stringify(app.apiRequests)), [
    {
      method: 'post',
      path: '/v1/me/library/playlists',
      body: {
        attributes: { name: 'Road trip', description: 'Example' },
        relationships: { tracks: { data: [{ id: '1000000001', type: 'songs' }] } },
      },
    },
    {
      method: 'post',
      path: '/v1/me/library/playlists/p.example/tracks',
      body: { data: [{ id: '1000000002', type: 'songs' }] },
    },
  ])
  assert.equal(
    app.events.find((event) => event.event === 'api-response' && event.requestId === 22).status,
    400,
  )
})

test('the signed-in account storefront wins over the page storefront', () => {
  const { events, music } = harness()
  assert.equal(music.storefrontId, 'de')
  const session = events.find((event) => event.event === 'session')
  assert.deepEqual(session, {
    event: 'session',
    storefront: 'de',
    authorized: true,
    hasUserToken: true,
  })
  assert.equal('developerToken' in session, false)
  assert.equal('musicUserToken' in session, false)
})

test('a storefront arriving after cookie reflection is re-harvested', () => {
  const { events, music, musicListeners } = harness()
  music.storefrontCountryCode = 'fr'
  musicListeners.get('storefrontCountryCodeDidChange')()

  assert.equal(music.storefrontId, 'fr')
  assert.equal(events.filter((event) => event.event === 'session').at(-1).storefront, 'fr')
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
