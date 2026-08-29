// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { EventEmitter } = require('node:events')
const {
  appleCookie,
  cookieSetDetails,
  createCookieVault,
  ensurePrivateDir,
  parseSnapshot,
  readPrivate,
} = require('./session-vault')

function cookieStore() {
  const store = new EventEmitter()
  store.get = async () => []
  store.set = async () => {}
  return store
}

function cookie(overrides = {}) {
  return {
    name: 'session',
    value: 'test-value',
    domain: '.apple.com',
    hostOnly: false,
    path: '/',
    secure: true,
    httpOnly: true,
    session: false,
    sameSite: 'no_restriction',
    expirationDate: 2_000_000_000,
    ...overrides,
  }
}

test('the vault accepts only real Apple cookie domains', () => {
  assert.ok(appleCookie(cookie()))
  assert.ok(appleCookie(cookie({ domain: 'music.apple.com', hostOnly: true })))
  assert.equal(appleCookie(cookie({ domain: 'evilapple.com' })), null)
  assert.equal(appleCookie(cookie({ domain: 'apple.com.evil.example' })), null)
  assert.equal(appleCookie(cookie({ value: 'x'.repeat(20 * 1024) })), null)
  assert.equal(appleCookie(cookie({ secure: false })), null)
})

test('restored cookie URLs contain only the validated domain and path', () => {
  const details = cookieSetDetails(appleCookie(cookie()))
  assert.equal(details.url, 'https://apple.com/')
  assert.equal(details.domain, '.apple.com')
  assert.equal(details.value, 'test-value')
})

test('the decrypted snapshot is versioned, bounded and fully validated', () => {
  const valid = JSON.stringify({ version: 1, cookies: [cookie()] })
  assert.equal(parseSnapshot(valid).length, 1)
  assert.throws(() => parseSnapshot(JSON.stringify({ version: 2, cookies: [] })))
  assert.throws(() => parseSnapshot(JSON.stringify({
    version: 1,
    cookies: [cookie({ domain: 'evilapple.com' })],
  })))
})

test('private vault reads refuse symlinks', async (t) => {
  if (process.platform !== 'linux' || fs.constants.O_NOFOLLOW === undefined) {
    t.skip('O_NOFOLLOW is a Linux hardening guarantee')
    return
  }
  const dir = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'jamelade-vault-'))
  t.after(() => fs.promises.rm(dir, { recursive: true, force: true }))
  const target = path.join(dir, 'other-file')
  const vault = path.join(dir, 'apple-session.vault')
  await fs.promises.writeFile(target, 'do not read through me')
  await fs.promises.symlink(target, vault)

  await assert.rejects(readPrivate(vault, 1024))
})

test('private vault directories refuse symlinks', async (t) => {
  if (process.platform !== 'linux' || fs.constants.O_NOFOLLOW === undefined) {
    t.skip('O_NOFOLLOW is a Linux hardening guarantee')
    return
  }
  const dir = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'jamelade-vault-dir-'))
  t.after(() => fs.promises.rm(dir, { recursive: true, force: true }))
  const target = path.join(dir, 'other-directory')
  const privateDir = path.join(dir, 'Jamelade')
  await fs.promises.mkdir(target)
  await fs.promises.symlink(target, privateDir)

  await assert.rejects(ensurePrivateDir(privateDir))
})

test('a failed restore preserves the encrypted vault and disables anonymous writes', async (t) => {
  const dir = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'jamelade-restore-'))
  t.after(() => fs.promises.rm(dir, { recursive: true, force: true }))
  const filePath = path.join(dir, 'apple-session.vault')
  const original = Buffer.from('still-encrypted-session')
  await fs.promises.writeFile(filePath, original, { mode: 0o600 })

  const store = cookieStore()
  let reported = 0
  const vault = createCookieVault({
    safeStorage: {
      decryptString() { throw new Error('wallet locked') },
      encryptString() { throw new Error('must not write') },
    },
    cookieStore: store,
    filePath,
    onError: () => { reported += 1 },
  })

  assert.equal(await vault.restore([]), 'failed')
  assert.equal(vault.watch(), false)
  assert.equal(store.listenerCount('changed'), 0)
  assert.equal(reported, 1)
  assert.deepEqual(await fs.promises.readFile(filePath), original)
})

test('a restored or missing vault enables watching, but only explicit login replaces failure', async (t) => {
  const dir = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'jamelade-restore-state-'))
  t.after(() => fs.promises.rm(dir, { recursive: true, force: true }))
  const filePath = path.join(dir, 'apple-session.vault')
  const store = cookieStore()
  const vault = createCookieVault({
    safeStorage: {
      decryptString: () => JSON.stringify({ version: 1, cookies: [cookie()] }),
      encryptString: (value) => Buffer.from(value),
    },
    cookieStore: store,
    filePath,
  })

  assert.equal(await vault.restore([]), 'missing')
  assert.equal(vault.watch(), true)
  assert.equal(store.listenerCount('changed'), 1)

  const failedStore = cookieStore()
  await fs.promises.writeFile(filePath, 'bad', { mode: 0o600 })
  const failed = createCookieVault({
    safeStorage: {
      decryptString() { throw new Error('locked') },
      encryptString: (value) => Buffer.from(value),
    },
    cookieStore: failedStore,
    filePath,
  })
  assert.equal(await failed.restore([]), 'failed')
  assert.equal(failed.watch(), false)
  failed.allowReplacement()
  assert.equal(failedStore.listenerCount('changed'), 1)
})
