// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

const { readPrivate, writePrivate } = require('./session-vault')

const MAX_EMAIL_BYTES = 320
const MAX_ENCRYPTED_BYTES = 4 * 1024

function normalizeEmail(value) {
  if (typeof value !== 'string') return null
  const email = value.trim()
  if (!email
    || Buffer.byteLength(email, 'utf8') > MAX_EMAIL_BYTES
    || /[\s\0\r\n]/u.test(email)) {
    return null
  }
  const at = email.indexOf('@')
  if (at <= 0 || at !== email.lastIndexOf('@') || at === email.length - 1) return null
  const labels = email.slice(at + 1).split('.')
  if (labels.length < 2
    || labels.some((label) => label.length === 0)
    || labels.at(-1).length < 2) return null
  return email
}

function createRememberedEmail({ safeStorage, filePath, onError }) {
  let remembered = null

  const report = (err) => {
    if (typeof onError === 'function') onError(err)
  }

  async function load() {
    try {
      const encrypted = await readPrivate(filePath, MAX_ENCRYPTED_BYTES)
      const email = normalizeEmail(safeStorage.decryptString(encrypted))
      if (!email) throw new Error('remembered Apple ID email is invalid')
      remembered = email
      return email
    } catch (err) {
      if (err && err.code === 'ENOENT') return null
      report(err)
      return null
    }
  }

  async function remember(value) {
    const email = normalizeEmail(value)
    if (!email) return false
    try {
      const encrypted = safeStorage.encryptString(email)
      if (!Buffer.isBuffer(encrypted) || encrypted.length > MAX_ENCRYPTED_BYTES) {
        throw new Error('encrypted Apple ID email exceeded its safe size')
      }
      await writePrivate(filePath, encrypted)
      remembered = email
      return true
    } catch (err) {
      report(err)
      return false
    }
  }

  function current() {
    return remembered
  }

  return { current, load, remember }
}

module.exports = { createRememberedEmail, normalizeEmail }
