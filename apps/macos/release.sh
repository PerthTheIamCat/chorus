#!/usr/bin/env bash
# Build, sign and publish a Chorus release that Sparkle can auto-update to.
#
#   ./release.sh 0.1.0          build + sign + appcast, stop before publishing
#   ./release.sh 0.1.0 publish  ...then create the GitHub Release v0.1.0
#
# The EdDSA private key is read from the login Keychain (account "chorus",
# created once with `generate_keys --account chorus`). It is never written to
# disk or printed by this script.
set -euo pipefail

cd "$(dirname "$0")"
VERSION="${1:?usage: release.sh <version> [publish]}"
PUBLISH="${2:-}"
REPO="PerthTheIamCat/chorus"
DIST="dist"
BIN=".build/artifacts/sparkle/Sparkle/bin"

./package-dmg.sh "$VERSION"
DMG="$DIST/Chorus-$VERSION.dmg"

# generate_appcast scans a folder, so give it one that holds only this DMG.
FEED_DIR="$DIST/appcast"
rm -rf "$FEED_DIR" && mkdir -p "$FEED_DIR"
cp "$DMG" "$FEED_DIR/"

# Release notes = this version's section of CHANGELOG.md. Saved next to the
# DMG with the same name so generate_appcast embeds it (Sparkle shows it in
# the update window), and reused for the GitHub Release body.
NOTES="$FEED_DIR/Chorus-$VERSION.md"
awk -v heading="## [$VERSION]" '
    index($0, heading) == 1 { in_section = 1; next }
    in_section && /^## \[/ { exit }
    in_section { print }
' ../../CHANGELOG.md > "$NOTES"
if ! grep -q '[^[:space:]]' "$NOTES"; then
    echo "CHANGELOG.md has no '## [$VERSION]' section — add one before releasing" >&2
    exit 1
fi
# CI passes the private key via SPARKLE_ED_KEY (a repo secret) on stdin so it
# never touches disk or argv; locally it is read from the login Keychain.
if [ -n "${SPARKLE_ED_KEY:-}" ]; then
    printf '%s' "$SPARKLE_ED_KEY" | "$BIN/generate_appcast" --ed-key-file - --embed-release-notes \
        --download-url-prefix "https://github.com/$REPO/releases/download/v$VERSION/" "$FEED_DIR"
else
"$BIN/generate_appcast" --account chorus --embed-release-notes \
    --download-url-prefix "https://github.com/$REPO/releases/download/v$VERSION/" \
    "$FEED_DIR"
fi
echo "appcast: $FEED_DIR/appcast.xml"

if [ "$PUBLISH" = "publish" ]; then
    gh release create "v$VERSION" "$DMG" "$FEED_DIR/appcast.xml" \
        --repo "$REPO" --title "Chorus $VERSION" \
        --notes "$(cat "$NOTES")

---
ไม่ได้ notarize: ครั้งแรกให้คลิกขวา → Open (หรือ xattr -dr com.apple.quarantine Chorus.app) การอัปเดตหลังจากนั้นทำในแอปได้เลย (Chorus → Check for Updates…)"
fi
