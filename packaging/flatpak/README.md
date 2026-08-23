# Flatpak

Jamelade needs libadwaita ≥ 1.8 and GTK ≥ 4.20. The Flatpak carries the GNOME
49 runtime, so systems with older desktop libraries can still run it.

Jamelade is **not currently submitted to Flathub**. Public beta releases ship
as standalone Flatpak bundles; these targets produce the same format locally.

```bash
make flatpak          # build and install locally
make flatpak-bundle   # produces Jamelade.flatpak to carry elsewhere
```

The first build needs these user-installed Flathub components:

```bash
flatpak install --user flathub \
  org.flatpak.Builder \
  org.gnome.Sdk//49 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08 \
  org.electronjs.Electron2.BaseApp//25.08
```

The finished bundle needs only `org.gnome.Platform//49`; its runtime metadata
lets Flatpak offer that runtime on a clean machine.

## Files

| | |
| --- | --- |
| `io.github.Jamelade.Jamelade.yml` | the manifest |
| `cargo-sources.json` | 251 crate archives by hash — generated, committed |
| `generate-sources.sh` | regenerates the above from `Cargo.lock` |
| `electron-shim` | puts zypak where it has to be |

The manifest also builds the pinned MIT-licensed `gtk4-layer-shell` 1.3.0
library. It powers only the optional Jamkin overlay on compatible Wayland
compositors and requires no additional sandbox permission.

Run `generate-sources.sh` whenever `Cargo.lock` changes. There is no npm
equivalent to regenerate — see below.

## Three things that are not obvious

**zypak wraps `electron`, not the app.** It has to be the *direct* parent of
the Chromium process, and Chromium is the app's grandchild: the launcher starts
the Rust binary, which spawns Electron. Wrapping the launcher leaves the sidecar
aborting on `chrome-sandbox … mode 4755` exactly as if zypak were absent, and
the supervisor restarts it for ever. So `electron-shim` stands where
`electron_binary()` looks and wraps the real binary beside it.

**No npm tree.** The sidecar's only dependency is Electron itself, and the app
runs `node_modules/electron/dist/electron` directly — the other thirteen
packages exist only to *download* that binary. So the castLabs release is a
single pinned archive rather than a generated node-sources list. Bumping
Electron means changing the URL and the `sha256` in the manifest, and nothing
else.

**The build is offline**, because `flatpak-builder` forbids network access
during a build. Every crate is declared with a hash up front.

## Permissions, and why

- `--share=network` — Widevine has no persistent licences on Linux, so playback
  needs a connection every time, and the CDM is fetched on first run.
- `--device=dri` — without it GTK renders in software and the grids scroll badly
  enough to read as a bug in the app.
- `--own-name=…` — Flatpak's bus proxy only lets an app own names matching its
  ID; the explicit names cover Jamelade's stable portal-managed sub-launcher
  and MPRIS player. Without the former `GtkApplication` exits 0 with no window.
- **No home-folder access.** Settings, artwork cache, session and the CDM all
  live under the app's own directories. The only filesystem grants are the
  standard narrow runtime paths for Discord's local IPC socket; Jamelade does
  not probe them until the separately off-by-default Discord Activity setting
  is enabled, and the code accepts only same-user Unix sockets in those paths.

Changing the rounded Jamkin launcher tile uses the standard Dynamic Launcher
portal. The desktop shows its own confirmation and writes one fixed
`io.github.Jamelade.Jamelade.Launcher.desktop` sub-entry; Jamelade never receives
access to `~/.local/share/applications` or any other launcher.

## The CDM

It is fetched, not bundled — by Chromium's own component updater, into
`~/.var/app/io.github.Jamelade.Jamelade/config/Jamelade/WidevineCdm/`. Nothing
proprietary is redistributed here: Electron is MIT, Jamelade is GPL-3, and the
CDM arrives on the user's machine through their own updater.

Playback was verified in the release sandbox without home-directory access or
a preinstalled CDM.
