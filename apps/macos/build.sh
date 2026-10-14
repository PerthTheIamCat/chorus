#!/usr/bin/env bash
# Build the Rust core, then the SwiftUI shell, then wrap it in a real
# Chorus.app bundle.
#
#   ./build.sh            debug build + run
#   ./build.sh release    release build + run
#   ./build.sh debug no-run
#
# Why a bundle and not the bare `swift build` binary: a bare Mach-O has no
# bundle identity, so macOS gives it no Dock icon, no app menu with the right
# name, no window restoration, and no place to hang entitlements later. It also
# cannot be double-clicked, which is how anyone other than us would open it.
# The bundle is assembled here rather than checked in so there is exactly one
# source of truth for the version and the identifier.
set -euo pipefail

cd "$(dirname "$0")"
MODE="${1:-debug}"
RUN="${2:-run}"
VERSION="0.1.0"
BUNDLE_ID="dev.chorus.app"

echo "── cargo build ($MODE) ──"
if [ "$MODE" = "release" ]; then
    (cd ../.. && cargo build --release -p chorus-ffi -p chorus-acp --bin antigravity-acp -p chorus-daemon --bin chorusd)
    sed -i '' 's/let target = "debug"/let target = "release"/' Package.swift
else
    (cd ../.. && cargo build -p chorus-ffi -p chorus-acp --bin antigravity-acp -p chorus-daemon --bin chorusd)
    sed -i '' 's/let target = "release"/let target = "debug"/' Package.swift
fi

echo "── swift build ──"
if [ "$MODE" = "release" ]; then
    swift build -c release
    BIN="$(swift build -c release --show-bin-path)/Chorus"
else
    swift build
    BIN="$(swift build --show-bin-path)/Chorus"
fi

echo "── bundle ──"
APP="$(dirname "$BIN")/Chorus.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/Chorus"
cp -R "$(dirname "$BIN")/Chorus_Chorus.bundle" "$APP/Contents/Resources/"
if [ "$MODE" = "release" ]; then
    cp ../../target/release/antigravity-acp "$APP/Contents/MacOS/antigravity-acp"
    cp ../../target/release/chorusd "$APP/Contents/MacOS/chorusd"
else
    cp ../../target/debug/antigravity-acp "$APP/Contents/MacOS/antigravity-acp"
    cp ../../target/debug/chorusd "$APP/Contents/MacOS/chorusd"
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>              <string>Chorus</string>
    <key>CFBundleDisplayName</key>       <string>Chorus</string>
    <key>CFBundleExecutable</key>        <string>Chorus</string>
    <key>CFBundleIdentifier</key>        <string>${BUNDLE_ID}</string>
    <key>CFBundlePackageType</key>       <string>APPL</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundleVersion</key>           <string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key>    <string>13.0</string>
    <key>NSHighResolutionCapable</key>   <true/>
    <!-- Chorus spawns agent CLIs (claude, codex) as child processes and reads
         the user's repo. Both die under the App Sandbox, so it is off until
         the Seatbelt profile in the design doc is actually written. -->
    <key>LSApplicationCategoryType</key> <string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST

# Ad-hoc signature: without it macOS kills the app on some machines with
# "code has no resources but signature indicates they must be present", and
# TCC prompts (Files & Folders) have no stable identity to attach to.
# Sign nested executables first and fail if signing or verification fails.
for EXECUTABLE in "$APP/Contents/MacOS/antigravity-acp" "$APP/Contents/MacOS/chorusd"; do
    codesign --force --sign - --timestamp=none "$EXECUTABLE"
done
codesign --force --sign - --timestamp=none "$APP"
codesign --verify --deep --strict "$APP"

echo "built: $APP"

if [ -d "$HOME/Applications/Chorus.app" ] || [ -d "$HOME/Applications" ]; then
    mkdir -p "$HOME/Applications"
    rm -rf "$HOME/Applications/Chorus.app"
    cp -R "$APP" "$HOME/Applications/Chorus.app"
    echo "synced: $HOME/Applications/Chorus.app"
fi

if [ "$RUN" = "run" ]; then
    LOG="${TMPDIR:-/tmp}/chorus-app.log"
    : > "$LOG"
    echo "log:   $LOG   (tail -f เพื่อดู stderr)"
    exec open -n --stdout "$LOG" --stderr "$LOG" "$APP"
fi
