#!/bin/bash
# Build and inspect the exact macOS candidate package consumed by the Tart gate.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
VERSION=$(grep '^version' "$ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')
MANIFEST_URL="${CAPSEM_INSTALL_MANIFEST_URL:-https://release.capsem.org/assets/stable/manifest.json}"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            VERSION="${2:?--version requires a value}"
            shift 2
            ;;
        --manifest-url)
            MANIFEST_URL="${2:?--manifest-url requires a value}"
            shift 2
            ;;
        *)
            echo "usage: $0 [--version VERSION] [--manifest-url URL]" >&2
            exit 2
            ;;
    esac
done

[ "$(uname -s)" = "Darwin" ] || {
    echo "ERROR: macOS package proof requires macOS" >&2
    exit 1
}

cd "$ROOT"
bash scripts/check-web-surface.sh frontend-build
cargo tauri build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'
cargo build --release \
    -p capsem \
    -p capsem-service \
    -p capsem-process \
    -p capsem-tui \
    -p capsem-mcp \
    -p capsem-mcp-aggregator \
    -p capsem-mcp-builtin \
    -p capsem-gateway \
    -p capsem-tray \
    -p capsem-admin
if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
    for binary in \
        capsem \
        capsem-service \
        capsem-process \
        capsem-tui \
        capsem-mcp \
        capsem-mcp-aggregator \
        capsem-mcp-builtin \
        capsem-gateway \
        capsem-tray \
        capsem-admin
    do
        codesign \
            --sign "$APPLE_SIGNING_IDENTITY" \
            --options runtime \
            --timestamp \
            --entitlements "$ROOT/entitlements.plist" \
            --force \
            "$ROOT/target/release/$binary"
    done
fi
bash scripts/build-pkg.sh \
    --manifest "$MANIFEST_URL" \
    --signing-identity "${CAPSEM_INSTALLER_SIGNING_IDENTITY:?missing installer identity}" \
    "$ROOT/target/release/bundle/macos/Capsem.app" \
    "$ROOT/target/release" \
    "$ROOT/assets" \
    "$ROOT/target/config" \
    "$VERSION"

PKG="$ROOT/packages/Capsem-$VERSION.pkg"
test -s "$PKG"
/usr/sbin/pkgutil --check-signature "$PKG" | grep -F "Developer ID Installer"
codesign --verify --deep --strict --verbose=2 "$ROOT/target/release/bundle/macos/Capsem.app"
SBOM="$ROOT/target/macos-package-sbom.spdx.json"
python3 scripts/generate-host-binary-sbom.py --output "$SBOM" "$PKG"
python3 - "$SBOM" <<'PY'
import json
import pathlib
import sys

expected = {
    "capsem", "capsem-admin", "capsem-app", "capsem-gateway",
    "capsem-mcp", "capsem-mcp-aggregator", "capsem-mcp-builtin",
    "capsem-process", "capsem-service", "capsem-tray", "capsem-tui",
}
document = json.loads(pathlib.Path(sys.argv[1]).read_text())
actual = {pathlib.Path(row["fileName"]).name for row in document["files"]}
missing = sorted(expected - actual)
if missing:
    raise SystemExit(f"macOS package SBOM missing executables: {missing}")
PY

echo "Built macOS package for Tart install proof: $PKG"
