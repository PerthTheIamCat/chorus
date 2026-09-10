#!/usr/bin/env bash
# Build the Rust core, then the SwiftUI shell.
#
#   ./build.sh            debug build + run
#   ./build.sh release    release build + run
#   ./build.sh debug no-run
set -euo pipefail

cd "$(dirname "$0")"
MODE="${1:-debug}"
RUN="${2:-run}"

echo "── cargo build ($MODE) ──"
if [ "$MODE" = "release" ]; then
    (cd ../.. && cargo build --release -p chorus-ffi)
    sed -i '' 's/let target = "debug"/let target = "release"/' Package.swift
else
    (cd ../.. && cargo build -p chorus-ffi)
    sed -i '' 's/let target = "release"/let target = "debug"/' Package.swift
fi

echo "── swift build ──"
if [ "$MODE" = "release" ]; then
    swift build -c release
    BIN=$(swift build -c release --show-bin-path)/Chorus
else
    swift build
    BIN=$(swift build --show-bin-path)/Chorus
fi

echo "built: $BIN"
[ "$RUN" = "run" ] && exec "$BIN"
