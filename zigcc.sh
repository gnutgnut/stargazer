#!/bin/sh
ZIG="${HOME}/.local/bin/zig"
args=""
for arg in "$@"; do
    case "$arg" in
        --target=*-unknown-linux-gnu)
            triple="$(echo "$arg" | sed 's/--target=//;s/-unknown-linux-gnu/-linux-gnu/')"
            args="$args --target=$triple"
            ;;
        *)
            args="$args $arg"
            ;;
    esac
done
exec "$ZIG" cc $args
