#!/bin/sh
# Capsem installer -- downloads the latest release and installs it.
#   macOS: downloads .dmg, copies to /Applications
#   Linux: downloads .deb, installs via apt
# Usage: curl -fsSL https://capsem.org/install.sh | sh
set -eu

REPO="google/capsem"

# -- Testable functions ------------------------------------------------------
# These functions can be unit-tested by sourcing this script with
# __INSTALL_SH_SOURCED=1, which skips the main entry point.

detect_os() {
    _KERNEL="$(uname -s)"
    case "$_KERNEL" in
        Darwin) OS="darwin" ;;
        Linux)  OS="linux" ;;
        *)
            echo "Error: unsupported operating system: $_KERNEL. Capsem supports macOS and Linux." >&2
            return 1
            ;;
    esac
}

detect_arch() {
    _MACHINE="$(uname -m)"
    case "$OS" in
        darwin)
            case "$_MACHINE" in
                arm64) ARCH="arm64" ;;
                *)
                    echo "Error: Capsem requires Apple Silicon (arm64). Detected: $_MACHINE" >&2
                    return 1
                    ;;
            esac
            ;;
        linux)
            case "$_MACHINE" in
                x86_64|amd64)    ARCH="amd64" ;;
                aarch64|arm64)   ARCH="arm64" ;;
                *)
                    echo "Error: unsupported architecture: $_MACHINE. Capsem supports x86_64 and arm64." >&2
                    return 1
                    ;;
            esac
            ;;
    esac
}

find_asset_url() {
    _release_json="$1"
    _os="$2"
    _arch="$3"
    case "$_os" in
        darwin)
            _pattern='_aarch64\.dmg"'
            ;;
        linux)
            _pattern="_${_arch}\.deb\""
            ;;
    esac
    ASSET_URL="$(echo "$_release_json" | grep '"browser_download_url"' | grep "$_pattern" | head -1 | sed 's/.*"browser_download_url": *"//;s/".*//')"
    if [ -z "$ASSET_URL" ]; then
        echo "Error: no matching asset found for $_os/$_arch in this release." >&2
        return 1
    fi
}

install_macos() {
    _dmg_url="$1"
    _version="$2"

    TMPDIR_INSTALL="$(mktemp -d)"
    DMG_PATH="${TMPDIR_INSTALL}/Capsem.dmg"

    cleanup_macos() {
        if [ -n "${MOUNT_POINT:-}" ] && [ -d "$MOUNT_POINT" ]; then
            hdiutil detach "$MOUNT_POINT" -quiet 2>/dev/null || true
        fi
        rm -rf "$TMPDIR_INSTALL"
    }
    trap cleanup_macos EXIT

    echo "Downloading $_dmg_url..."
    curl -fSL --progress-bar -o "$DMG_PATH" "$_dmg_url"

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
    echo "Capsem $_version installed to /Applications/$APP_NAME"
    echo "Run it with: open /Applications/$APP_NAME"
}

install_linux() {
    _deb_url="$1"
    _version="$2"

    TMPDIR_INSTALL="$(mktemp -d)"
    DEB_PATH="${TMPDIR_INSTALL}/capsem.deb"

    cleanup_linux() {
        rm -rf "$TMPDIR_INSTALL"
    }
    trap cleanup_linux EXIT

    echo "Downloading $_deb_url..."
    curl -fSL --progress-bar -o "$DEB_PATH" "$_deb_url"

    echo "Installing .deb package (may prompt for sudo password)..."
    sudo apt install -y "$DEB_PATH"

    echo ""
    echo "Capsem $_version installed."
    echo "Run it with: capsem"
}

# -- Main entry point --------------------------------------------------------

if [ "${__INSTALL_SH_SOURCED:-}" = "1" ]; then
    return 0 2>/dev/null || true
fi

# Preflight: curl required
if ! command -v curl >/dev/null 2>&1; then
    echo "Error: curl is required but not found." >&2
    exit 1
fi

# Detect platform
detect_os
detect_arch

# OS-specific preflight
case "$OS" in
    darwin)
        MACOS_VERSION="$(sw_vers -productVersion)"
        MACOS_MAJOR="$(echo "$MACOS_VERSION" | cut -d. -f1)"
        if [ "$MACOS_MAJOR" -lt 14 ]; then
            echo "Error: Capsem requires macOS 14 (Sonoma) or later. Detected: $MACOS_VERSION" >&2
            exit 1
        fi
        ;;
    linux)
        if ! command -v apt >/dev/null 2>&1; then
            echo "Error: apt is required for installation. Capsem provides .deb packages for Debian/Ubuntu." >&2
            exit 1
        fi
        ;;
esac

# Fetch latest release
echo "Fetching latest Capsem release..."
RELEASE_JSON="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest")"

TAG="$(echo "$RELEASE_JSON" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"//;s/".*//')"
if [ -z "$TAG" ]; then
    echo "Error: could not determine latest release tag." >&2
    exit 1
fi

VERSION="${TAG#v}"
echo "Installing Capsem $VERSION..."

# Find the right asset
find_asset_url "$RELEASE_JSON" "$OS" "$ARCH"

# Install
case "$OS" in
    darwin) install_macos "$ASSET_URL" "$VERSION" ;;
    linux)  install_linux "$ASSET_URL" "$VERSION" ;;
esac
