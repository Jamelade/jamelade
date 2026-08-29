#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Jamelade contributors
# SPDX-License-Identifier: GPL-3.0-or-later

set -euo pipefail
export LC_ALL=C
export TZ=UTC
umask 022

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

build_root="$root/.appimage-build"
downloads="$build_root/downloads"
sources="$build_root/sources"
prefix="$build_root/prefix"
appdir="$build_root/Jamelade.AppDir"
output="$root/appimage-dist"

case "$build_root" in
    "$root"/.appimage-build) ;;
    *) printf 'AppImage build: refusing unexpected build path\n' >&2; exit 1 ;;
esac

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
[[ -n "$version" ]] || {
    printf 'AppImage build: could not read Cargo version\n' >&2
    exit 1
}

for tool in cargo rustc cmp curl git jq make meson ninja pkg-config sha256sum tar unzip \
    patchelf desktop-file-validate appstreamcli dpkg-query glib-compile-schemas ldconfig; do
    command -v "$tool" >/dev/null 2>&1 || {
        printf 'AppImage build: missing required tool: %s\n' "$tool" >&2
        exit 1
    }
done

mkdir -p "$downloads" "$sources" "$prefix" "$output"

fetch() {
    local url="$1" expected="$2" destination="$3" temporary
    if [[ -f "$destination" ]] \
        && printf '%s  %s\n' "$expected" "$destination" | sha256sum --check --status; then
        return
    fi
    temporary="$destination.part"
    rm -f -- "$temporary"
    curl --fail --location --proto '=https' --tlsv1.2 --retry 3 \
        --output "$temporary" "$url"
    printf '%s  %s\n' "$expected" "$temporary" | sha256sum --check --status || {
        rm -f -- "$temporary"
        printf 'AppImage build: checksum mismatch for %s\n' "$url" >&2
        exit 1
    }
    mv -- "$temporary" "$destination"
}

glib_version=2.86.5
glib_sha=ce85a947bb8b3c0204dbeff79aec39bcb46371c6fafb64ba5b8726c71e038d5f
wayland_version=1.24.0
wayland_sha=82892487a01ad67b334eca83b54317a7c86a03a89cfadacfef5211f11a5d0536
protocols_version=1.44
protocols_sha=3df1107ecf8bfd6ee878aeca5d3b7afd81248a48031e14caf6ae01f14eebb50e
cairo_version=1.18.4
cairo_sha=445ed8208a6e4823de1226a74ca319d3600e83f6369f99b14265006599c32ccb
harfbuzz_version=8.5.0
harfbuzz_sha=77e4f7f98f3d86bf8788b53e6832fb96279956e1c3961988ea3d4b7ca41ddc27
pango_version=1.56.4
pango_sha=17065e2fcc5f5a5bdbffc884c956bfc7c451a96e8c4fb2f8ad837c6413cb5a01
gtk_version=4.20.4
gtk_sha=a21f825bd44afc4dd99ba4eea8ff57c8f2e51085cb402a68ed4cbb35299826a4
adw_version=1.8.7
adw_sha=db61e9ed0f47a210869d4d36809b1deea140776c58632bdb6704691d7d6e7abb
layer_version=1.3.0
layer_sha=1ebb01ab14e98afd1727f68f64981c37bd23305b1f131f5667c02b94cf593192
electron_version='43.2.0+wvcus'
electron_sha=d69d7cb6d27651f51c106fec52c746097d064f95b2fee6966f9888e44e0cbf54
linuxdeploy_version=1-alpha-20251107-1
linuxdeploy_sha=c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d
appimage_runtime_version=20251108
appimage_runtime_commit=dd6cebedcbddde9c82f89b011e8e1d40b6e43868
appimage_runtime_sha=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
appimage_runtime_license_sha=aa154fc9070614bbe7921f89db11efd1dba7a1f3a41685958110e2230f9c0ca1

glib_archive="$downloads/glib-$glib_version.tar.xz"
wayland_archive="$downloads/wayland-$wayland_version.tar.xz"
protocols_archive="$downloads/wayland-protocols-$protocols_version.tar.xz"
cairo_archive="$downloads/cairo-$cairo_version.tar.xz"
harfbuzz_archive="$downloads/harfbuzz-$harfbuzz_version.tar.xz"
pango_archive="$downloads/pango-$pango_version.tar.xz"
gtk_archive="$downloads/gtk-$gtk_version.tar.xz"
adw_archive="$downloads/libadwaita-$adw_version.tar.xz"
layer_archive="$downloads/gtk4-layer-shell-$layer_version.tar.gz"
electron_archive="$downloads/electron-$electron_version-linux-x64.zip"
linuxdeploy="$downloads/linuxdeploy-$linuxdeploy_version-x86_64.AppImage"
appimage_runtime="$downloads/type2-runtime-$appimage_runtime_version-x86_64"
appimage_runtime_license="$downloads/type2-runtime-$appimage_runtime_version-LICENSE"

fetch "https://download.gnome.org/sources/glib/2.86/glib-$glib_version.tar.xz" \
    "$glib_sha" "$glib_archive"
fetch "https://gitlab.freedesktop.org/wayland/wayland/-/releases/$wayland_version/downloads/wayland-$wayland_version.tar.xz" \
    "$wayland_sha" "$wayland_archive"
fetch "https://gitlab.freedesktop.org/wayland/wayland-protocols/-/releases/$protocols_version/downloads/wayland-protocols-$protocols_version.tar.xz" \
    "$protocols_sha" "$protocols_archive"
fetch "https://cairographics.org/releases/cairo-$cairo_version.tar.xz" \
    "$cairo_sha" "$cairo_archive"
fetch "https://github.com/harfbuzz/harfbuzz/releases/download/$harfbuzz_version/harfbuzz-$harfbuzz_version.tar.xz" \
    "$harfbuzz_sha" "$harfbuzz_archive"
fetch "https://download.gnome.org/sources/pango/1.56/pango-$pango_version.tar.xz" \
    "$pango_sha" "$pango_archive"
fetch "https://download.gnome.org/sources/gtk/4.20/gtk-$gtk_version.tar.xz" \
    "$gtk_sha" "$gtk_archive"
fetch "https://download.gnome.org/sources/libadwaita/1.8/libadwaita-$adw_version.tar.xz" \
    "$adw_sha" "$adw_archive"
fetch "https://github.com/wmww/gtk4-layer-shell/archive/refs/tags/v$layer_version.tar.gz" \
    "$layer_sha" "$layer_archive"
fetch "https://github.com/castlabs/electron-releases/releases/download/v43.2.0%2Bwvcus/electron-v43.2.0%2Bwvcus-linux-x64.zip" \
    "$electron_sha" "$electron_archive"
fetch "https://github.com/linuxdeploy/linuxdeploy/releases/download/$linuxdeploy_version/linuxdeploy-x86_64.AppImage" \
    "$linuxdeploy_sha" "$linuxdeploy"
fetch "https://github.com/AppImage/type2-runtime/releases/download/$appimage_runtime_version/runtime-x86_64" \
    "$appimage_runtime_sha" "$appimage_runtime"
fetch "https://raw.githubusercontent.com/AppImage/type2-runtime/$appimage_runtime_commit/LICENSE" \
    "$appimage_runtime_license_sha" "$appimage_runtime_license"
chmod 0755 "$linuxdeploy"

deps_stamp="$prefix/.jamelade-appimage-deps"
expected_stamp="$(printf '%s\n' \
    "glib=$glib_version" \
    "wayland=$wayland_version" \
    "wayland-protocols=$protocols_version" \
    "cairo=$cairo_version" \
    "harfbuzz=$harfbuzz_version" \
    "pango=$pango_version" \
    "gtk=$gtk_version" \
    "libadwaita=$adw_version" \
    "gtk4-layer-shell=$layer_version")"
if [[ ! -f "$deps_stamp" || "$(cat "$deps_stamp")" != "$expected_stamp" ]]; then
    rm -rf -- "$prefix" "$sources" \
        "$build_root/glib-build" "$build_root/wayland-build" \
        "$build_root/protocols-build" "$build_root/cairo-build" \
        "$build_root/harfbuzz-build" "$build_root/pango-build" \
        "$build_root/gtk-build" "$build_root/adw-build" "$build_root/layer-build"
    mkdir -p "$prefix" "$sources"
    tar -xJf "$glib_archive" -C "$sources"
    tar -xJf "$wayland_archive" -C "$sources"
    tar -xJf "$protocols_archive" -C "$sources"
    tar -xJf "$cairo_archive" -C "$sources"
    tar -xJf "$harfbuzz_archive" -C "$sources"
    tar -xJf "$pango_archive" -C "$sources"
    tar -xJf "$gtk_archive" -C "$sources"
    tar -xJf "$adw_archive" -C "$sources"
    tar -xzf "$layer_archive" -C "$sources"

    meson setup "$build_root/wayland-build" "$sources/wayland-$wayland_version" \
        --prefix="$prefix" --libdir=lib --buildtype=release \
        -Dtests=false -Ddocumentation=false -Ddtd_validation=false
    meson compile -C "$build_root/wayland-build"
    meson install -C "$build_root/wayland-build"

    export PATH="$prefix/bin:$PATH"
    export PKG_CONFIG_PATH="$prefix/lib/pkgconfig:$prefix/share/pkgconfig:${PKG_CONFIG_PATH:-}"
    export LD_LIBRARY_PATH="$prefix/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

    meson setup "$build_root/protocols-build" \
        "$sources/wayland-protocols-$protocols_version" \
        --prefix="$prefix" --libdir=lib --buildtype=release -Dtests=false
    meson install -C "$build_root/protocols-build"

    meson setup "$build_root/glib-build" "$sources/glib-$glib_version" \
        --prefix="$prefix" --libdir=lib --buildtype=release \
        -Dselinux=disabled -Dlibmount=disabled -Dman-pages=disabled \
        -Ddtrace=disabled -Dsystemtap=disabled -Dsysprof=disabled \
        -Ddocumentation=false -Dtests=false -Dinstalled_tests=false \
        -Dintrospection=disabled
    meson compile -C "$build_root/glib-build"
    meson install -C "$build_root/glib-build"

    meson setup "$build_root/cairo-build" "$sources/cairo-$cairo_version" \
        --prefix="$prefix" --libdir=lib --buildtype=release \
        -Dtests=disabled -Dspectre=disabled -Dlzo=disabled \
        -Dgtk2-utils=disabled -Dsymbol-lookup=disabled -Dglib=enabled
    meson compile -C "$build_root/cairo-build"
    meson install -C "$build_root/cairo-build"

    meson setup "$build_root/harfbuzz-build" "$sources/harfbuzz-$harfbuzz_version" \
        --prefix="$prefix" --libdir=lib --buildtype=release \
        -Dtests=disabled -Ddocs=disabled -Dintrospection=disabled \
        -Dchafa=disabled -Dicu=disabled -Dgraphite2=disabled \
        -Dglib=enabled -Dgobject=enabled -Dcairo=enabled -Dfreetype=enabled
    meson compile -C "$build_root/harfbuzz-build"
    meson install -C "$build_root/harfbuzz-build"

    meson setup "$build_root/pango-build" "$sources/pango-$pango_version" \
        --prefix="$prefix" --libdir=lib --buildtype=release \
        -Ddocumentation=false -Dman-pages=false -Dintrospection=disabled \
        -Dbuild-testsuite=false -Dbuild-examples=false -Dsysprof=disabled
    meson compile -C "$build_root/pango-build"
    meson install -C "$build_root/pango-build"

    meson setup "$build_root/gtk-build" "$sources/gtk-$gtk_version" \
        --prefix="$prefix" --libdir=lib --buildtype=release \
        -Dx11-backend=true -Dwayland-backend=true -Dbroadway-backend=false \
        -Dmedia-gstreamer=disabled -Dprint-cpdb=disabled -Dprint-cups=disabled \
        -Dvulkan=disabled -Dcloudproviders=disabled -Dsysprof=disabled \
        -Dtracker=disabled -Dcolord=disabled -Daccesskit=disabled \
        -Dintrospection=disabled -Ddocumentation=false -Dscreenshots=false \
        -Dman-pages=false -Dbuild-demos=false -Dbuild-testsuite=false \
        -Dbuild-examples=false -Dbuild-tests=false
    meson compile -C "$build_root/gtk-build"
    meson install -C "$build_root/gtk-build"

    export PATH="$prefix/bin:$PATH"
    export PKG_CONFIG_PATH="$prefix/lib/pkgconfig:$prefix/share/pkgconfig:${PKG_CONFIG_PATH:-}"
    export LD_LIBRARY_PATH="$prefix/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

    meson setup "$build_root/adw-build" "$sources/libadwaita-$adw_version" \
        --prefix="$prefix" --libdir=lib --buildtype=release \
        -Dintrospection=disabled -Dvapi=false -Ddocumentation=false \
        -Dtests=false -Dexamples=false
    meson compile -C "$build_root/adw-build"
    meson install -C "$build_root/adw-build"

    meson setup "$build_root/layer-build" "$sources/gtk4-layer-shell-$layer_version" \
        --prefix="$prefix" --libdir=lib --buildtype=release \
        -Dexamples=false -Ddocs=false -Dtests=false -Dsmoke-tests=false \
        -Dintrospection=false -Dvapi=false
    meson compile -C "$build_root/layer-build"
    meson install -C "$build_root/layer-build"
    printf '%s\n' "$expected_stamp" >"$deps_stamp"
fi

export PATH="$prefix/bin:$PATH"
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig:$prefix/share/pkgconfig:${PKG_CONFIG_PATH:-}"
export LD_LIBRARY_PATH="$prefix/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD)"
export SOURCE_DATE_EPOCH
export CARGO_INCREMENTAL=0
export RUSTFLAGS="--remap-path-prefix=$root=/usr/src/jamelade"

cargo build --locked --release

rm -rf -- "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/jamelade/sidecar/node_modules/electron/dist"
install -Dm755 target/release/jamelade "$appdir/usr/bin/jamelade"
make PREFIX="$appdir/usr" dev-install

sidecar="$appdir/usr/share/jamelade/sidecar"
for source in package.json main.js preload.js page-hook.js security.js auth-preload.js session-vault.js login-email.js login-email-assist.js persistence.js; do
    install -Dm644 "sidecar/$source" "$sidecar/$source"
done
unzip -q "$electron_archive" -d "$sidecar/node_modules/electron/dist"
chmod 0755 "$sidecar/node_modules/electron/dist/electron"

if [[ -d "$prefix/share" ]]; then
    cp -a "$prefix/share/." "$appdir/usr/share/"
fi
if [[ -d "$appdir/usr/share/glib-2.0/schemas" ]]; then
    glib-compile-schemas "$appdir/usr/share/glib-2.0/schemas"
fi

pixbuf_module_dir="$(pkg-config --variable=gdk_pixbuf_moduledir gdk-pixbuf-2.0)"
pixbuf_binary_dir="$(pkg-config --variable=gdk_pixbuf_binarydir gdk-pixbuf-2.0)"
pixbuf_query_loaders="$pixbuf_binary_dir/gdk-pixbuf-query-loaders"
if [[ ! -x "$pixbuf_query_loaders" ]]; then
    pixbuf_query_loaders="$(dirname "$pixbuf_binary_dir")/gdk-pixbuf-query-loaders"
fi
test -d "$pixbuf_module_dir"
test -x "$pixbuf_query_loaders"
mkdir -p "$appdir/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders"
cp -a "$pixbuf_module_dir/." "$appdir/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders/"
install -Dm755 "$pixbuf_query_loaders" \
    "$appdir/usr/bin/gdk-pixbuf-query-loaders"

licences="$appdir/usr/share/licenses/io.github.Jamelade.Jamelade"
install -Dm644 COPYING "$licences/jamelade/COPYING"
install -Dm644 LEGAL.md "$appdir/usr/share/doc/jamelade/LEGAL.md"
install -Dm644 PRIVACY.md "$appdir/usr/share/doc/jamelade/PRIVACY.md"
install -Dm644 "$sources/glib-$glib_version/COPYING" "$licences/glib/COPYING"
install -Dm644 "$sources/wayland-$wayland_version/COPYING" "$licences/wayland/COPYING"
install -Dm644 "$sources/wayland-protocols-$protocols_version/COPYING" \
    "$licences/wayland-protocols/COPYING"
install -Dm644 "$sources/cairo-$cairo_version/COPYING" "$licences/cairo/COPYING"
install -Dm644 "$sources/cairo-$cairo_version/COPYING-LGPL-2.1" \
    "$licences/cairo/COPYING-LGPL-2.1"
install -Dm644 "$sources/cairo-$cairo_version/COPYING-MPL-1.1" \
    "$licences/cairo/COPYING-MPL-1.1"
install -Dm644 "$sources/harfbuzz-$harfbuzz_version/COPYING" "$licences/harfbuzz/COPYING"
install -Dm644 "$sources/pango-$pango_version/COPYING" "$licences/pango/COPYING"
install -Dm644 "$sources/gtk-$gtk_version/COPYING" "$licences/gtk/COPYING"
install -Dm644 "$sources/libadwaita-$adw_version/COPYING" "$licences/libadwaita/COPYING"
install -Dm644 "$sources/gtk4-layer-shell-$layer_version/LICENSE" \
    "$licences/gtk4-layer-shell/LICENSE"
install -Dm644 "$sidecar/node_modules/electron/dist/LICENSE" "$licences/electron/LICENSE"
install -Dm644 "$sidecar/node_modules/electron/dist/LICENSES.chromium.html" \
    "$licences/electron/LICENSES.chromium.html"
install -Dm644 "$appimage_runtime_license" "$licences/appimage-runtime/LICENSE"

vendor="$build_root/cargo-vendor"
rm -rf -- "$vendor"
cargo vendor --locked --versioned-dirs "$vendor" >/dev/null
./scripts/install-cargo-licenses.sh "$vendor" "$licences/rust-crates"

desktop-file-validate data/io.github.Jamelade.Jamelade.Launcher.desktop
appstreamcli validate --pedantic data/io.github.Jamelade.Jamelade.metainfo.xml

appimage="$output/Jamelade-$version-x86_64.AppImage"
rm -f -- "$appimage" "$output/APPIMAGE-BUILDINFO.txt" "$output/SHA256SUMS"

export NO_STRIP=1
"$linuxdeploy" --appimage-extract-and-run \
    --appdir "$appdir" \
    --executable "$appdir/usr/bin/jamelade" \
    --executable "$appdir/usr/bin/gdk-pixbuf-query-loaders" \
    --deploy-deps-only "$sidecar/node_modules/electron/dist" \
    --deploy-deps-only "$appdir/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders" \
    --desktop-file data/io.github.Jamelade.Jamelade.Launcher.desktop \
    --icon-file data/icons/hicolor/512x512/apps/io.github.Jamelade.Jamelade.png \
    --custom-apprun packaging/appimage/AppRun

# Copy copyright notices for system libraries deployed by linuxdeploy. Custom
# GTK and Electron notices are installed explicitly above.
system_licences="$licences/system-libraries"
mkdir -p "$system_licences"
printf 'library\tUbuntu package\n' >"$system_licences/PACKAGES.tsv"
while IFS= read -r library; do
    soname="$(basename "$library")"
    host_path="$(ldconfig -p | awk -v name="$soname" \
        '$1 == name && !found {print $NF; found=1}')"
    [[ -n "$host_path" ]] || continue
    host_path="$(readlink -f "$host_path")"
    [[ -f "$host_path" ]] || continue
    owner="$(dpkg-query -S "$host_path" 2>/dev/null || true)"
    owner="${owner%%$'\n'*}"
    package="${owner%%:*}"
    [[ -n "$package" ]] || continue
    package_dir="${package%%:*}"
    copyright="/usr/share/doc/$package_dir/copyright"
    [[ -f "$copyright" ]] || continue
    install -Dm644 "$copyright" "$system_licences/$package_dir/copyright"
    printf '%s\t%s\n' "$soname" "$package" >>"$system_licences/PACKAGES.tsv"
done < <(find "$appdir/usr/lib" \( -type f -o -type l \) | sort)
if [[ "$(wc -l <"$system_licences/PACKAGES.tsv")" -le 1 ]] \
    || ! find "$system_licences" -mindepth 2 -name copyright \
        -print -quit | grep -q .; then
    printf 'AppImage build: system-library licence inventory is empty\n' >&2
    exit 1
fi

export LDAI_OUTPUT="$appimage"
export LDAI_RUNTIME_FILE="$appimage_runtime"
"$linuxdeploy" --appimage-extract-and-run \
    --appdir "$appdir" \
    --output appimage
test -f "$appimage"
chmod 0755 "$appimage"
# appimagetool writes the package's MD5 into the runtime's dedicated 16-byte
# .digest_md5 section. Every other runtime byte must remain identical.
runtime_digest_offset=932096
runtime_digest_size=16
runtime_size="$(stat -c '%s' "$appimage_runtime")"
if ! cmp -n "$runtime_digest_offset" "$appimage_runtime" "$appimage" \
    || ! cmp -i "$((runtime_digest_offset + runtime_digest_size))" \
        -n "$((runtime_size - runtime_digest_offset - runtime_digest_size))" \
        "$appimage_runtime" "$appimage"; then
    printf 'AppImage build: packaged runtime does not match the pinned input\n' >&2
    exit 1
fi

extract="$build_root/extracted"
rm -rf -- "$extract"
mkdir -p "$extract"
(
    cd "$extract"
    "$appimage" --appimage-extract >/dev/null
)
extracted="$extract/squashfs-root"
test -x "$extracted/usr/bin/jamelade"
test -x "$extracted/usr/share/jamelade/sidecar/node_modules/electron/dist/electron"
test -f "$extracted/usr/share/licenses/io.github.Jamelade.Jamelade/jamelade/COPYING"
test -f "$extracted/usr/share/licenses/io.github.Jamelade.Jamelade/electron/LICENSES.chromium.html"
test -f "$extracted/usr/share/licenses/io.github.Jamelade.Jamelade/appimage-runtime/LICENSE"
test -f "$extracted/usr/share/licenses/io.github.Jamelade.Jamelade/rust-crates/CRATES.tsv"
if find "$extracted" -type f \
    \( -iname 'libwidevinecdm.so' -o -iname '*widevine*cdm*' \
       -o -iname '*.ttf' -o -iname '*.otf' -o -iname '*.ttc' \) \
    -print -quit | grep -q .; then
    printf 'AppImage build: prohibited Widevine CDM or bundled font entered the package\n' >&2
    exit 1
fi
if grep -F -- '--no-sandbox' packaging/appimage/AppRun >/dev/null; then
    printf 'AppImage build: Chromium sandbox disabling is prohibited\n' >&2
    exit 1
fi
for executable in \
    "$extracted/usr/bin/jamelade" \
    "$extracted/usr/share/jamelade/sidecar/node_modules/electron/dist/electron"; do
    if LD_LIBRARY_PATH="$extracted/usr/lib" ldd "$executable" | grep -F 'not found'; then
        printf 'AppImage build: unresolved runtime dependency in %s\n' "$executable" >&2
        exit 1
    fi
done

{
    printf 'Jamelade %s\n' "$version"
    printf 'Git commit: %s\n' "$(git rev-parse HEAD)"
    printf 'Target: x86_64 AppImage, Ubuntu 24.04 / glibc 2.39 floor\n'
    printf 'Rust: %s\n' "$(rustc --version)"
    printf 'GTK: %s\n' "$(pkg-config --modversion gtk4)"
    printf 'libadwaita: %s\n' "$(pkg-config --modversion libadwaita-1)"
    printf 'Electron: %s\n' "$electron_version"
    printf 'AppImage runtime: %s (%s)\n' \
        "$appimage_runtime_version" "$appimage_runtime_sha"
    printf 'Chromium renderer sandbox disabled: no\n'
    printf 'Signed AppImage: no\n'
} >"$output/APPIMAGE-BUILDINFO.txt"

(
    cd "$output"
    sha256sum "$(basename "$appimage")" APPIMAGE-BUILDINFO.txt >SHA256SUMS
)

printf 'AppImage release candidate written to %s\n' "$output"
