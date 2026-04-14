#!/bin/bash
# build-pkg.sh -- Build a macOS .pkg installer from Tauri output + companion binaries.
#
# Usage: build-pkg.sh <app_path> <bin_dir> <assets_dir> <version> [signing_identity]
#
# Arguments:
#   app_path          Path to signed Capsem.app (from Tauri build)
#   bin_dir           Directory containing companion binaries (capsem, capsem-service, etc.)
#   assets_dir        Directory containing VM assets (manifest.json, vmlinuz, initrd.img, etc.)
#   version           Version string (e.g. "0.16.1")
#   signing_identity  Optional: Developer ID Installer identity for productsign
#
# Output: Capsem-<version>.pkg in the current directory
#
# The .pkg installs:
#   /Applications/Capsem.app           -- Tauri GUI
#   /usr/local/share/capsem/bin/       -- 6 companion binaries
#   /usr/local/share/capsem/assets/    -- manifest.json only (heavy assets downloaded on first use)
#   /usr/local/share/capsem/entitlements.plist
#
# A postinstall script copies binaries to ~/.capsem/bin/, codesigns them,
# registers the LaunchAgent, and runs capsem setup (which downloads VM assets).
set -euo pipefail

APP_PATH="${1:?usage: build-pkg.sh <app_path> <bin_dir> <assets_dir> <version> [signing_identity]}"
BIN_DIR="${2:?usage: build-pkg.sh <app_path> <bin_dir> <assets_dir> <version> [signing_identity]}"
ASSETS_DIR="${3:?usage: build-pkg.sh <app_path> <bin_dir> <assets_dir> <version> [signing_identity]}"
VERSION="${4:?usage: build-pkg.sh <app_path> <bin_dir> <assets_dir> <version> [signing_identity]}"
SIGNING_IDENTITY="${5:-}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

echo "=== Assembling .pkg payload ==="

# Application bundle
mkdir -p "$WORK_DIR/payload/Applications"
cp -R "$APP_PATH" "$WORK_DIR/payload/Applications/Capsem.app"

# Companion binaries
SHARE_DIR="$WORK_DIR/payload/usr/local/share/capsem"
mkdir -p "$SHARE_DIR/bin"
for bin in capsem capsem-service capsem-process capsem-mcp capsem-gateway capsem-tray; do
    src="$BIN_DIR/$bin"
    if [ -f "$src" ]; then
        cp "$src" "$SHARE_DIR/bin/$bin"
        chmod 755 "$SHARE_DIR/bin/$bin"
    else
        echo "ERROR: binary not found: $src" >&2
        exit 1
    fi
done

# Entitlements (needed by postinstall for codesigning)
if [ -f "$SCRIPT_DIR/../entitlements.plist" ]; then
    cp "$SCRIPT_DIR/../entitlements.plist" "$SHARE_DIR/"
fi

# VM assets: only bundle the manifest. The heavy assets (kernel, rootfs)
# are downloaded on first use by `capsem setup` / auto-setup.
mkdir -p "$SHARE_DIR/assets"
if [ -f "$ASSETS_DIR/manifest.json" ]; then
    cp "$ASSETS_DIR/manifest.json" "$SHARE_DIR/assets/"
fi

echo "=== Building component package ==="

# Build the component .pkg with postinstall script
pkgbuild \
    --root "$WORK_DIR/payload" \
    --scripts "$SCRIPT_DIR/pkg-scripts" \
    --identifier "com.capsem.pkg" \
    --version "$VERSION" \
    "$WORK_DIR/capsem.pkg"

echo "=== Building distribution package ==="

# Create welcome HTML
cat > "$WORK_DIR/welcome.html" <<'WELCOME_EOF'
<html>
<body>
<h1>Capsem</h1>
<p>Sandboxes AI agents in air-gapped Linux VMs on macOS.</p>
<p>This installer will:</p>
<ul>
  <li>Install Capsem.app to /Applications</li>
  <li>Install CLI tools to ~/.capsem/bin/</li>
  <li>Register the background service</li>
  <li>Download VM assets if needed</li>
</ul>
<p>After installation, open a new terminal and run <code>capsem shell</code> to start.</p>
</body>
</html>
WELCOME_EOF

# Stamp version into distribution XML (append build timestamp for uniqueness)
BUILD_TS=$(date +%s)
PKG_VERSION="$VERSION.$BUILD_TS"
sed "s/__VERSION__/$PKG_VERSION/g" "$SCRIPT_DIR/pkg-distribution.xml" > "$WORK_DIR/pkg-distribution.xml"

# Build the distribution .pkg (wraps component with UI)
productbuild \
    --distribution "$WORK_DIR/pkg-distribution.xml" \
    --resources "$WORK_DIR" \
    --package-path "$WORK_DIR" \
    "$WORK_DIR/Capsem-$VERSION-unsigned.pkg"

# Sign if identity provided
mkdir -p "$(dirname "$0")/../packages"
OUTPUT_PKG="$(dirname "$0")/../packages/Capsem-$VERSION.pkg"
if [ -n "$SIGNING_IDENTITY" ]; then
    echo "=== Signing .pkg ==="
    productsign \
        --sign "$SIGNING_IDENTITY" \
        "$WORK_DIR/Capsem-$VERSION-unsigned.pkg" \
        "$OUTPUT_PKG"
else
    cp "$WORK_DIR/Capsem-$VERSION-unsigned.pkg" "$OUTPUT_PKG"
    echo "WARNING: .pkg is unsigned (no signing identity provided)"
fi

echo "=== Built: $OUTPUT_PKG ==="
ls -lh "$OUTPUT_PKG"
