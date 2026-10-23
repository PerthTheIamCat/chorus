#!/usr/bin/env bash
# Developer ID sign + notarize + staple a built Chorus.app. NOT YET RUN — it
# needs the maintainer's Developer ID, which this repo deliberately does not
# contain. Nothing secret is read or written here: the identity is a name in
# the login Keychain and the notary credentials live in a notarytool profile.
#
# One-time setup (on the maintainer's Mac):
#   xcrun notarytool store-credentials chorus-notary \
#       --apple-id <apple id> --team-id <TEAMID>      # prompts for an app-specific password
#
# Then:
#   CHORUS_SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" \
#   CHORUS_NOTARY_PROFILE=chorus-notary \
#   ./notarize.sh path/to/Chorus.app
#
# Order matters: nested code is signed first, inside-out, each with the
# hardened runtime and a secure timestamp (notarization rejects anything
# else); the outer bundle last.
set -euo pipefail

APP="${1:?usage: notarize.sh <Chorus.app>}"
ID="${CHORUS_SIGN_IDENTITY:?set CHORUS_SIGN_IDENTITY to your \"Developer ID Application: …\" identity}"
PROFILE="${CHORUS_NOTARY_PROFILE:?set CHORUS_NOTARY_PROFILE to your notarytool keychain profile}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ENT="$HERE/Chorus.entitlements"

sign() { codesign --force --options runtime --timestamp --sign "$ID" "$@"; }

echo "── sign nested code ──"
# Sparkle ships XPC services and helpers inside its framework; sign deepest first.
SPARKLE="$APP/Contents/Frameworks/Sparkle.framework"
if [ -d "$SPARKLE" ]; then
    find "$SPARKLE" -type d \( -name "*.xpc" -o -name "*.app" \) -print0 | sort -rz |
        while IFS= read -r -d '' item; do sign "$item"; done
    [ -e "$SPARKLE/Versions/B/Autoupdate" ] && sign "$SPARKLE/Versions/B/Autoupdate"
    sign "$SPARKLE"
fi
for helper in "$APP/Contents/MacOS/antigravity-acp" "$APP/Contents/MacOS/chorusd"; do
    [ -e "$helper" ] && sign --entitlements "$ENT" "$helper"
done
echo "── sign app ──"
sign --entitlements "$ENT" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

echo "── notarize (waits for Apple) ──"
ZIP="$(mktemp -d)/Chorus.zip"
ditto -c -k --keepParent "$APP" "$ZIP"
xcrun notarytool submit "$ZIP" --keychain-profile "$PROFILE" --wait

echo "── staple ──"
xcrun stapler staple "$APP"
xcrun stapler validate "$APP"
spctl --assess --type execute --verbose "$APP"
echo "ok: $APP is signed, notarized and stapled. Package it with package-dmg.sh next."
