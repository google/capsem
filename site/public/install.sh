#!/bin/sh
# Capsem installer -- downloads the latest release and installs it.
#   macOS: downloads .pkg, installs with the native installer
#   Linux: downloads .deb, installs via apt
# Usage: curl -fsSL https://capsem.org/install.sh | sh
set -eu

REPO="google/capsem"
MANIFEST_PUBKEY='RWSbrIiyy3Cgk9Ax/nqK4QNjnClKlsaXunBHFFgVo4POGZHTkrrvwVr1'

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
                    echo "Error: Capsem on macOS requires Apple Silicon (arm64). Detected: $_MACHINE" >&2
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
            _pattern='/Capsem-[^/"]+\.pkg"'
            ;;
        linux)
            _pattern="/Capsem_[^/\"]+_${_arch}\.deb\""
            ;;
    esac
    ASSET_URL="$(echo "$_release_json" | grep '"browser_download_url"' | grep -E "$_pattern" | head -1 | sed 's/.*"browser_download_url": *"//;s/".*//')"
    if [ -z "$ASSET_URL" ]; then
        echo "Error: no matching asset found for $_os/$_arch in this release." >&2
        return 1
    fi
}

find_named_asset_url() {
    _release_json="$1"
    _asset_name="$2"
    ASSET_URL="$(echo "$_release_json" | grep '"browser_download_url"' | grep "/${_asset_name}\"" | head -1 | sed 's/.*"browser_download_url": *"//;s/".*//')"
    if [ -z "$ASSET_URL" ]; then
        echo "Error: no ${_asset_name} asset found in this release." >&2
        return 1
    fi
}

asset_name_from_url() {
    _url="$1"
    _name="${_url##*/}"
    printf '%s\n' "${_name%%\?*}"
}

write_manifest_pubkey() {
    _path="$1"
    {
        echo "untrusted comment: minisign public key 93A070CBB288AC9B"
        echo "$MANIFEST_PUBKEY"
    } > "$_path"
}

sha256_file() {
    _path="$1"
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$_path" | awk '{print $1}'
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$_path" | awk '{print $1}'
    else
        echo "Error: shasum or sha256sum is required to verify package hashes." >&2
        return 1
    fi
}

manifest_expected_sha() {
    _manifest="$1"
    _asset_name="$2"
    if ! command -v python3 >/dev/null 2>&1; then
        echo "Error: python3 is required to read the release manifest." >&2
        return 1
    fi
    python3 - "$_manifest" "$_asset_name" <<'PY'
import json
import sys
from pathlib import Path

manifest = Path(sys.argv[1])
asset_name = sys.argv[2]
data = json.loads(manifest.read_text())
current = data.get("binaries", {}).get("current")
files = data.get("binaries", {}).get("releases", {}).get(current, {}).get("files", [])
for item in files:
    if item.get("name") == asset_name:
        print(item["sha256"])
        raise SystemExit(0)
raise SystemExit(f"{asset_name} not found in manifest binaries.releases[{current}]")
PY
}

download_release_manifest() {
    _release_json="$1"
    _dest="$2"

    find_named_asset_url "$_release_json" "manifest.json"
    _manifest_url="$ASSET_URL"
    find_named_asset_url "$_release_json" "manifest.json.minisig"
    _manifest_sig_url="$ASSET_URL"

    curl -fSL --progress-bar -o "$_dest/manifest.json" "$_manifest_url"
    curl -fSL --progress-bar -o "$_dest/manifest.json.minisig" "$_manifest_sig_url"
}

verify_release_manifest() {
    _manifest="$1"
    _manifest_sig="$2"

    if ! command -v minisign >/dev/null 2>&1; then
        echo "Warning: minisign not found; skipping manifest signature verification." >&2
        return 0
    fi

    _pubkey="${_manifest}.pub"
    write_manifest_pubkey "$_pubkey"
    minisign -Vm "$_manifest" -x "$_manifest_sig" -p "$_pubkey" >/dev/null
    rm -f "$_pubkey"
}

verify_asset_hash() {
    _manifest="$1"
    _asset_path="$2"
    _asset_name="$3"

    if ! command -v python3 >/dev/null 2>&1; then
        echo "Warning: python3 not found; skipping package hash verification." >&2
        return 0
    fi
    _expected="$(manifest_expected_sha "$_manifest" "$_asset_name")"
    _actual="$(sha256_file "$_asset_path")"
    if [ "$_actual" != "$_expected" ]; then
        echo "Error: package hash mismatch for $_asset_name" >&2
        echo "  expected: $_expected" >&2
        echo "  actual:   $_actual" >&2
        return 1
    fi
}

install_macos() {
    _pkg_url="$1"
    _version="$2"

    TMPDIR_INSTALL="$(mktemp -d)"
    PKG_NAME="$(asset_name_from_url "$_pkg_url")"
    PKG_PATH="${TMPDIR_INSTALL}/${PKG_NAME}"

    cleanup_macos() {
        rm -rf "$TMPDIR_INSTALL"
    }
    trap cleanup_macos EXIT

    echo "Downloading $_pkg_url..."
    curl -fSL --progress-bar -o "$PKG_PATH" "$_pkg_url"

    download_release_manifest "$RELEASE_JSON" "$TMPDIR_INSTALL"
    verify_release_manifest "$TMPDIR_INSTALL/manifest.json" "$TMPDIR_INSTALL/manifest.json.minisig"
    verify_asset_hash "$TMPDIR_INSTALL/manifest.json" "$PKG_PATH" "$PKG_NAME"

    if command -v pkgutil >/dev/null 2>&1; then
        pkgutil --check-signature "$PKG_PATH" >/dev/null
    fi

    echo "Installing .pkg package (may prompt for sudo password)..."
    sudo installer -pkg "$PKG_PATH" -target /
    if command -v open >/dev/null 2>&1; then
        open -a Capsem >/dev/null 2>&1 || true
    fi

    echo ""
    echo "Capsem $_version installed."
    echo "Open a new terminal and run: capsem shell"
}

install_linux() {
    _deb_url="$1"
    _version="$2"

    TMPDIR_INSTALL="$(mktemp -d)"
    DEB_NAME="$(asset_name_from_url "$_deb_url")"
    DEB_PATH="${TMPDIR_INSTALL}/${DEB_NAME}"

    cleanup_linux() {
        rm -rf "$TMPDIR_INSTALL"
    }
    trap cleanup_linux EXIT

    echo "Downloading $_deb_url..."
    curl -fSL --progress-bar -o "$DEB_PATH" "$_deb_url"

    download_release_manifest "$RELEASE_JSON" "$TMPDIR_INSTALL"
    verify_release_manifest "$TMPDIR_INSTALL/manifest.json" "$TMPDIR_INSTALL/manifest.json.minisig"
    verify_asset_hash "$TMPDIR_INSTALL/manifest.json" "$DEB_PATH" "$DEB_NAME"

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
