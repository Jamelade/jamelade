// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

const { normalizeEmail } = require('./login-email')

const DEFAULT_INTERVAL_MS = 500
const FIELD_STABLE_MS = 1_500
// Login can include a password-manager handoff or two-factor approval. Keep the
// bounded helper alive for that flow, while reveal()/conceal() still stop it as
// soon as the explicit Apple login window closes.
const DEFAULT_MAX_TICKS = 1_200

function emailFieldScript(remembered) {
  const value = JSON.stringify(normalizeEmail(remembered))
  return `(() => {
    const remembered = ${value};
    const stableFor = ${FIELD_STABLE_MS};
    const fieldSelector = [
      'input[type="email"]',
      'input[autocomplete="username"]',
      'input[autocomplete="email"]',
      'input[inputmode="email"]',
      'input[name="accountName"]',
      'input[name*="account_name"]',
      'input[id*="accountName"]',
      'input[id*="account_name"]'
    ].join(',');
    const valid = (candidate) => {
      if (typeof candidate !== 'string') return null;
      const email = candidate.trim();
      if (!email || new TextEncoder().encode(email).length > 320 || /[\\s\\0\\r\\n]/u.test(email)) return null;
      const at = email.indexOf('@');
      if (at <= 0 || at !== email.lastIndexOf('@') || at >= email.length - 1) return null;
      const labels = email.slice(at + 1).split('.');
      return labels.length >= 2 && labels.every((label) => label.length > 0) && labels.at(-1).length >= 2
        ? email
        : null;
    };
    const stateKey = Symbol.for('io.github.Jamelade.login-email-assist');
    let state = window[stateKey];
    if (!state || typeof state !== 'object') {
      state = { candidate: null, changedAt: 0, committed: null, listening: false };
      Object.defineProperty(window, stateKey, {
        value: state,
        configurable: false,
        enumerable: false,
        writable: false
      });
    }
    const capture = (input, commit = false) => {
      if (!(input instanceof HTMLInputElement) || !input.matches(fieldSelector)) return;
      const email = valid(input.value);
      if (!email) return;
      if (state.candidate !== email) {
        state.candidate = email;
        state.changedAt = Date.now();
      }
      if (commit) state.committed = email;
    };
    if (!state.listening) {
      document.addEventListener('input', (event) => capture(event.target), true);
      document.addEventListener('change', (event) => capture(event.target, true), true);
      document.addEventListener('focusout', (event) => capture(event.target, true), true);
      document.addEventListener('submit', () => {
        for (const input of document.querySelectorAll(fieldSelector)) capture(input, true);
      }, true);
      state.listening = true;
    }
    const inputs = document.querySelectorAll(fieldSelector);
    for (const input of inputs) {
      if (!input.value.trim() && remembered) {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
        if (setter) {
          setter.call(input, remembered);
          input.dispatchEvent(new Event('input', { bubbles: true }));
        }
      }
      capture(input);
    }
    if (state.committed) return state.committed;
    return state.candidate && Date.now() - state.changedAt >= stableFor
      ? state.candidate
      : null;
  })()`
}

function createLoginEmailAssist({
  getContents,
  isTrustedAppleUrl,
  currentEmail,
  rememberEmail,
  intervalMs = DEFAULT_INTERVAL_MS,
  maxTicks = DEFAULT_MAX_TICKS,
}) {
  let timer = null
  let running = false
  let ticks = 0

  function stop() {
    running = false
    clearTimeout(timer)
    timer = null
  }

  async function scan() {
    if (!running) return
    ticks += 1
    const script = emailFieldScript(currentEmail())
    for (const contents of getContents()) {
      if (!contents || contents.isDestroyed()) continue
      const root = contents.mainFrame
      const frames = [root, ...(root && root.framesInSubtree ? root.framesInSubtree : [])]
      for (const frame of new Set(frames)) {
        if (!frame || !isTrustedAppleUrl(frame.url)) continue
        try {
          const email = normalizeEmail(await frame.executeJavaScript(script, false))
          if (email) {
            await rememberEmail(email)
            stop()
            return
          }
        } catch {
          // Frames can disappear while Apple advances the login flow. The next
          // bounded scan observes the replacement; no remote detail is logged.
        }
      }
    }
    if (running && ticks < maxTicks) {
      timer = setTimeout(scan, intervalMs)
    } else {
      stop()
    }
  }

  function start() {
    stop()
    running = true
    ticks = 0
    scan()
  }

  return { start, stop }
}

module.exports = { createLoginEmailAssist, emailFieldScript }
