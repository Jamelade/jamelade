// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')
const {
  isAllowedNetworkUrl,
  isAllowedRendererEvent,
  isTrustedAppleUrl,
  mayPersistCookies,
  runtimeIdentity,
  safeErrorDetail,
} = require('./security')

test('runtime identity accepts only the fixed broker-test profile', () => {
  assert.equal(runtimeIdentity('broker-test').appName, 'Jamelade Broker Test')
  assert.equal(runtimeIdentity('broker-test').partition, 'jamelade-broker-test-memory')
  assert.equal(runtimeIdentity('BROKER-TEST').appName, 'Jamelade')
  assert.equal(runtimeIdentity('../broker-test').profileName, 'Jamelade')
  assert.ok(Object.isFrozen(runtimeIdentity('broker-test')))
})

test('Apple navigation requires HTTPS and a real domain boundary', () => {
  assert.equal(isTrustedAppleUrl('https://music.apple.com/us/browse'), true)
  assert.equal(isTrustedAppleUrl('https://idmsa.apple.com/appleauth/auth/signin'), true)
  assert.equal(isTrustedAppleUrl('https://is1-ssl.mzstatic.com/image/thumb/x'), true)
  assert.equal(isTrustedAppleUrl('https://evilapple.com/'), false)
  assert.equal(isTrustedAppleUrl('https://apple.com.evil.example/'), false)
  assert.equal(isTrustedAppleUrl('https://apple.com@evil.example/'), false)
  assert.equal(isTrustedAppleUrl('http://music.apple.com/'), false)
  assert.equal(isTrustedAppleUrl('javascript:alert(1)'), false)
})

test('the credential-bearing session can contact only Apple service domains', () => {
  assert.equal(isAllowedNetworkUrl('https://music.apple.com/us/listen-now'), true)
  assert.equal(isAllowedNetworkUrl('https://audio.applemusic.com/stream/segment'), true)
  assert.equal(isAllowedNetworkUrl('https://is1-ssl.mzstatic.com/image/thumb/x'), true)
  assert.equal(isAllowedNetworkUrl('https://appleid.cdn-apple.com/appleauth/static/jsapi/'), true)
  assert.equal(isAllowedNetworkUrl('wss://music.apple.com/socket'), true)
  assert.equal(isAllowedNetworkUrl('https://example.com/collect'), false)
  assert.equal(isAllowedNetworkUrl('https://apple.com.evil.example/'), false)
  assert.equal(isAllowedNetworkUrl('https://applemusic.com.evil.example/'), false)
  assert.equal(isAllowedNetworkUrl('https://user@music.apple.com/'), false)
  assert.equal(isAllowedNetworkUrl('http://music.apple.com/'), false)
  assert.equal(isAllowedNetworkUrl('file:///etc/passwd'), false)
})

test('session events are credential-free and structurally checked', () => {
  const event = {
    event: 'session',
    storefront: 'us',
    authorized: true,
    hasUserToken: true,
  }
  assert.equal(isAllowedRendererEvent(event), true)
  assert.equal(isAllowedRendererEvent({ ...event, storefront: '../' }), false)
  assert.equal(isAllowedRendererEvent({ ...event, hasUserToken: 'yes' }), false)
  assert.equal(isAllowedRendererEvent({ ...event, developerToken: 'must-not-cross' }), false)
  assert.equal(isAllowedRendererEvent({ event: 'not-allowed' }), false)
  assert.equal(isAllowedRendererEvent({ event: 'error', detail: 'x'.repeat(4097) }), false)
  assert.equal(isAllowedRendererEvent({ event: 'position', position: Number.NaN }), false)
  assert.equal(isAllowedRendererEvent({ event: 'cmd-recv', cmd: '../bad' }), false)
  assert.equal(isAllowedRendererEvent({
    event: 'hook-ready',
    authorized: true,
    trigger: 'main-probe',
    version: 'test',
    musicUserToken: 'must-not-cross',
  }), false)
  assert.equal(isAllowedRendererEvent({
    event: 'hook-boot',
    readyState: 'complete',
    href: 'https://music.apple.com/us/album/private/123',
  }), false)
  assert.equal(isAllowedRendererEvent({
    event: 'authorization-reflected',
    authorized: true,
  }), true)
  assert.equal(isAllowedRendererEvent({ event: 'authorization-reflected' }), false)
  assert.equal(isAllowedRendererEvent({ event: 'playback-rate', rate: 0.5 }), true)
  assert.equal(isAllowedRendererEvent({ event: 'playback-rate', rate: 1.7 }), true)
  assert.equal(isAllowedRendererEvent({ event: 'playback-rate', rate: 1.75 }), false)
  assert.equal(isAllowedRendererEvent({ event: 'playback-rate', rate: 9 }), false)
  const queue = { event: 'queue', reason: 'items', position: 0, items: [] }
  assert.equal(isAllowedRendererEvent(queue), true)
  assert.equal(isAllowedRendererEvent({ ...queue, token: 'must-not-cross' }), false)
  assert.equal(isAllowedRendererEvent({ ...queue, items: Array(4096).fill(null) }), true)
  assert.equal(isAllowedRendererEvent({ ...queue, items: Array(4097).fill(null) }), false)
})

test('browser broker responses are bounded and have no extra capability fields', () => {
  const response = {
    event: 'api-response',
    requestId: 9,
    status: 200,
    body: '{"data":[]}',
  }
  assert.equal(isAllowedRendererEvent(response), true)
  assert.equal(isAllowedRendererEvent({ ...response, requestId: 0 }), false)
  assert.equal(isAllowedRendererEvent({ ...response, status: 999 }), false)
  assert.equal(isAllowedRendererEvent({ ...response, url: 'https://example.com' }), false)
  assert.equal(isAllowedRendererEvent({
    ...response,
    body: 'x'.repeat(3 * 1024 * 1024 + 1),
  }), false)
})

test('error details redact credential-shaped values', () => {
  const jwt = `eyJ${'a'.repeat(30)}.${'b'.repeat(30)}.${'c'.repeat(30)}`
  const detail = safeErrorDetail(`failed with ${jwt}\nand ${'x'.repeat(90)}`)
  assert.equal(detail.includes(jwt), false)
  assert.equal(detail.includes('x'.repeat(90)), false)
  assert.equal(detail.includes('\n'), false)

  const headers = safeErrorDetail(
    `Authorization: Bearer ${'a'.repeat(48)} Cookie: media-user-token=${'b'.repeat(48)}`,
  )
  assert.equal(headers.includes('a'.repeat(48)), false)
  assert.equal(headers.includes('b'.repeat(48)), false)

  const query = safeErrorDetail(`https://example.test/?token=${'c'.repeat(48)}&ok=1`)
  assert.equal(query.includes('c'.repeat(48)), false)

  const path = safeErrorDetail('/home/private-name/.config/Jamelade/file failed')
  assert.equal(path.includes('private-name'), false)
})

test('Linux persistence requires a real Secret Service backend', () => {
  if (process.platform !== 'linux') return
  assert.equal(mayPersistCookies(true, 'gnome_libsecret'), true)
  assert.equal(mayPersistCookies(true, 'kwallet6'), true)
  assert.equal(mayPersistCookies(true, 'basic_text'), false)
  assert.equal(mayPersistCookies(false, 'gnome_libsecret'), false)
})
