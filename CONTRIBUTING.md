<!--
SPDX-FileCopyrightText: 2026 Jamelade contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Contributing to Jamelade

Thank you for helping make Jamelade better. Bug reports, focused feature
proposals, documentation improvements, accessibility work, tests, and reviewed
code changes are welcome.

## Before opening an issue

- Search existing issues first.
- Use a short, descriptive title and include reproducible steps.
- State the Linux distribution, desktop environment, display protocol
  (Wayland or X11), installation method, and Jamelade version when relevant.
- Redact account names, private playlists, local paths, device names, and
  listening history from screenshots and logs.
- Never attach Apple cookies, tokens, credentials, keyring contents, Discord
  secrets, or a complete Jamelade data directory.

Security vulnerabilities do not belong in public issues. Follow
[SECURITY.md](SECURITY.md) instead.

## Development checks

Jamelade uses Rust, GTK4/libadwaita, and a small Node.js Electron sidecar. The
Flatpak is the most reproducible development environment on distributions that
do not yet provide the required GTK versions.

```bash
make sidecar-check
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`make check` runs the complete project check set, including size budgets.
Test account, display, compositor, keyring, and lifecycle changes manually;
unit tests cannot cover those environments.

## Pull requests

- Keep each pull request focused and explain its user-visible effect.
- Add or update tests for behavior changes.
- Preserve privacy defaults: optional network integrations stay off until the
  user explicitly enables them.
- Treat authentication, cookie storage, sidecar IPC, navigation rules,
  external URLs, host-helper IPC, packaging, and dependency updates as
  security-sensitive.
- Do not add telemetry, advertising, bundled credentials, broad filesystem
  access, remote code loading, or silent update mechanisms.
- Keep generated files and large binaries out of Git unless the repository
  already documents them as reviewed project assets.
- Rebase or merge changes from upstream Slipmat deliberately. Authentication,
  playback, queue, and sidecar conflicts require a fresh security review; do
  not resolve them mechanically merely to make a merge compile.
- Use synthetic metadata in tests. Never depend on a real Apple account,
  cookie vault, private playlist, local username, home path, or browser profile.

Changes to dependencies, packaging, Electron/Chromium, icons, fonts, Jamkin
art, or screenshots must preserve applicable licences, notices, privacy
defaults, and reproducible checks.

AI-assisted contributions are welcome when the submitter has reviewed,
understood, tested, and takes responsibility for the result. Mention material
AI assistance in the pull-request description.

By contributing, you confirm that you have the right to submit the material,
that it contains no confidential data or unlicensed third-party asset, and that
your contribution may be distributed under Jamelade's GPL-3.0-or-later licence.
