#!/bin/bash
# Build the unsigned local macOS candidate package consumed by the Tart gate.
#
# The installer postinstall ad-hoc signs the installed Mach-O payload with the
# required entitlements. Developer ID signing, notarization, and stapling belong
# only to the tagged publication workflow.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$SCRIPT_DIR/../../.." && pwd)
VERSION=$(grep '^version' "$ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')
MANIFEST_URL="${CAPSEM_INSTALL_MANIFEST_URL:-https://release.capsem.org/assets/stable/manifest.json}"
ASSETS_DIR=""
CONFIG_ROOT=""

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
        --assets-dir)
            ASSETS_DIR="${2:?--assets-dir requires a value}"
            shift 2
            ;;
        --config-root)
            CONFIG_ROOT="${2:?--config-root requires a value}"
            shift 2
            ;;
        *)
            echo "usage: $0 [--version VERSION] [--manifest-url URL] --assets-dir DIR --config-root DIR" >&2
            exit 2
            ;;
    esac
done

[ -n "$ASSETS_DIR" ] && [ -n "$CONFIG_ROOT" ] || {
    echo "ERROR: --assets-dir and --config-root are required as one content pair" >&2
    exit 2
}
[ -d "$ASSETS_DIR" ] && [ -d "$CONFIG_ROOT" ] || {
    echo "ERROR: selected macOS package content is incomplete" >&2
    exit 1
}

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
    -p capsem-admin \
    -p capsem-mock-server \
    -p capsem-bench
bash scripts/check-build-provenance.sh "$ROOT/target/release/capsem"
bash "$SCRIPT_DIR/build-pkg.sh" \
    --manifest "$MANIFEST_URL" \
    "$ROOT/target/release/bundle/macos/Capsem.app" \
    "$ROOT/target/release" \
    "$ASSETS_DIR" \
    "$CONFIG_ROOT" \
    "$VERSION"

PKG="$ROOT/packages/Capsem-$VERSION.pkg"
test -s "$PKG"
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
    "capsem-mock-server", "capsem-bench-rs",
}
document = json.loads(pathlib.Path(sys.argv[1]).read_text())
actual = {pathlib.Path(row["fileName"]).name for row in document["files"]}
missing = sorted(expected - actual)
if missing:
    raise SystemExit(f"macOS package SBOM missing executables: {missing}")
PY

echo "Built macOS package for Tart install proof: $PKG"
