#!/bin/sh
# Install or remove Jamelade's narrow, per-user launcher-icon helper.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
prefix=${PREFIX:-"${HOME:?}/.local"}
binary="$prefix/libexec/jamelade-icon-helper"
service="$prefix/share/dbus-1/services/io.github.Jamelade.IconHelper.service"

case ${1:-install} in
    install)
        if [ -n "${HELPER_BINARY:-}" ]; then
            built=$HELPER_BINARY
        else
            cargo build --locked --release --manifest-path "$root/icon-helper/Cargo.toml"
            built="$root/icon-helper/target/release/jamelade-icon-helper"
        fi
        install -Dm755 "$built" "$binary"
        install -d "$(dirname -- "$service")"
        temporary=$(mktemp "$service.XXXXXX")
        trap 'rm -f "$temporary"' EXIT HUP INT TERM
        printf '%s\n' \
            '[D-BUS Service]' \
            'Name=io.github.Jamelade.IconHelper' \
            "Exec=$binary --service" >"$temporary"
        chmod 644 "$temporary"
        mv -f "$temporary" "$service"
        trap - EXIT HUP INT TERM
        ;;
    uninstall)
        rm -f "$service" "$binary"
        ;;
    *)
        echo "usage: $0 [install|uninstall]" >&2
        exit 2
        ;;
esac
