// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

const test = require('node:test')
const assert = require('node:assert/strict')
const vm = require('node:vm')
const { createLoginEmailAssist, emailFieldScript } = require('./login-email-assist')

test('the injected script names only bounded username or email fields', () => {
  const source = emailFieldScript('listener@example.test')
  assert.match(source, /input\[type="email"\]/)
  assert.match(source, /input\[autocomplete="username"\]/)
  assert.match(source, /input\[inputmode="email"\]/)
  assert.match(source, /addEventListener\('input'/)
  assert.doesNotMatch(source, /input\[type="text"\]|input:not\(\[type\]\)/)
  assert.doesNotMatch(source, /password/i)
  assert.doesNotMatch(source, /fetch\(|XMLHttpRequest|WebSocket/)
})

test('typing waits for a complete stable address instead of saving a prefix', () => {
  let now = 0
  const listeners = new Map()
  class FakeInput {
    constructor() { this.value = '' }
    matches() { return true }
  }
  const input = new FakeInput()
  const context = vm.createContext({
    Date: { now: () => now },
    Event: class {},
    HTMLInputElement: FakeInput,
    Symbol,
    TextEncoder,
    document: {
      addEventListener: (name, callback) => listeners.set(name, callback),
      querySelectorAll: () => [input],
    },
    window: {},
  })
  const scan = () => vm.runInContext(emailFieldScript(null), context)

  assert.equal(scan(), null)
  input.value = 'listener@example.c'
  listeners.get('input')({ target: input })
  now = 100
  assert.equal(scan(), null)

  input.value = 'listener@example.co'
  listeners.get('input')({ target: input })
  now = 300
  input.value = 'listener@example.com'
  listeners.get('input')({ target: input })
  now = 1_000
  assert.equal(scan(), null)
  now = 1_800
  assert.equal(scan(), 'listener@example.com')
})

test('only trusted Apple frames can supply one remembered email', async () => {
  const remembered = []
  let untrustedRuns = 0
  const trusted = {
    url: 'https://idmsa.apple.com/signin',
    framesInSubtree: [],
    executeJavaScript: async () => 'listener@example.test',
  }
  const untrusted = {
    url: 'https://example.test/',
    framesInSubtree: [],
    executeJavaScript: async () => {
      untrustedRuns += 1
      return 'attacker@example.test'
    },
  }
  const assist = createLoginEmailAssist({
    getContents: () => [
      { isDestroyed: () => false, mainFrame: untrusted },
      { isDestroyed: () => false, mainFrame: trusted },
    ],
    isTrustedAppleUrl: (url) => new URL(url).hostname.endsWith('.apple.com'),
    currentEmail: () => null,
    rememberEmail: async (email) => remembered.push(email),
    intervalMs: 0,
    maxTicks: 1,
  })
  assist.start()
  await new Promise((resolve) => setTimeout(resolve, 10))
  assert.deepEqual(remembered, ['listener@example.test'])
  assert.equal(untrustedRuns, 0)
})
