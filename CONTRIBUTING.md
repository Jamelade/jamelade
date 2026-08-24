# Contributing

Focused bug fixes, accessibility improvements, tests, documentation, and
reviewed features are welcome.

- Search existing issues and include reproducible steps, distribution, desktop,
  display protocol, installation method, and Jamelade version.
- Redact account names, private playlists, local paths, device names, and
  listening history. Never attach credentials, cookies, tokens, keyring data,
  or a complete app-data directory.
- Report security issues privately through [SECURITY.md](SECURITY.md).
- Keep optional network integrations off until separately enabled. Do not add
  analytics, ads, bundled credentials, broad filesystem access, remote code
  loading, or silent updates.
- Treat authentication, sidecar IPC, cookie storage, networking, dependencies,
  packaging, and Jamkin assets as high-risk areas. Read
  [ARCHITECTURE.md](ARCHITECTURE.md) first.
- Use synthetic test data and update tests and user-facing documentation with
  behavior changes. Do not patch only an installed Flatpak.

Run before submitting:

```bash
make check
git diff --check
```

AI-assisted contributions are welcome when the submitter has reviewed,
understood, tested, and accepts responsibility for the result. Contributions
are distributed under GPL-3.0-or-later.
