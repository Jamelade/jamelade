# AppImage packaging

The AppImage is a secondary, native package for current x86_64 distributions.
The Flatpak remains the recommended package because it also confines Jamelade
with an outer application sandbox.

GitHub Actions builds the AppImage on Ubuntu 24.04 and bundles pinned GTK 4.20,
libadwaita 1.8, gtk4-layer-shell, castLabs Electron, the Jamkin assets, and the
required licence notices. It does not bundle Widevine or fonts. The practical
host floor is glibc 2.39, which includes Ubuntu 24.04 and contemporary Fedora.

```bash
./packaging/appimage/build.sh
chmod +x appimage-dist/Jamelade-*-x86_64.AppImage
./appimage-dist/Jamelade-*-x86_64.AppImage
```

The build needs the packages installed by the AppImage job in
`.github/workflows/release.yml`. It downloads only versioned, checksum-pinned
GTK, libadwaita, gtk4-layer-shell, linuxdeploy, and castLabs Electron archives.

The Electron sidecar keeps Chromium's renderer sandbox enabled. Jamelade never
adds `--no-sandbox`; a host that blocks Chromium's available sandbox mechanisms
will therefore reject playback rather than weaken the browser boundary.

Unlike the Flatpak, an AppImage runs with the invoking user's normal filesystem
permissions. Jamelade still uses only its documented XDG locations, but the
package itself does not enforce that boundary.
