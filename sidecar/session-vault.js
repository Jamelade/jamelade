// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

const crypto = require('node:crypto')
const fs = require('node:fs')
const path = require('node:path')

const MAX_COOKIE_COUNT = 512
const MAX_COOKIE_VALUE = 16 * 1024
const MAX_PLAINTEXT_BYTES = 2 * 1024 * 1024
const MAX_VAULT_BYTES = 3 * 1024 * 1024
const SAVE_DEBOUNCE_MS = 250
const RESTORE_RETRY_DELAYS_MS = [250, 750, 1_500, 2_500]
const SAME_SITE = new Set(['unspecified', 'no_restriction', 'lax', 'strict'])

function boundedString(value, maximum, allowEmpty = true) {
  return typeof value === 'string'
    && (allowEmpty || value.length > 0)
    && value.length <= maximum
    && !/[\r\n\0]/.test(value)
}

function appleCookie(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  if (!boundedString(raw.domain, 255, false)) return null
  const host = raw.domain.replace(/^\./, '').toLowerCase()
  if (host !== 'apple.com' && !host.endsWith('.apple.com')) return null
  if (!boundedString(raw.name, 256, false)) return null
  if (!boundedString(raw.value, MAX_COOKIE_VALUE)) return null
  if (!boundedString(raw.path, 1024, false) || !raw.path.startsWith('/')) return null
  if (!SAME_SITE.has(raw.sameSite)) return null
  // Authentication state must never be replayable over plaintext HTTP. Any
  // non-secure Apple cookie is presentation/analytics state we do not need to
  // persist; rejecting it also makes the restore URL unconditionally HTTPS.
  if (raw.secure !== true) return null

  const isSession = raw.session !== false
  if (!isSession && (!Number.isFinite(raw.expirationDate) || raw.expirationDate <= 0)) {
    return null
  }

  return {
    name: raw.name,
    value: raw.value,
    domain: raw.domain.toLowerCase(),
    hostOnly: raw.hostOnly === true,
    path: raw.path,
    secure: true,
    httpOnly: raw.httpOnly === true,
    session: isSession,
    sameSite: raw.sameSite,
    ...(isSession ? {} : { expirationDate: raw.expirationDate }),
  }
}

function cookieSetDetails(cookie) {
  const host = cookie.domain.replace(/^\./, '')
  const details = {
    url: `https://${host}${cookie.path}`,
    name: cookie.name,
    value: cookie.value,
    path: cookie.path,
    secure: cookie.secure,
    httpOnly: cookie.httpOnly,
    sameSite: cookie.sameSite,
  }
  if (!cookie.hostOnly) details.domain = cookie.domain
  if (!cookie.session) details.expirationDate = cookie.expirationDate
  return details
}

function parseSnapshot(plaintext) {
  if (typeof plaintext !== 'string'
    || Buffer.byteLength(plaintext, 'utf8') > MAX_PLAINTEXT_BYTES) {
    throw new Error('cookie vault plaintext exceeded its safe size')
  }
  const parsed = JSON.parse(plaintext)
  if (!parsed || parsed.version !== 1 || !Array.isArray(parsed.cookies)) {
    throw new Error('cookie vault has an unsupported shape')
  }
  if (parsed.cookies.length > MAX_COOKIE_COUNT) {
    throw new Error('cookie vault contains too many cookies')
  }
  const cookies = parsed.cookies.map(appleCookie)
  if (cookies.some((cookie) => cookie === null)) {
    throw new Error('cookie vault contains an invalid cookie')
  }
  return cookies
}

async function removeIfPresent(filePath) {
  try {
    await fs.promises.unlink(filePath)
  } catch (err) {
    if (!err || err.code !== 'ENOENT') throw err
  }
}

async function ensurePrivateDir(dir) {
  await fs.promises.mkdir(dir, { recursive: true, mode: 0o700 })
  const noFollow = fs.constants.O_NOFOLLOW ?? 0
  const directory = fs.constants.O_DIRECTORY ?? 0
  let handle
  try {
    handle = await fs.promises.open(dir, fs.constants.O_RDONLY | noFollow | directory)
    const stat = await handle.stat()
    if (!stat.isDirectory()) throw new Error('private data directory is not a directory')
    await handle.chmod(0o700)
  } finally {
    if (handle) await handle.close().catch(() => {})
  }
}

async function writePrivate(filePath, bytes) {
  const dir = path.dirname(filePath)
  await ensurePrivateDir(dir)

  const temporary = `${filePath}.${crypto.randomUUID()}.tmp`
  let handle
  let created = false
  try {
    handle = await fs.promises.open(temporary, 'wx', 0o600)
    created = true
    await handle.writeFile(bytes)
    await handle.sync()
    await handle.close()
    handle = null
    await fs.promises.rename(temporary, filePath)
  } finally {
    if (handle) await handle.close().catch(() => {})
    if (created) await removeIfPresent(temporary).catch(() => {})
  }
}

/** Open and inspect the same inode, refusing symlinks on Jamelade's Linux target. */
async function readPrivate(filePath, maximum) {
  const noFollow = fs.constants.O_NOFOLLOW ?? 0
  let handle
  try {
    handle = await fs.promises.open(filePath, fs.constants.O_RDONLY | noFollow)
    const stat = await handle.stat()
    if (!stat.isFile() || stat.size > maximum) {
      throw new Error('private data is not a bounded regular file')
    }
    // Tighten an older installation through the descriptor we just validated;
    // doing this by pathname would reopen the symlink race we are avoiding.
    await handle.chmod(0o600)
    const bytes = await handle.readFile()
    if (bytes.length > maximum) {
      throw new Error('private data exceeded its safe size')
    }
    return bytes
  } finally {
    if (handle) await handle.close().catch(() => {})
  }
}

/**
 * Persist only Apple cookies, encrypted as one opaque blob by Electron's OS
 * keyring integration. The Chromium partition itself stays memory-only, so
 * localStorage and IndexedDB can never leave plaintext tokens on disk.
 */
function createCookieVault({ safeStorage, cookieStore, filePath, onError }) {
  let suppressed = false
  // No writes until an existing vault was restored, found missing, or the
  // user explicitly completed a replacement sign-in. A locked keyring must
  // never turn a valid encrypted session into a tiny anonymous-cookie vault.
  let writable = false
  let timer = null
  let saveChain = Promise.resolve()
  let watching = false

  const report = (err) => {
    if (typeof onError === 'function') onError(err)
  }

  async function saveNow() {
    if (suppressed || !writable) return
    const cookies = (await cookieStore.get({})).map(appleCookie).filter(Boolean)
    if (cookies.length > MAX_COOKIE_COUNT) {
      throw new Error('refusing to persist an excessive number of Apple cookies')
    }
    if (cookies.length === 0) {
      await removeIfPresent(filePath)
      return
    }

    const plaintext = JSON.stringify({ version: 1, cookies })
    if (Buffer.byteLength(plaintext, 'utf8') > MAX_PLAINTEXT_BYTES) {
      throw new Error('refusing to persist an oversized cookie snapshot')
    }
    const encrypted = safeStorage.encryptString(plaintext)
    if (!Buffer.isBuffer(encrypted) || encrypted.length > MAX_VAULT_BYTES) {
      throw new Error('encrypted cookie vault exceeded its safe size')
    }
    await writePrivate(filePath, encrypted)
  }

  function queueSave() {
    if (suppressed || !writable) return
    clearTimeout(timer)
    timer = setTimeout(() => {
      timer = null
      saveChain = saveChain.then(saveNow).catch(report)
    }, SAVE_DEBOUNCE_MS)
  }

  async function restore(delays = RESTORE_RETRY_DELAYS_MS) {
    suppressed = true
    try {
      let lastError = null
      for (let attempt = 0; ; attempt += 1) {
        try {
          const encrypted = await readPrivate(filePath, MAX_VAULT_BYTES)
          const cookies = parseSnapshot(safeStorage.decryptString(encrypted))
          for (const cookie of cookies) {
            await cookieStore.set(cookieSetDetails(cookie))
          }
          writable = true
          return 'restored'
        } catch (err) {
          if (err && err.code === 'ENOENT') {
            writable = true
            return 'missing'
          }
          lastError = err
          const wait = delays[attempt]
          if (!Number.isFinite(wait) || wait < 0) break
          await new Promise((resolve) => setTimeout(resolve, wait))
        }
      }
      writable = false
      report(lastError)
      return 'failed'
    } finally {
      suppressed = false
    }
  }

  function watch() {
    if (watching || !writable) return false
    watching = true
    cookieStore.on('changed', queueSave)
    return true
  }

  // Called only after Apple's visible login completed in this process. At
  // that point replacing an unreadable old session is an explicit user action,
  // not an anonymous page load racing a locked keyring.
  function allowReplacement() {
    writable = true
    watch()
  }

  function suspend() {
    suppressed = true
    clearTimeout(timer)
    timer = null
  }

  function resume() {
    suppressed = false
  }

  async function clearSaved() {
    clearTimeout(timer)
    timer = null
    await saveChain
    await removeIfPresent(filePath)
    writable = true
  }

  async function flush() {
    clearTimeout(timer)
    timer = null
    await saveChain
    if (!suppressed && writable) await saveNow()
  }

  return { allowReplacement, clearSaved, flush, restore, resume, suspend, watch }
}

module.exports = {
  appleCookie,
  cookieSetDetails,
  createCookieVault,
  ensurePrivateDir,
  parseSnapshot,
  readPrivate,
  writePrivate,
}
