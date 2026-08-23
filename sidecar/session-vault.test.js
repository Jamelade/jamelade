// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const {
  appleCookie,
  cookieSetDetails,
  ensurePrivateDir,
  parseSnapshot,
  readPrivate,
} = require('./session-vault')

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
