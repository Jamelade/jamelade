# Flatpak packaging

The Flatpak carries GNOME Platform 49 because Jamelade needs GTK 4.20 and
libadwaita 1.8. Jamelade is not currently on Flathub.

```bash
make flatpak          # build and install for the current user
make flatpak-bundle   # write Jamelade.flatpak
```

Required build components:

```bash
flatpak install --user flathub \
  org.flatpak.Builder \
  org.gnome.Sdk//49 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08 \
  org.electronjs.Electron2.BaseApp//25.08
```

`io.github.Jamelade.Jamelade.yml` is the production manifest.
`io.github.Jamelade.Jamelade.BrokerTest.yml` is an isolated development
identity with separate settings and session state. `cargo-sources.json` pins
all crate archives; regenerate it after a lockfile change with
`generate-sources.sh`.

The castLabs Electron archive is pinned directly in the manifests. The app
runs only its Electron binary, so no npm dependency tree is packaged. `zypak`
must remain Electron's direct parent; `electron-shim` provides that boundary.

The app requires network, display, audio, DRI, keyring, MPRIS, launcher, and
narrow optional Discord-socket permissions. It has no home-directory grant.
`gtk4-layer-shell` powers only the optional Desktop Jamkin overlay and adds no
permission. Global shortcuts and the per-app key used for an optional encrypted
ListenBrainz token use desktop portals and need no additional broad grant.

Widevine is not bundled. Chromium's component updater fetches it into the
app-private profile on the user's machine. Do not add a CDM or SF Pro font to
the manifest, source, or package.
