#!/bin/sh
# Wrapper: translate Rust's target triple to zig's format, or strip it
args=""
for arg in "$@"; do
    case "$arg" in
        --target=x86_64-unknown-linux-gnu)
            args="$args --target=x86_64-linux-gnu"
            ;;
        *)
            args="$args $arg"
            ;;
    esac
done
exec /home/jay/.local/bin/zig cc $args
