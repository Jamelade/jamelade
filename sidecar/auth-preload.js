// SPDX-FileCopyrightText: 2026 Jamelade contributors
// SPDX-License-Identifier: GPL-3.0-or-later

'use strict'

// Intentionally empty. Apple authentication popups get a sandboxed renderer
// without the player bridge or any other Electron capability. The privileged
// main process performs the bounded email-only assist on validated Apple
// frames; this preload never receives even that value.
