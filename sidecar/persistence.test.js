// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { EventEmitter } = require('node:events')
const { createPersistence } = require('./persistence')

function store() {
  const cookies = new EventEmitter()
  cookies.get = async () => [{
    name: 'session',
    value: 'fresh-login',
    domain: '.apple.com',
    hostOnly: false,
    path: '/',
    secure: true,
    httpOnly: true,
    session: false,
    sameSite: 'no_restriction',
    expirationDate: 2_000_000_000,
  }]
  cookies.set = async () => {}
  return cookies
}

function persistence(dir, safeStorage, cookieStore, events, blocked = () => {}) {
  return createPersistence({
    app: { getPath: () => dir },
    cookieStore,
    safeStorage,
    mayPersistCookies: (available, backend) => available && backend === 'kwallet6',
    send: (event) => events.push(event),
    log: () => {},
    blocked,
    retryDelays: [],
    restoreDelays: [],
  })
}

test('an unavailable boot keyring leaves the saved vault untouched', async (t) => {
  const dir = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'jamelade-persistence-'))
  t.after(() => fs.promises.rm(dir, { recursive: true, force: true }))
  const filePath = path.join(dir, 'apple-session.vault')
  const original = Buffer.from('encrypted-existing-session')
  await fs.promises.writeFile(filePath, original, { mode: 0o600 })
  const events = []
  const cookies = store()
  const manager = persistence(dir, {
    isEncryptionAvailable: () => false,
    getSelectedStorageBackend: () => 'kwallet6',
  }, cookies, events)

  await manager.start()
  assert.deepEqual(events, [{ event: 'storage-mode', persistent: false }])
  assert.equal(cookies.listenerCount('changed'), 0)
  assert.deepEqual(await fs.promises.readFile(filePath), original)
})

test('failed decryption is reported and only explicit login may replace it', async (t) => {
  const dir = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'jamelade-persistence-fail-'))
  t.after(() => fs.promises.rm(dir, { recursive: true, force: true }))
  const filePath = path.join(dir, 'apple-session.vault')
  const original = Buffer.from('encrypted-existing-session')
  await fs.promises.writeFile(filePath, original, { mode: 0o600 })
  const events = []
  const cookies = store()
  const manager = persistence(dir, {
    isEncryptionAvailable: () => true,
    getSelectedStorageBackend: () => 'kwallet6',
    decryptString() { throw new Error('wallet locked') },
    encryptString: (value) => Buffer.from(value),
  }, cookies, events)

  await manager.start()
  assert.deepEqual(events, [
    { event: 'storage-mode', persistent: true },
    { event: 'storage-restore-failed' },
  ])
  assert.equal(cookies.listenerCount('changed'), 0)
  assert.deepEqual(await fs.promises.readFile(filePath), original)

  await manager.explicitLogin()
  assert.equal(cookies.listenerCount('changed'), 1)
  assert.notDeepEqual(await fs.promises.readFile(filePath), original)
})

test('remembered-email updates are deduplicated, serialized and rate-limited', async (t) => {
  const dir = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'jamelade-email-ipc-'))
  t.after(() => fs.promises.rm(dir, { recursive: true, force: true }))
  const events = []
  const blocks = []
  const manager = persistence(dir, {
    isEncryptionAvailable: () => true,
    getSelectedStorageBackend: () => 'kwallet6',
    decryptString: (value) => value.toString(),
    encryptString: (value) => Buffer.from(value),
  }, store(), events, (reason) => blocks.push(reason))
  await manager.start()

  for (let index = 0; index < 10; index += 1) {
    await manager.rememberEmail(`listener${index}@example.test`)
  }
  await manager.explicitLogin()

  assert.equal(blocks.filter((reason) => reason.includes('flood')).length, 2)
  assert.equal(
    await fs.promises.readFile(path.join(dir, 'apple-login-email.vault'), 'utf8'),
    'listener7@example.test',
  )
})
