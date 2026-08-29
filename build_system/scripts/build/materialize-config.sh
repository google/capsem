#!/usr/bin/env bash
set -euo pipefail

pair_content=0
case "$#" in
    0)
        ;;
    1)
        if [ "$1" != "--pair-content" ]; then
            echo "ERROR: unknown materialize-config argument: $1" >&2
            exit 2
        fi
        pair_content=1
        ;;
    *)
        echo "ERROR: materialize-config accepts only --pair-content" >&2
        exit 2
        ;;
esac

ROOT="${CAPSEM_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)}"
ASSETS_DIR="${CAPSEM_ASSETS_DIR:-target/assets}"
OUTPUT_ROOT="${CAPSEM_CONFIG_OUTPUT_ROOT:-$ROOT/target/config}"
CONFIG_ROOT="${CAPSEM_CONFIG_ROOT:-$ROOT/config}"
MANIFEST="${CAPSEM_ASSET_MANIFEST:-$ROOT/$ASSETS_DIR/manifest.json}"
ASSETS_PATH="${CAPSEM_ASSETS_PATH:-$ROOT/$ASSETS_DIR}"

manifest_url() {
    python3 - "$MANIFEST" <<'PY'
from pathlib import Path
import sys
from urllib.parse import urlparse

source = sys.argv[1]
parsed = urlparse(source)
if parsed.scheme in {"file", "http", "https"}:
    print(source)
elif parsed.scheme:
    raise SystemExit(f"unsupported manifest URL scheme: {parsed.scheme}")
else:
    print(Path(source).resolve().as_uri())
PY
}

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

manifest_selection="$(
    python3 - "$MANIFEST" <<'PY'
import json
import sys
from pathlib import Path
from urllib.parse import urlparse
from urllib.request import Request, url2pathname, urlopen

USER_AGENT = "capsem-materialize-config/1"

source = sys.argv[1]
parsed = urlparse(source)
if parsed.scheme in {"http", "https"}:
    request = Request(source, headers={"User-Agent": USER_AGENT})
    content = urlopen(request, timeout=60).read().decode("utf-8")
elif parsed.scheme == "file":
    content = Path(url2pathname(parsed.path)).read_text()
elif parsed.scheme:
    raise SystemExit(f"unsupported manifest URL scheme: {parsed.scheme}")
else:
    content = Path(source).read_text()
manifest = json.loads(content)
if "assets" in manifest:
    print("SCHEMA\tlegacy")
    current = manifest["assets"]["current"]
    arches = set(manifest["assets"]["releases"][current]["arches"])
elif "profiles" in manifest:
    profiles = manifest["profiles"]
    if not isinstance(profiles, dict) or not profiles:
        raise SystemExit("release manifest profiles must be a non-empty object")
    print("SCHEMA\trelease")
    active_profiles = []
    for profile_id, profile in sorted(profiles.items()):
        if (
            not isinstance(profile_id, str)
            or not profile_id
            or profile_id in {".", ".."}
            or "/" in profile_id
            or "\\" in profile_id
            or "\n" in profile_id
            or "\r" in profile_id
        ):
            raise SystemExit(f"release manifest contains unsafe profile identity: {profile_id!r}")
        if not isinstance(profile, dict):
            raise SystemExit(f"release manifest profile {profile_id} must be an object")
        if str(profile.get("status", "")).lower() == "revoked":
            continue
        active_profiles.append((profile_id, profile))
    if not active_profiles:
        raise SystemExit("release manifest profiles contain no active profiles")
    arches = {
        entry["architecture"]
        for _, profile in active_profiles
        for entry in profile.get("architectures", [])
        if isinstance(entry, dict) and isinstance(entry.get("architecture"), str)
    }
    if not arches:
        raise SystemExit("release manifest profiles contain no architectures")
    for profile_id, _ in active_profiles:
        print(f"PROFILE\t{profile_id}")
else:
    raise SystemExit("manifest must contain legacy assets or release profiles")
for arch in sorted(arches):
    print(f"ARCH\t{arch}")
PY
)"

manifest_schema="release"
if printf '%s\n' "$manifest_selection" | grep -Fqx $'SCHEMA\tlegacy'; then
    manifest_schema="legacy"
fi
manifest_arches="$(
    printf '%s\n' "$manifest_selection" |
        awk -F '\t' '$1 == "ARCH" { print substr($0, index($0, "\t") + 1) }'
)"
profile_ids="$(
    printf '%s\n' "$manifest_selection" |
        awk -F '\t' '$1 == "PROFILE" { print substr($0, index($0, "\t") + 1) }'
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

profile_paths=()
if [ "$manifest_schema" = "release" ]; then
    while IFS= read -r profile_id; do
        [ -n "$profile_id" ] || continue
        profile_path="$CONFIG_ROOT/profiles/$profile_id/profile.toml"
        if [ ! -f "$profile_path" ]; then
            echo "ERROR: selected release profile source is missing: $profile_path" >&2
            exit 1
        fi
        profile_paths+=("$profile_path")
    done <<< "$profile_ids"
else
    profile_paths=("$CONFIG_ROOT"/profiles/*/profile.toml)
    if [ "${#profile_paths[@]}" -eq 0 ] || [ ! -f "${profile_paths[0]}" ]; then
        echo "ERROR: no profile inputs found under $CONFIG_ROOT/profiles" >&2
        exit 1
    fi
fi

if [ "${#profile_paths[@]}" -eq 0 ]; then
    echo "ERROR: selected release manifest contains no materializable profiles" >&2
    exit 1
fi

rm -rf "$OUTPUT_ROOT"

for profile_path in "${profile_paths[@]}"; do
    profile_id="$(basename "$(dirname "$profile_path")")"
    echo "  materializing profile: $profile_id"
    cargo run -p capsem-admin -- profile materialize \
        --profile "$profile_path" \
        --config-root "$CONFIG_ROOT" \
        --manifest "$(manifest_url)" \
        --assets-dir "$ASSETS_PATH" \
        --output-root "$OUTPUT_ROOT" \
        --arch "$arch"
done

case "$pair_content" in
    0)
        ;;
    1)
        python3 - "$ASSETS_PATH" "$OUTPUT_ROOT" "$MANIFEST" <<'PY'
import json
import os
from pathlib import Path
import sys
import tempfile

assets = Path(sys.argv[1])
config = Path(sys.argv[2])
selected_manifest = Path(sys.argv[3])
for label, directory in (("assets", assets), ("config", config)):
    if directory.is_symlink() or not directory.is_dir():
        raise SystemExit(f"paired content {label} must be a real directory: {directory}")

runtime_manifest = config / "assets" / "manifest.json"
asset_manifest = assets / "manifest.json"
if selected_manifest.resolve() != asset_manifest.resolve():
    raise SystemExit(
        "paired content manifest must be the selected asset manifest: "
        f"{selected_manifest} != {asset_manifest}"
    )
try:
    payload = runtime_manifest.read_bytes()
    document = json.loads(payload)
    current = document["assets"]["current"]
    document["assets"]["releases"][current]["arches"]
except (OSError, json.JSONDecodeError, KeyError, TypeError) as error:
    raise SystemExit(f"materialized runtime manifest is invalid: {runtime_manifest}: {error}")

with tempfile.NamedTemporaryFile(dir=assets, prefix=".manifest.", delete=False) as handle:
    temporary = Path(handle.name)
    handle.write(payload)
try:
    os.chmod(temporary, 0o644)
    os.replace(temporary, asset_manifest)
finally:
    temporary.unlink(missing_ok=True)
if asset_manifest.read_bytes() != runtime_manifest.read_bytes():
    raise SystemExit("paired content manifests differ after atomic finalization")
PY
        ;;
esac
