#!/bin/bash
# build-pkg.sh -- Build a macOS .pkg installer from Tauri output + companion binaries.
#
# Usage: build-pkg.sh [--manifest file://...|http://...|https://...] <app_path> <bin_dir> <assets_dir> <config_root> <version> [signing_identity]
#
# Arguments:
#   app_path          Path to signed Capsem.app (from Tauri build)
#   bin_dir           Directory containing companion binaries (capsem, capsem-service, etc.)
#   assets_dir        Directory containing manifest.json when --manifest is omitted.
#   config_root       Materialized runtime config root (usually target/config)
#   version           Version string (e.g. "0.16.1")
#   signing_identity  Optional: Developer ID Installer identity for productsign
#   --manifest        Optional manifest URL to record for postinstall hydration.
#
# Output: Capsem-<version>.pkg in the current directory
#
# The .pkg installs:
#   /Applications/Capsem.app           -- Tauri GUI
#   /usr/local/share/capsem/bin/       -- 6 companion binaries
#   /usr/local/share/capsem/assets/    -- selected manifest URL provenance
#   /usr/local/share/capsem/profiles/  -- materialized profile catalog + rule files
#   /usr/local/share/capsem/entitlements.plist
#
# A postinstall script copies binaries to ~/.capsem/bin/, codesigns them,
# registers the LaunchAgent, and waits for service readiness.
set -euo pipefail
export COPYFILE_DISABLE=1

usage() {
    echo "usage: build-pkg.sh [--manifest file://...|http://...|https://...] <app_path> <bin_dir> <assets_dir> <config_root> <version> [signing_identity]" >&2
}

MANIFEST_PATH=""
SIGNING_IDENTITY=""
POSITIONAL=()
while [ "$#" -gt 0 ]; do
    case "$1" in
        --manifest)
            MANIFEST_PATH="${2:?--manifest requires a URL}"
            shift 2
            ;;
        --signing-identity)
            SIGNING_IDENTITY="${2:?--signing-identity requires a value}"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            while [ "$#" -gt 0 ]; do
                POSITIONAL+=("$1")
                shift
            done
            ;;
        --*)
            echo "ERROR: unknown option $1" >&2
            usage
            exit 2
            ;;
        *)
            POSITIONAL+=("$1")
            shift
            ;;
    esac
done

if [ "${#POSITIONAL[@]}" -lt 5 ] || [ "${#POSITIONAL[@]}" -gt 6 ]; then
    usage
    exit 2
fi

APP_PATH="${POSITIONAL[0]}"
BIN_DIR="${POSITIONAL[1]}"
ASSETS_DIR="${POSITIONAL[2]}"
CONFIG_ROOT="${POSITIONAL[3]}"
VERSION="${POSITIONAL[4]}"
if [ -z "$SIGNING_IDENTITY" ] && [ "${#POSITIONAL[@]}" -eq 6 ]; then
    SIGNING_IDENTITY="${POSITIONAL[5]}"
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

copy_tree_clean() {
    local src="${1:?copy_tree_clean <src> <dst>}"
    local dst="${2:?copy_tree_clean <src> <dst>}"
    mkdir -p "$dst"
    if command -v ditto >/dev/null 2>&1; then
        ditto --norsrc --noextattr "$src" "$dst"
    else
        COPYFILE_DISABLE=1 cp -R "$src/." "$dst/"
    fi
}

write_manifest_metadata() {
    local manifest_source="${1:?write_manifest_metadata <manifest_source> <package_version> <dst>}"
    local package_version="${2:?write_manifest_metadata <manifest_source> <package_version> <dst>}"
    local dst="${3:?write_manifest_metadata <manifest_source> <package_version> <dst>}"
    python3 - "$manifest_source" "$package_version" "$dst" <<'PY'
import datetime
import json
import pathlib
import sys
import urllib.parse

raw_source = sys.argv[1]
package_version = sys.argv[2]
dst = pathlib.Path(sys.argv[3])
parsed = urllib.parse.urlparse(raw_source)
if parsed.scheme not in ("http", "https", "file"):
    raise SystemExit(f"manifest must be a URL: {raw_source}")
parts = [part for part in parsed.path.split("/") if part]
public_channel = (
    parts[-2]
    if len(parts) >= 3
    and parts[-3] == "assets"
    and parts[-2] in ("stable", "nightly")
    and parts[-1] == "manifest.json"
    else None
)
fetched_at = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
dst.write_text(json.dumps({
    "schema": "capsem.manifest_metadata.v1",
    "origin": "package",
    "manifest_url": raw_source,
    "channel": public_channel or "corp",
    "channel_kind": "public" if public_channel else "corporate",
    "channel_locked": public_channel is None,
    "fetched_at": fetched_at,
    "packaged_at": fetched_at,
    "package_version": package_version,
}, sort_keys=True) + "\n")
PY
}

validate_app_bundle_version() {
    local app_path="${1:?validate_app_bundle_version <app_path> <version>}"
    local expected_version="${2:?validate_app_bundle_version <app_path> <version>}"
    python3 - "$app_path" "$expected_version" <<'PY'
import pathlib
import plistlib
import sys

app = pathlib.Path(sys.argv[1])
expected = sys.argv[2]
plist_path = app / "Contents" / "Info.plist"
if not plist_path.is_file():
    raise SystemExit(f"Capsem.app Info.plist missing: {plist_path}")
with plist_path.open("rb") as fh:
    info = plistlib.load(fh)
actual = info.get("CFBundleShortVersionString")
if actual != expected:
    raise SystemExit(
        "Capsem.app CFBundleShortVersionString mismatch: "
        f"expected {expected}, got {actual or '<missing>'}"
    )
PY
}

file_url() {
    local path="${1:?file_url <path>}"
    python3 - "$path" <<'PY'
import pathlib
import sys

print(pathlib.Path(sys.argv[1]).resolve().as_uri())
PY
}

reject_retired_credential_store_markers() {
    local path="${1:?reject_retired_credential_store_markers <path>}"
    local marker
    for marker in \
        "CAPSEM_CREDENTIAL_BROKER_TEST_STORE" \
        "org.capsem.credentials" \
        "com.capsem.credential" \
        "open default keychain" \
        "security-framework/src/os/macos/keychain.rs"
    do
        if LC_ALL=C grep -aFq "$marker" "$path"; then
            echo "ERROR: binary contains retired native Keychain credential-store marker: $path ($marker)" >&2
            exit 1
        fi
    done
}

echo "=== Assembling .pkg payload ==="

# Application bundle
validate_app_bundle_version "$APP_PATH" "$VERSION"
mkdir -p "$WORK_DIR/payload/Applications"
cp -R "$APP_PATH" "$WORK_DIR/payload/Applications/Capsem.app"

# Companion binaries
SHARE_DIR="$WORK_DIR/payload/usr/local/share/capsem"
mkdir -p "$SHARE_DIR/bin"
for bin in capsem capsem-service capsem-process capsem-tui capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray capsem-admin; do
    src="$BIN_DIR/$bin"
    if [ -f "$src" ]; then
        reject_retired_credential_store_markers "$src"
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

# VM manifest source. The package carries only the selected manifest URL and
# provenance. Postinstall hydrates assets from that live channel URL.
mkdir -p "$SHARE_DIR/assets"
if [ -n "$MANIFEST_PATH" ]; then
    SELECTED_MANIFEST_SOURCE="$MANIFEST_PATH"
else
    if [ -z "$ASSETS_DIR" ] || [ ! -f "$ASSETS_DIR/manifest.json" ]; then
        echo "ERROR: manifest not found: $ASSETS_DIR/manifest.json" >&2
        exit 1
    fi
    SELECTED_MANIFEST_SOURCE="$(file_url "$ASSETS_DIR/manifest.json")"
fi
write_manifest_metadata "$SELECTED_MANIFEST_SOURCE" "$VERSION" "$SHARE_DIR/assets/manifest-metadata.json"

# Materialized profile catalog. Profiles pin the asset hashes the daemon boots;
# the package installs the profile ledger and the manifest ledger together, but
# never embeds the VM asset blobs themselves.
if [ ! -d "$CONFIG_ROOT/profiles" ]; then
    echo "ERROR: materialized profiles not found: $CONFIG_ROOT/profiles" >&2
    echo "Run: just _materialize-config" >&2
    exit 1
fi
mkdir -p "$SHARE_DIR/profiles"
copy_tree_clean "$CONFIG_ROOT/profiles" "$SHARE_DIR/profiles"

echo "=== Building component package ==="

PKG_SCRIPTS="$WORK_DIR/pkg-scripts"
mkdir -p "$PKG_SCRIPTS"
install -m 0755 "$SCRIPT_DIR/pkg-scripts/preinstall" "$PKG_SCRIPTS/preinstall"
install -m 0755 "$SCRIPT_DIR/pkg-scripts/postinstall" "$PKG_SCRIPTS/postinstall"
install -m 0755 "$SCRIPT_DIR/pkg-scripts/install-diagnostics" "$PKG_SCRIPTS/install-diagnostics"

# Strip macOS extended attributes in the temporary staging area. Otherwise
# pkgbuild serializes AppleDouble `._*` sidecars into Payload/Scripts.
if command -v xattr >/dev/null 2>&1; then
    xattr -rc "$WORK_DIR/payload" "$PKG_SCRIPTS" 2>/dev/null || true
fi
find "$WORK_DIR/payload" "$PKG_SCRIPTS" -name '._*' -delete

# Build the component .pkg with package-owned preinstall/postinstall scripts.
pkgbuild \
    --root "$WORK_DIR/payload" \
    --scripts "$PKG_SCRIPTS" \
    --identifier "com.capsem.pkg" \
    --version "$VERSION" \
    --filter '/\._[^/]*$' \
    --filter '\.DS_Store$' \
    "$WORK_DIR/capsem.pkg"

echo "=== Building distribution package ==="

# Create welcome HTML
cat > "$WORK_DIR/welcome.html" <<'WELCOME_EOF'
<html>
<body>
<h1>Capsem</h1>
<p>The fastest way to ship with AI securely.</p>
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

# Stamp version into distribution XML.
sed "s/__VERSION__/$VERSION/g" "$SCRIPT_DIR/pkg-distribution.xml" > "$WORK_DIR/pkg-distribution.xml"

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
