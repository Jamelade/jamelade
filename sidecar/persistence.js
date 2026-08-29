// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

const fs = require('node:fs')
const path = require('node:path')
const { createRememberedEmail, normalizeEmail } = require('./login-email')
const { createCookieVault } = require('./session-vault')

const STORAGE_RETRY_DELAYS_MS = [250, 750, 1_500, 2_500]
const MAX_EMAIL_UPDATES_PER_MINUTE = 8

function createPersistence({
  app,
  cookieStore,
  safeStorage,
  mayPersistCookies,
  send,
  log,
  blocked,
  retryDelays = STORAGE_RETRY_DELAYS_MS,
  restoreDelays,
}) {
  const cookiePath = path.join(app.getPath('userData'), 'apple-session.vault')
  const emailPath = path.join(app.getPath('userData'), 'apple-login-email.vault')
  const cookieVault = createCookieVault({
    safeStorage,
    cookieStore,
    filePath: cookiePath,
    onError: () => log('cookie vault operation failed'),
  })
  const emailMemory = createRememberedEmail({
    safeStorage,
    filePath: emailPath,
    onError: () => log('remembered email operation failed'),
  })
  let pendingLoginEmail = null
  let emailWriteChain = Promise.resolve()
  let emailWindowStarted = Date.now()
  let emailUpdates = 0

  function available() {
    let backend = 'unknown'
    let encryptionAvailable = false
    try {
      encryptionAvailable = safeStorage.isEncryptionAvailable()
      if (process.platform === 'linux') backend = safeStorage.getSelectedStorageBackend()
    } catch {
      encryptionAvailable = false
    }
    return mayPersistCookies(encryptionAvailable, backend)
  }

  async function start() {
    const hasSavedState = fs.existsSync(cookiePath) || fs.existsSync(emailPath)
    const delays = hasSavedState ? retryDelays : []
    let persistent = false
    for (let attempt = 0; ; attempt += 1) {
      if (available()) {
        persistent = true
        break
      }
      const wait = delays[attempt]
      if (!Number.isFinite(wait)) break
      await new Promise((resolve) => setTimeout(resolve, wait))
    }
    send({ event: 'storage-mode', persistent })
    if (!persistent) return

    const restored = await cookieVault.restore(restoreDelays)
    if (restored === 'failed') {
      send({ event: 'storage-restore-failed' })
    } else {
      cookieVault.watch()
    }
    await emailMemory.load()
  }

  async function ensureEmailMemory() {
    if (!available()) return null
    if (!emailMemory.current()) await emailMemory.load()
    return emailMemory
  }

  async function explicitLogin() {
    if (available()) {
      cookieVault.allowReplacement()
      await cookieVault.flush()
    }
    if (pendingLoginEmail) {
      await emailWriteChain
      const memory = await ensureEmailMemory()
      if (memory) await memory.remember(pendingLoginEmail)
    }
  }

  function emailUpdateAllowed() {
    const now = Date.now()
    if (now - emailWindowStarted >= 60_000) {
      emailWindowStarted = now
      emailUpdates = 0
    }
    emailUpdates += 1
    return emailUpdates <= MAX_EMAIL_UPDATES_PER_MINUTE
  }

  async function rememberEmail(value) {
    const email = normalizeEmail(value)
    if (!email) {
      blocked('blocked invalid remembered-email update')
      return false
    }
    if (email === pendingLoginEmail) return true
    if (!emailUpdateAllowed()) {
      blocked('blocked remembered-email update flood')
      return false
    }
    pendingLoginEmail = email
    emailWriteChain = emailWriteChain
      .then(async () => {
        const memory = await ensureEmailMemory()
        if (memory) await memory.remember(email)
      })
      .catch(() => log('remembered email operation failed'))
    await emailWriteChain
    return true
  }

  return {
    clearSaved: () => cookieVault.clearSaved(),
    currentEmail: () => emailMemory.current(),
    explicitLogin,
    flush: () => cookieVault.flush(),
    rememberEmail,
    resume: () => cookieVault.resume(),
    start,
    suspend: () => cookieVault.suspend(),
  }
}

module.exports = { createPersistence }
