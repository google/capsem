#!/bin/bash
# simulate-install.sh -- Reproduce the installed layout for testing.
# Usage: simulate-install.sh <bin_dir_src> <assets_dir_src>
# Installs to ~/.capsem/{bin,assets,run}
#
# This is the single source of truth for how binaries land in ~/.capsem/.
# Both `just install` and the Docker e2e test harness call this script.
# When WB7 (install.sh) lands, the real script replaces this one and test
# fixtures swap the fixture -- same tests, real script.
set -euo pipefail

BIN_SRC="${1:?usage: simulate-install.sh <bin_dir> <assets_dir>}"
ASSETS_SRC="${2:?usage: simulate-install.sh <bin_dir> <assets_dir>}"

# Honor CAPSEM_HOME so the install-test suite can redirect this script into
# an isolated temp dir (see tests/capsem-install/conftest.py::_resolve_capsem_home).
CAPSEM_HOME_DIR="${CAPSEM_HOME:-$HOME/.capsem}"
INSTALL_DIR="$CAPSEM_HOME_DIR/bin"
ASSETS_DST="$CAPSEM_HOME_DIR/assets"
RUN_DIR="${CAPSEM_RUN_DIR:-$CAPSEM_HOME_DIR/run}"

# Preflight: reap any running capsem processes FROM THIS INSTALL PREFIX so
# reinstalling mid-session doesn't leave the old service (and its per-VM
# capsem-process children) holding Apple VZ memory.
#
# Scoped to ``$INSTALL_DIR/`` so parallel pytest workers running
# ``target/debug/capsem-*`` are not caught in the blast. A bare
# ``pkill -x capsem-service`` matches every capsem-service on the box, which
# poisoned the full test suite whenever any install fixture fired this script.
for name in capsem-service capsem-tray capsem-gateway capsem-process; do
    pkill -9 -f "$INSTALL_DIR/$name" 2>/dev/null || true
done

mkdir -p "$INSTALL_DIR" "$RUN_DIR"
# Remove dev symlink if present (just _ensure-service creates one)
if [[ -L "$ASSETS_DST" ]]; then
    rm "$ASSETS_DST"
fi
mkdir -p "$ASSETS_DST"

# Copy binaries
for bin in capsem capsem-service capsem-process capsem-mcp capsem-gateway capsem-tray; do
    src="$BIN_SRC/$bin"
    if [[ ! -f "$src" ]]; then
        echo "ERROR: binary not found: $src" >&2
        exit 1
    fi
    cp "$src" "$INSTALL_DIR/$bin"
    chmod 755 "$INSTALL_DIR/$bin"
done

# Copy assets: manifest + the per-arch hash-named files. Matches the layout
# ManifestV2::resolve() actually reads: $ASSETS_DST/$ARCH/{hash_filename}.
if [[ -f "$ASSETS_SRC/manifest.json" ]]; then
    cp "$ASSETS_SRC/manifest.json" "$ASSETS_DST/"
fi

ARCH=$(uname -m)
[[ "$ARCH" == "aarch64" ]] && ARCH="arm64"

if [[ -d "$ASSETS_SRC/$ARCH" ]]; then
    mkdir -p "$ASSETS_DST/$ARCH"
    cp -f "$ASSETS_SRC/$ARCH"/* "$ASSETS_DST/$ARCH/"
fi

# Drop legacy v1 layout directories that ManifestV2::resolve() no longer reads.
for legacy in "$ASSETS_DST"/v1.0.*; do
    [[ -d "$legacy" ]] || continue
    rm -rf "$legacy"
done

echo "Installed to $INSTALL_DIR ($(ls "$INSTALL_DIR" | wc -l | tr -d ' ') binaries)"
echo "Assets at $ASSETS_DST"

# Print build hash for verification (use source binary -- installed copy may not be signed yet)
"$BIN_SRC/capsem" version 2>/dev/null | head -1 || true
