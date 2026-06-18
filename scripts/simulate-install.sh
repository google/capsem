#!/bin/bash
# simulate-install.sh -- Reproduce the installed layout for testing.
# Usage: simulate-install.sh <bin_dir_src> <assets_dir_src> <config_root>
# Installs to ~/.capsem/{bin,assets,profiles,run}
#
# This is the single source of truth for how binaries land in ~/.capsem/.
# Both `just install` and the Docker e2e test harness call this script.
# When WB7 (install.sh) lands, the real script replaces this one and test
# fixtures swap the fixture -- same tests, real script.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_SRC="${1:?usage: simulate-install.sh <bin_dir> <assets_dir> <config_root>}"
ASSETS_SRC="${2:?usage: simulate-install.sh <bin_dir> <assets_dir> <config_root>}"
CONFIG_ROOT="${3:?usage: simulate-install.sh <bin_dir> <assets_dir> <config_root>}"

# Honor CAPSEM_HOME so the install-test suite can redirect this script into
# an isolated temp dir (see tests/capsem-install/conftest.py::_resolve_capsem_home).
CAPSEM_HOME_DIR="${CAPSEM_HOME:-$HOME/.capsem}"
INSTALL_DIR="$CAPSEM_HOME_DIR/bin"
ASSETS_DST="$CAPSEM_HOME_DIR/assets"
PROFILES_DST="$CAPSEM_HOME_DIR/profiles"
RUN_DIR="${CAPSEM_RUN_DIR:-$CAPSEM_HOME_DIR/run}"

# Preflight: reap any running capsem processes FROM THIS INSTALL PREFIX so
# reinstalling mid-session doesn't leave the old service (and its per-VM
# capsem-process children) holding Apple VZ memory.
#
# Scoped to ``$INSTALL_DIR/`` so parallel pytest workers running
# ``target/debug/capsem-*`` are not caught in the blast. A bare
# ``pkill -x capsem-service`` matches every capsem-service on the box, which
# poisoned the full test suite whenever any install fixture fired this script.
for name in capsem-service capsem-tray capsem-gateway capsem-process capsem-mcp-aggregator capsem-mcp-builtin; do
    pkill -9 -f "$INSTALL_DIR/$name" 2>/dev/null || true
done

rm -rf "$CAPSEM_HOME_DIR"/bin.backup*
retired_user_config="user"".toml"
rm -f "$CAPSEM_HOME_DIR/$retired_user_config" "$CAPSEM_HOME_DIR/service.toml"
echo "event=retired_config_removed"
mkdir -p "$INSTALL_DIR" "$RUN_DIR"
rm -rf "$INSTALL_DIR/capsem-admin-python"
echo "event=retired_python_admin_bundle_removed"
# Remove dev symlink if present (just _ensure-service creates one)
if [[ -L "$ASSETS_DST" ]]; then
    rm "$ASSETS_DST"
fi
mkdir -p "$ASSETS_DST"
if [[ ! -d "$CONFIG_ROOT/profiles" ]]; then
    echo "ERROR: materialized profiles not found: $CONFIG_ROOT/profiles" >&2
    echo "Run: just _materialize-config" >&2
    exit 1
fi

# Copy binaries
for bin in capsem capsem-service capsem-process capsem-tui capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray capsem-admin; do
    src="$BIN_SRC/$bin"
    if [[ ! -f "$src" ]]; then
        echo "ERROR: binary not found: $src" >&2
        exit 1
    fi
    cp "$src" "$INSTALL_DIR/$bin"
    chmod 755 "$INSTALL_DIR/$bin"
done

# Codesign real macOS Mach-O binaries with Virtualization entitlements. Fake
# shell-script binaries used by install tests are intentionally skipped.
if [[ "$(uname -s)" == "Darwin" ]]; then
    ENTITLEMENTS="$(cd "$SCRIPT_DIR/.." && pwd)/entitlements.plist"
    for bin in "$INSTALL_DIR"/capsem*; do
        [[ -f "$bin" ]] || continue
        if file "$bin" | grep -q "Mach-O"; then
            if [[ ! -r "$ENTITLEMENTS" ]]; then
                echo "ERROR: entitlements.plist not found at $ENTITLEMENTS" >&2
                exit 1
            fi
            codesign --sign - --entitlements "$ENTITLEMENTS" --force "$bin"
        fi
    done
fi

# Copy assets through the same manifest-driven path used by local packages.
if [[ -f "$ASSETS_SRC/manifest.json" ]]; then
    bash "$SCRIPT_DIR/sync-dev-assets.sh" "$ASSETS_SRC" "$ASSETS_DST"
fi

rm -rf "$PROFILES_DST"
mkdir -p "$PROFILES_DST"
cp -R "$CONFIG_ROOT/profiles/." "$PROFILES_DST/"

# Drop legacy v1 layout directories that ManifestV2::resolve() no longer reads.
for legacy in "$ASSETS_DST"/v1.0.*; do
    [[ -d "$legacy" ]] || continue
    rm -rf "$legacy"
done

echo "Installed to $INSTALL_DIR ($(ls "$INSTALL_DIR" | wc -l | tr -d ' ') binaries)"
echo "Assets at $ASSETS_DST"
echo "Profiles at $PROFILES_DST"

# Print build hash for verification (use source binary -- installed copy may not be signed yet)
"$BIN_SRC/capsem" version 2>/dev/null | head -1 || true
