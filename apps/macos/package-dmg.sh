#!/usr/bin/env bash
# Package Chorus into a distributable, compressed macOS Disk Image (.dmg)
#
# Usage:
#   ./package-dmg.sh [version]
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VERSION="${1:-$(git -C "$SCRIPT_DIR" describe --tags --abbrev=0 2>/dev/null | sed 's/^v//')}"
VERSION="${VERSION:?no version given and no git tag found}"
export CHORUS_VERSION="$VERSION"
DMG_NAME="Chorus-${VERSION}.dmg"
DIST_DIR="${SCRIPT_DIR}/dist"
STAGE_DIR="${DIST_DIR}/stage"

echo "=========================================================="
echo "          CHORUS MACOS DISK IMAGE PACKAGER (.dmg)         "
echo "=========================================================="

# 1. Build release app bundle
echo "==> Building Chorus Release bundle..."
"${SCRIPT_DIR}/build.sh" release no-run

# Use the exact output of this release build, never a previously installed app.
APP_SOURCE="$(cd "${SCRIPT_DIR}" && swift build -c release --show-bin-path)/Chorus.app"

if [ ! -d "${APP_SOURCE}" ]; then
    echo "Error: Release bundle not found at ${APP_SOURCE}"
    exit 1
fi

echo "==> Found bundle at: ${APP_SOURCE}"

# 2. Prepare staging directory
rm -rf "${STAGE_DIR}"
mkdir -p "${STAGE_DIR}" "${DIST_DIR}"

echo "==> Staging app bundle and /Applications symlink..."
cp -R "${APP_SOURCE}" "${STAGE_DIR}/Chorus.app"
ln -s /Applications "${STAGE_DIR}/Applications"

# 3. Code sign check
echo "==> Verifying code signature..."
codesign --verify --deep --strict "${STAGE_DIR}/Chorus.app"

# 4. Create DMG using hdiutil
OUTPUT_DMG="${DIST_DIR}/${DMG_NAME}"
rm -f "${OUTPUT_DMG}"

echo "==> Generating compressed UDZO disk image: ${OUTPUT_DMG}"
hdiutil create -volname "Chorus" \
               -srcfolder "${STAGE_DIR}" \
               -ov \
               -format UDZO \
               "${OUTPUT_DMG}"

# Clean up stage
rm -rf "${STAGE_DIR}"

# 5. Output checksum
echo "==> Calculating SHA-256 checksum..."
SHA="$(shasum -a 256 "${OUTPUT_DMG}" | awk '{print $1}')"

echo "=========================================================="
echo "Packaging complete!"
echo "Artifact: ${OUTPUT_DMG}"
echo "SHA-256:  ${SHA}"
echo "=========================================================="
