#!/bin/sh
# Capsem installer -- downloads the latest release DMG and installs to /Applications.
# Usage: curl -fsSL https://capsem.dev/install.sh | sh
set -eu

REPO="google/capsem"

# -- Preflight checks --------------------------------------------------------

if [ "$(uname -s)" != "Darwin" ]; then
    echo "Error: Capsem requires macOS." >&2
    exit 1
fi

ARCH="$(uname -m)"
if [ "$ARCH" != "arm64" ]; then
    echo "Error: Capsem requires Apple Silicon (arm64). Detected: $ARCH" >&2
    exit 1
fi

MACOS_VERSION="$(sw_vers -productVersion)"
MACOS_MAJOR="$(echo "$MACOS_VERSION" | cut -d. -f1)"
if [ "$MACOS_MAJOR" -lt 14 ]; then
    echo "Error: Capsem requires macOS 14 (Sonoma) or later. Detected: $MACOS_VERSION" >&2
    exit 1
fi

if ! command -v curl >/dev/null 2>&1; then
    echo "Error: curl is required but not found." >&2
    exit 1
fi

# -- Resolve latest release ---------------------------------------------------

echo "Fetching latest Capsem release..."
RELEASE_JSON="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest")"

TAG="$(echo "$RELEASE_JSON" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"//;s/".*//')"
if [ -z "$TAG" ]; then
    echo "Error: could not determine latest release tag." >&2
    exit 1
fi

# Find the .dmg asset URL
DMG_URL="$(echo "$RELEASE_JSON" | grep '"browser_download_url"' | grep '\.dmg"' | head -1 | sed 's/.*"browser_download_url": *"//;s/".*//')"
if [ -z "$DMG_URL" ]; then
    echo "Error: no .dmg asset found in release $TAG." >&2
    exit 1
fi

VERSION="${TAG#v}"
echo "Installing Capsem $VERSION..."

# -- Download DMG -------------------------------------------------------------

TMPDIR_INSTALL="$(mktemp -d)"
DMG_PATH="${TMPDIR_INSTALL}/Capsem.dmg"

cleanup() {
    # Detach if still mounted
    if [ -n "${MOUNT_POINT:-}" ] && [ -d "$MOUNT_POINT" ]; then
        hdiutil detach "$MOUNT_POINT" -quiet 2>/dev/null || true
    fi
    rm -rf "$TMPDIR_INSTALL"
}
trap cleanup EXIT

echo "Downloading $DMG_URL..."
curl -fSL --progress-bar -o "$DMG_PATH" "$DMG_URL"

# -- Mount and install --------------------------------------------------------

echo "Mounting DMG..."
MOUNT_POINT="$(hdiutil attach "$DMG_PATH" -nobrowse -readonly | grep '/Volumes/' | sed 's|.*\(/Volumes/.*\)|\1|')"

if [ -z "$MOUNT_POINT" ]; then
    echo "Error: failed to mount DMG." >&2
    exit 1
fi

APP_PATH="$(find "$MOUNT_POINT" -maxdepth 1 -name '*.app' -print -quit)"
if [ -z "$APP_PATH" ]; then
    echo "Error: no .app bundle found in DMG." >&2
    exit 1
fi

APP_NAME="$(basename "$APP_PATH")"
DEST="/Applications/$APP_NAME"

# Stop running instance if any
if pgrep -x "Capsem" >/dev/null 2>&1; then
    echo "Stopping running Capsem..."
    pkill -x "Capsem" 2>/dev/null || true
    sleep 1
fi

if [ -d "$DEST" ]; then
    echo "Removing existing $APP_NAME..."
    rm -rf "$DEST"
fi

echo "Installing to /Applications..."
cp -R "$APP_PATH" /Applications/

echo "Unmounting DMG..."
hdiutil detach "$MOUNT_POINT" -quiet

echo ""
echo "Capsem $VERSION installed to /Applications/$APP_NAME"
echo "Run it with: open /Applications/$APP_NAME"
