// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { createRememberedEmail, normalizeEmail } = require('./login-email')

test('only a bounded email address is remembered', () => {
  assert.equal(normalizeEmail('  listener@example.test  '), 'listener@example.test')
  assert.equal(normalizeEmail('not-an-email'), null)
  assert.equal(normalizeEmail('two@@example.test'), null)
  assert.equal(normalizeEmail('unfinished@example'), null)
  assert.equal(normalizeEmail('still-typing@example.c'), null)
  assert.equal(normalizeEmail('line\nbreak@example.test'), null)
  assert.equal(normalizeEmail(`${'x'.repeat(320)}@example.test`), null)
})

test('the remembered email is keyring-encrypted and private on disk', async (t) => {
  const dir = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'jamelade-email-'))
  t.after(() => fs.promises.rm(dir, { recursive: true, force: true }))
  const filePath = path.join(dir, 'apple-login-email.vault')
  const safeStorage = {
    encryptString: (value) => Buffer.from(value).map((byte) => byte ^ 0x5a),
    decryptString: (value) => Buffer.from(value).map((byte) => byte ^ 0x5a).toString(),
  }
  const writer = createRememberedEmail({ safeStorage, filePath })
  assert.equal(await writer.remember('listener@example.test'), true)
  assert.equal((await fs.promises.stat(filePath)).mode & 0o777, 0o600)
  assert.equal((await fs.promises.readFile(filePath, 'utf8')).includes('listener@'), false)

  const reader = createRememberedEmail({ safeStorage, filePath })
  assert.equal(await reader.load(), 'listener@example.test')
  assert.equal(reader.current(), 'listener@example.test')
})

test('a decryption failure preserves the remembered-email file', async (t) => {
  const dir = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'jamelade-email-fail-'))
  t.after(() => fs.promises.rm(dir, { recursive: true, force: true }))
  const filePath = path.join(dir, 'apple-login-email.vault')
  const bytes = Buffer.from('opaque-encrypted-email')
  await fs.promises.writeFile(filePath, bytes, { mode: 0o600 })
  let errors = 0
  const memory = createRememberedEmail({
    safeStorage: { decryptString() { throw new Error('wallet locked') } },
    filePath,
    onError: () => { errors += 1 },
  })
  assert.equal(await memory.load(), null)
  assert.equal(errors, 1)
  assert.deepEqual(await fs.promises.readFile(filePath), bytes)
})
