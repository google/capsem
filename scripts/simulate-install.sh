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

INSTALL_DIR="$HOME/.capsem/bin"
ASSETS_DST="$HOME/.capsem/assets"
RUN_DIR="$HOME/.capsem/run"

mkdir -p "$INSTALL_DIR" "$ASSETS_DST" "$RUN_DIR"

# Copy binaries
for bin in capsem capsem-service capsem-process capsem-mcp; do
    src="$BIN_SRC/$bin"
    if [[ ! -f "$src" ]]; then
        echo "ERROR: binary not found: $src" >&2
        exit 1
    fi
    cp "$src" "$INSTALL_DIR/$bin"
    chmod 755 "$INSTALL_DIR/$bin"
done

# Copy assets (manifest + arch dir -> versioned layout)
if [[ -f "$ASSETS_SRC/manifest.json" ]]; then
    cp "$ASSETS_SRC/manifest.json" "$ASSETS_DST/"
fi

ARCH=$(uname -m)
[[ "$ARCH" == "aarch64" ]] && ARCH="arm64"

if [[ -d "$ASSETS_SRC/$ARCH" ]]; then
    # Determine version from the capsem binary
    VERSION=$("$INSTALL_DIR/capsem" version 2>/dev/null | head -1 | sed -n 's/capsem \([^ ]*\).*/\1/p')
    if [[ -z "$VERSION" ]]; then
        VERSION="dev"
    fi
    mkdir -p "$ASSETS_DST/v$VERSION"
    cp -r "$ASSETS_SRC/$ARCH"/* "$ASSETS_DST/v$VERSION/"
fi

echo "Installed to $INSTALL_DIR ($(ls "$INSTALL_DIR" | wc -l | tr -d ' ') binaries)"
echo "Assets at $ASSETS_DST"

# Print build hash for verification
"$INSTALL_DIR/capsem" version 2>/dev/null | head -1 || true
