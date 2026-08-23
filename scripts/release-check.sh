#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Jamelade contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# Build a reviewable Jamelade release candidate from a clean commit.
#
# RELEASE_ALLOW_DEV=1 permits the current -dev version for a local candidate.
# RELEASE_ALLOW_DIRTY=1 exists only for throwaway diagnostics: the source
# archive still comes from HEAD, so never publish an artefact made that way.
# RELEASE_SKIP_SOURCE_CHECKS=1 is reserved for CI after its isolated check job.
# REPRO_CHECK=1 repeats the complete Flatpak build in a fresh directory and
# refuses a byte-different unsigned bundle.
# FLATPAK_GPG_KEY=<id> signs the exported Flatpak commit when a maintained
# release key exists; checksums should still be signed separately at publish.

set -euo pipefail
export LC_ALL=C
export TZ=UTC
umask 022

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
manifest="$root/packaging/flatpak/io.github.Jamelade.Jamelade.yml"

release_epoch="$(sed -n "s/^[[:space:]]*SOURCE_DATE_EPOCH: ['\"]\([0-9][0-9]*\)['\"]$/\1/p" "$manifest" | head -n 1)"
if [[ ! "$release_epoch" =~ ^[0-9]+$ ]]; then
    printf 'release check: manifest needs one numeric SOURCE_DATE_EPOCH\n' >&2
    exit 1
fi
export SOURCE_DATE_EPOCH="$release_epoch"

if ! command -v cargo >/dev/null 2>&1; then
    data_home="${XDG_DATA_HOME:-${HOME}/.local/share}"
    flatpak_rust="$data_home/flatpak/runtime/org.freedesktop.Sdk.Extension.rust-stable/x86_64/25.08/active/files/bin"
    if [[ -x "$flatpak_rust/cargo" ]]; then
        export PATH="$flatpak_rust:$PATH"
    fi
fi

for tool in cargo rustc git node npm flatpak appstreamcli desktop-file-validate jq sha256sum gzip cmp; do
    command -v "$tool" >/dev/null 2>&1 || {
        printf 'release check: missing required tool: %s\n' "$tool" >&2
        exit 1
    }
done

builder=(flatpak run org.flatpak.Builder)
if command -v flatpak-builder >/dev/null 2>&1; then
    builder=(flatpak-builder)
fi

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
[[ -n "$version" ]] || {
    printf 'release check: could not read Cargo version\n' >&2
    exit 1
}
if [[ "$version" == *-dev* && "${RELEASE_ALLOW_DEV:-0}" != 1 ]]; then
    printf 'release check: %s is a development version; set a release/RC version or use RELEASE_ALLOW_DEV=1 locally\n' "$version" >&2
    exit 1
fi

if [[ -n "$(git status --porcelain)" && "${RELEASE_ALLOW_DIRTY:-0}" != 1 ]]; then
    printf 'release check: working tree is not clean\n' >&2
    exit 1
fi

if [[ "${REPRO_CHECK:-0}" == 1 && -n "${FLATPAK_GPG_KEY:-}" ]]; then
    printf 'release check: reproducibility comparison must use an unsigned build\n' >&2
    exit 1
fi

git diff --check
if [[ "${RELEASE_SKIP_SOURCE_CHECKS:-0}" != 1 ]]; then
    make check
fi
desktop-file-validate \
    data/io.github.Jamelade.Jamelade.desktop \
    data/io.github.Jamelade.Jamelade.Launcher.desktop
appstreamcli validate --pedantic data/io.github.Jamelade.Jamelade.metainfo.xml
"${builder[@]}" --show-manifest "$manifest" >/dev/null
jq empty packaging/flatpak/cargo-sources.json sidecar/package-lock.json

output="$root/dist"
repo="$root/release-repo"
mkdir -p "$output"

# Start from an empty generated repository. Reusing an older OSTree repository
# can make otherwise identical bundles differ through unrelated retained refs.
if [[ "$repo" != "$root/release-repo" ]]; then
    printf 'release check: refusing unexpected repository path\n' >&2
    exit 1
fi
rm -rf -- "$repo"
mkdir -p "$repo"

sign_args=()
if [[ -n "${FLATPAK_GPG_KEY:-}" ]]; then
    sign_args+=("--gpg-sign=$FLATPAK_GPG_KEY")
fi

"${builder[@]}" --override-source-date-epoch="$release_epoch" --force-clean --user \
    --repo="$repo" "${sign_args[@]}" \
    "$root/build-dir" "$manifest"

bundle_name="Jamelade-${version}-x86_64.flatpak"
source_name="jamelade-${version}.tar.gz"
bundle="$output/$bundle_name"
source_archive="$output/$source_name"
rm -f -- "$bundle" "$source_archive" "$output/BUILDINFO.txt" "$output/SHA256SUMS"

flatpak build-bundle \
    --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
    "$repo" "$bundle" io.github.Jamelade.Jamelade master

git archive --format=tar --prefix="jamelade-${version}/" HEAD | gzip -n -9 >"$source_archive"

electron_dist="$root/build-dir/files/share/jamelade/sidecar/node_modules/electron/dist"
test -f "$electron_dist/LICENSE"
test -f "$electron_dist/LICENSES.chromium.html"
test -f "$root/build-dir/files/share/licenses/io.github.Jamelade.Jamelade/jamelade/COPYING"
test -f "$root/build-dir/files/share/licenses/io.github.Jamelade.Jamelade/gtk4-layer-shell/LICENSE"
rust_licences="$root/build-dir/files/share/licenses/io.github.Jamelade.Jamelade/rust-crates"
test -f "$rust_licences/CRATES.tsv"
test -f "$rust_licences/CRATE_COUNT"
expected_crates="$(jq '[.[] | select(.type == "archive" and (.dest // "" | startswith("cargo/vendor/")))] | length' packaging/flatpak/cargo-sources.json)"
actual_crates="$(cat "$rust_licences/CRATE_COUNT")"
if [[ ! "$actual_crates" =~ ^[0-9]+$ || "$actual_crates" -ne "$expected_crates" ]]; then
    printf 'release check: Rust licence set is incomplete (%s of %s crates)\n' \
        "$actual_crates" "$expected_crates" >&2
    exit 1
fi
test -f "$root/build-dir/files/share/doc/jamelade/LEGAL.md"
test -f "$root/build-dir/files/share/doc/jamelade/PRIVACY.md"

if find "$root/build-dir/files" -type f \
    \( -iname 'libwidevinecdm.so' -o -iname '*widevine*cdm*' \
       -o -iname '*.ttf' -o -iname '*.otf' -o -iname '*.ttc' \) \
    -print -quit | grep -q .; then
    printf 'release check: prohibited Widevine CDM or bundled font entered the app\n' >&2
    exit 1
fi

commit="$(git rev-parse HEAD)"
epoch="$(git show -s --format=%ct HEAD)"
{
    printf 'Jamelade %s\n' "$version"
    printf 'Git commit: %s\n' "$commit"
    printf 'Git commit epoch: %s\n' "$epoch"
    printf 'Target: x86_64 Flatpak, GNOME Platform 49\n'
    printf 'Rust: %s\n' "$(rustc --version)"
    printf 'Cargo: %s\n' "$(cargo --version)"
    printf 'Node: %s\n' "$(node --version)"
    printf 'Flatpak: %s\n' "$(flatpak --version)"
    printf 'Flatpak Builder: %s\n' "$("${builder[@]}" --version)"
    printf 'Signed Flatpak commit: %s\n' "$([[ -n "${FLATPAK_GPG_KEY:-}" ]] && printf yes || printf no)"
} >"$output/BUILDINFO.txt"

(
    cd "$output"
    sha256sum "$bundle_name" "$source_name" BUILDINFO.txt >SHA256SUMS
)

if [[ "${REPRO_CHECK:-0}" == 1 ]]; then
    second="$(mktemp -d -p "$root" .jamelade-repro-XXXXXX)"
    if [[ "$second" != "$root"/.jamelade-repro-* ]]; then
        printf 'release check: refusing unexpected reproduction path\n' >&2
        exit 1
    fi
    trap 'rm -rf -- "$second"' EXIT
    "${builder[@]}" --override-source-date-epoch="$release_epoch" \
        --disable-cache --force-clean \
        --user --state-dir="$second/state" --repo="$second/repo" \
        "$second/build" "$manifest"
    flatpak build-bundle \
        --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
        "$second/repo" "$second/$bundle_name" io.github.Jamelade.Jamelade master
    cmp --silent "$bundle" "$second/$bundle_name" || {
        printf 'release check: repeated Flatpak bundle is not byte-identical\n' >&2
        exit 1
    }
    git archive --format=tar --prefix="jamelade-${version}/" HEAD \
        | gzip -n -9 >"$second/$source_name"
    cmp --silent "$source_archive" "$second/$source_name" || {
        printf 'release check: repeated source archive is not byte-identical\n' >&2
        exit 1
    }
fi

printf 'release candidate written to %s\n' "$output"
printf 'verify with: (cd %s && sha256sum -c SHA256SUMS)\n' "$output"
