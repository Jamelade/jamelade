#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Jamelade contributors
# SPDX-License-Identifier: GPL-3.0-or-later

# Preserve the licence evidence shipped in every vendored Rust crate. Flatpak
# statically links many of these crates, so keeping only Cargo.lock is not a
# sufficient binary-distribution record.

set -euo pipefail
export LC_ALL=C
umask 022

source_dir="${1:?usage: install-cargo-licenses.sh VENDOR_DIR DEST_DIR}"
destination="${2:?usage: install-cargo-licenses.sh VENDOR_DIR DEST_DIR}"

[[ -d "$source_dir" ]] || {
    printf 'cargo licence install: missing vendor directory: %s\n' "$source_dir" >&2
    exit 1
}

mkdir -p "$destination"
manifest="$destination/CRATES.tsv"
printf 'crate\tdeclared licence\trepository\n' >"$manifest"
count=0

for crate_dir in "$source_dir"/*; do
    [[ -d "$crate_dir" ]] || continue
    crate="$(basename "$crate_dir")"
    [[ "$crate" =~ ^[A-Za-z0-9._+-]+$ ]] || {
        printf 'cargo licence install: unsafe crate directory: %s\n' "$crate" >&2
        exit 1
    }

    target="$destination/$crate"
    mkdir -p "$target"

    for metadata in Cargo.toml.orig Cargo.toml; do
        if [[ -f "$crate_dir/$metadata" ]]; then
            install -Dm644 "$crate_dir/$metadata" "$target/$metadata"
        fi
    done

    copied=0
    while IFS= read -r -d '' notice; do
        relative="${notice#"$crate_dir"/}"
        install -Dm644 "$notice" "$target/$relative"
        copied=1
    done < <(
        find "$crate_dir" -maxdepth 2 -type f \
            \( -iname 'license*' -o -iname 'licence*' \
               -o -iname 'copying*' -o -iname 'notice*' \
               -o -iname 'copyright*' -o -iname 'unlicense*' \) \
            -print0
    )

    # A few crates declare a licence in Cargo.toml but omit its full text from
    # the crates.io archive. Retain their README as additional provenance and
    # make the omission visible in CRATES.tsv instead of silently inventing a
    # notice on the crate author's behalf.
    if (( copied == 0 )) && [[ -f "$crate_dir/README.md" ]]; then
        install -Dm644 "$crate_dir/README.md" "$target/README.md"
    fi

    cargo_toml="$crate_dir/Cargo.toml"
    licence="$(sed -n 's/^license = "\([^"]*\)"/\1/p' "$cargo_toml" | head -n 1)"
    licence_file="$(sed -n 's/^license-file = "\([^"]*\)"/file: \1/p' "$cargo_toml" | head -n 1)"
    repository="$(sed -n 's/^repository = "\([^"]*\)"/\1/p' "$cargo_toml" | head -n 1)"
    [[ -n "$licence" ]] || licence="$licence_file"
    [[ -n "$licence" ]] || licence='not declared'
    printf '%s\t%s\t%s\n' "$crate" "$licence" "$repository" >>"$manifest"
    count=$((count + 1))
done

(( count > 0 )) || {
    printf 'cargo licence install: no crates found\n' >&2
    exit 1
}
printf '%s\n' "$count" >"$destination/CRATE_COUNT"
