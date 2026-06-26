#!/usr/bin/env bash
set -euo pipefail

ROOT="${CAPSEM_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
ASSETS_DIR="${CAPSEM_ASSETS_DIR:-assets}"
OUTPUT_ROOT="${CAPSEM_CONFIG_OUTPUT_ROOT:-$ROOT/target/config}"
CONFIG_ROOT="${CAPSEM_CONFIG_ROOT:-$ROOT/config}"
MANIFEST="${CAPSEM_ASSET_MANIFEST:-$ROOT/$ASSETS_DIR/manifest.json}"
ASSETS_PATH="${CAPSEM_ASSETS_PATH:-$ROOT/$ASSETS_DIR}"

normalize_arch() {
    local arch="$1"
    case "$arch" in
        arm64|aarch64)
            echo "arm64"
            ;;
        x86_64|amd64)
            echo "x86_64"
            ;;
        *)
            echo "ERROR: unsupported materialize arch: $arch" >&2
            return 1
            ;;
    esac
}

manifest_arches="$(
    python3 - "$MANIFEST" <<'PY'
import json
import sys
from pathlib import Path

manifest = json.loads(Path(sys.argv[1]).read_text())
current = manifest["assets"]["current"]
arches = manifest["assets"]["releases"][current]["arches"]
for arch in sorted(arches):
    print(arch)
PY
)"

arch_source="host"
if [ -n "${CAPSEM_ARCH:-}" ]; then
    arch_source="CAPSEM_ARCH"
    arch="$(normalize_arch "$CAPSEM_ARCH")"
else
    arch="$(normalize_arch "$(uname -m)")"
fi

if ! printf '%s\n' "$manifest_arches" | grep -Fxq "$arch"; then
    manifest_arch_count="$(printf '%s\n' "$manifest_arches" | grep -c .)"
    if [ "$arch_source" = "host" ] && [ "$manifest_arch_count" = "1" ]; then
        fallback_arch="$(printf '%s\n' "$manifest_arches" | awk 'NF { print; exit }')"
        echo "  host arch $arch is not present in $MANIFEST; using sole manifest arch $fallback_arch"
        arch="$fallback_arch"
    else
        echo "ERROR: materialize arch $arch from $arch_source is not present in $MANIFEST" >&2
        echo "available manifest arches:" >&2
        printf '  %s\n' $manifest_arches >&2
        exit 1
    fi
fi

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
