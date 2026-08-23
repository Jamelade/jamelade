<!--
SPDX-FileCopyrightText: 2026 Jamelade contributors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Security policy

## Reporting a vulnerability

Please do not open a public issue for a vulnerability involving authentication,
Apple cookies or tokens, the keyring-backed session vault, Electron navigation,
sidecar IPC, network restrictions, update or packaging behavior, or unintended
disclosure of listening data.

Use this repository's **Private vulnerability reporting** form under the
Security tab. Include:

- the affected commit or version;
- a concise impact description;
- reproducible steps or a minimal proof of concept;
- relevant logs with credentials, account data, local paths, and listening
  history removed; and
- any suggested mitigation.

Do not send real Apple credentials, cookies, tokens, keyring exports, or private
library data. Reports and disclosure coordination stay inside GitHub's private
advisory.

## Scope and support

| Version | Supported |
| --- | --- |
| Current `main` and the newest published beta/release | Yes |
| Older development snapshots and superseded releases | No |

Relevant reports include credential or cookie disclosure, unsafe session
persistence, navigation or permission bypasses, sidecar protocol injection,
unintended local-file access, optional providers receiving data without
consent, sensitive logging, unsafe release or update behavior, and exploitable
dependency issues. Ordinary Apple service outages, missing lyrics, DRM support
on unsupported architectures, or a user deliberately enabling documented
Discord or lyrics sharing are not vulnerabilities by themselves.

Jamelade cannot guarantee the availability or behavior of Apple Music,
MusicKit, Widevine, lyrics providers, Discord, or other third-party services.
Reports should concern weaknesses Jamelade can reasonably control. The main
trust boundaries are described in [ARCHITECTURE.md](ARCHITECTURE.md).
