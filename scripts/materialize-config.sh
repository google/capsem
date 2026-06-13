#!/usr/bin/env bash
set -euo pipefail

ROOT="${CAPSEM_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
ASSETS_DIR="${CAPSEM_ASSETS_DIR:-assets}"
OUTPUT_ROOT="${CAPSEM_CONFIG_OUTPUT_ROOT:-$ROOT/target/config}"
CONFIG_ROOT="${CAPSEM_CONFIG_ROOT:-$ROOT/config}"
MANIFEST="${CAPSEM_ASSET_MANIFEST:-$ROOT/$ASSETS_DIR/manifest.json}"
ASSETS_PATH="${CAPSEM_ASSETS_PATH:-$ROOT/$ASSETS_DIR}"

arch="${CAPSEM_ARCH:-$(uname -m)}"
case "$arch" in
    arm64|aarch64)
        arch="arm64"
        ;;
    x86_64|amd64)
        arch="x86_64"
        ;;
    *)
        echo "ERROR: unsupported materialize arch: $arch" >&2
        exit 1
        ;;
esac

echo "=== Materialize runtime config ==="
rm -rf "$ROOT/target/config"
if [ "$OUTPUT_ROOT" != "$ROOT/target/config" ]; then
    rm -rf "$OUTPUT_ROOT"
fi

profile_paths=("$ROOT"/config/profiles/*/profile.toml)
if [ "${#profile_paths[@]}" -eq 0 ] || [ ! -f "${profile_paths[0]}" ]; then
    echo "ERROR: no checked-in profiles found under $ROOT/config/profiles" >&2
    exit 1
fi

for profile_path in "${profile_paths[@]}"; do
    profile_id="$(basename "$(dirname "$profile_path")")"
    echo "  materializing profile: $profile_id"
    cargo run -p capsem-admin -- profile materialize \
        --profile "$profile_path" \
        --config-root "$CONFIG_ROOT" \
        --manifest "$MANIFEST" \
        --assets-dir "$ASSETS_PATH" \
        --output-root "$OUTPUT_ROOT" \
        --arch "$arch"
done
