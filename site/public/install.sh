#!/bin/sh
# Capsem installer -- downloads the stable binary package and installs it.
#   macOS: downloads .pkg, opens the native installer GUI
#   Linux: downloads .deb, installs via apt
# Usage: curl -fsSL https://capsem.org/install.sh | sh
set -eu

CAPSEM_CHANNEL="${CAPSEM_CHANNEL:-stable}"
CAPSEM_RELEASE_BASE_URL="${CAPSEM_RELEASE_BASE_URL:-https://release.capsem.org}"
CAPSEM_MANIFEST_URL="${CAPSEM_MANIFEST_URL:-${CAPSEM_RELEASE_BASE_URL}/assets/${CAPSEM_CHANNEL}/manifest.json}"

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
    _manifest_json="$1"
    _os="$2"
    _arch="$3"
    case "$_os" in
        darwin)
            _platform="macos"
            _kind="macos_pkg"
            _manifest_arch="arm64"
            _name_suffix=".pkg"
            ;;
        linux)
            _platform="linux"
            _kind="debian_package"
            case "$_arch" in
                amd64)
                    _manifest_arch="x86_64"
                    _name_suffix="_amd64.deb"
                    ;;
                arm64)
                    _manifest_arch="arm64"
                    _name_suffix="_arm64.deb"
                    ;;
                *)
                    echo "Error: no matching asset found for $_os/$_arch in the ${CAPSEM_CHANNEL} release channel." >&2
                    return 1
                    ;;
            esac
            ;;
    esac

    _asset_record="$(printf '%s\n' "$_manifest_json" | awk \
        -v platform="$_platform" \
        -v kind="$_kind" \
        -v arch="$_manifest_arch" '
        function count_char(text, char, i, n) {
            n = 0
            for (i = 1; i <= length(text); i++) {
                if (substr(text, i, 1) == char) {
                    n++
                }
            }
            return n
        }
        BEGIN {
            platform_re = "\"" "platform" "\"" "[[:space:]]*:[[:space:]]*\"" platform "\""
            kind_re = "\"" "kind" "\"" "[[:space:]]*:[[:space:]]*\"" kind "\""
            arch_re = "\"" "architecture" "\"" "[[:space:]]*:[[:space:]]*\"" arch "\""
            status_re = "\"" "status" "\"" "[[:space:]]*:[[:space:]]*\"current\""
        }
        /"packages"[[:space:]]*:/ {
            in_packages = 1
            next
        }
        in_packages {
            if (!in_package && $0 ~ /^[[:space:]]*\{/) {
                in_package = 1
                depth = 0
                block = ""
            }
            if (in_package) {
                block = block $0 "\n"
                depth += count_char($0, "{") - count_char($0, "}")
                if (depth == 0) {
                    if (block ~ platform_re && block ~ kind_re && block ~ arch_re && block ~ status_re) {
                        printf "%s", block
                        exit
                    }
                    in_package = 0
                    block = ""
                }
            }
        }
    ')"

    ASSET_URL="$(printf '%s\n' "$_asset_record" | awk -F'"' '/"url"[[:space:]]*:/ { value = $4 } END { print value }')"
    ASSET_VERSION="$(printf '%s\n' "$_asset_record" | awk -F'"' '/"version"[[:space:]]*:/ { value = $4 } END { print value }')"
    ASSET_NAME="$(printf '%s\n' "$_asset_record" | awk -F'"' '/"name"[[:space:]]*:/ { value = $4 } END { print value }')"

    if [ -z "$ASSET_URL" ] || [ -z "$ASSET_VERSION" ] || [ -z "$ASSET_NAME" ]; then
        echo "Error: no matching asset found for $_os/$_arch in the ${CAPSEM_CHANNEL} release channel." >&2
        return 1
    fi
    case "$ASSET_NAME" in
        *"$_name_suffix") ;;
        *)
            echo "Error: release channel selected unexpected package $ASSET_NAME for $_os/$_arch." >&2
            return 1
            ;;
    esac
    case "$ASSET_URL" in
        http://*|https://*) ;;
        *)
            echo "Error: release channel package $ASSET_NAME has an invalid URL: $ASSET_URL" >&2
            return 1
            ;;
    esac
}

fetch_release_manifest() {
    if [ -z "$CAPSEM_MANIFEST_URL" ]; then
        echo "Error: CAPSEM_MANIFEST_URL is empty." >&2
        return 1
    fi
    curl -fsSL "$CAPSEM_MANIFEST_URL"
}

install_macos() {
    _pkg_url="$1"
    _version="$2"

    TMPDIR_INSTALL="$(mktemp -d)"
    PKG_PATH="${TMPDIR_INSTALL}/Capsem.pkg"

    cleanup_macos() {
        rm -rf "$TMPDIR_INSTALL"
    }
    trap cleanup_macos EXIT

    echo "Downloading $_pkg_url..."
    curl -fSL --progress-bar -o "$PKG_PATH" "$_pkg_url"

    echo "Opening installer..."
    open "$PKG_PATH"

    echo ""
    echo "Capsem $_version installer launched."
    echo "Follow the installer GUI to complete installation."
    echo "After install, open a new terminal and run: capsem shell"
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

# Fetch stable release-channel manifest. The asset lane may be newer than the
# binary lane, so do not use GitHub's "latest release" endpoint here.
echo "Fetching Capsem ${CAPSEM_CHANNEL} release channel..."
RELEASE_MANIFEST="$(fetch_release_manifest)"

# Find the right asset
find_asset_url "$RELEASE_MANIFEST" "$OS" "$ARCH"
VERSION="$ASSET_VERSION"
echo "Installing Capsem $VERSION from ${CAPSEM_CHANNEL}..."

# Install
case "$OS" in
    darwin) install_macos "$ASSET_URL" "$VERSION" ;;
    linux)  install_linux "$ASSET_URL" "$VERSION" ;;
esac
