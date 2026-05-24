#!/bin/bash
# simulate-install.sh -- Reproduce the installed layout for testing.
# Usage: simulate-install.sh <bin_dir_src> <assets_dir_src>
# Installs to ~/.capsem/{bin,assets,profiles,run}
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
PROFILES_DST="$CAPSEM_HOME_DIR/profiles/base"
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

mkdir -p "$INSTALL_DIR" "$PROFILES_DST" "$RUN_DIR"
# Remove dev symlink if present (just _ensure-service creates one)
if [[ -L "$ASSETS_DST" ]]; then
    rm "$ASSETS_DST"
fi
mkdir -p "$ASSETS_DST"

copy_if_different() {
    local src="$1"
    local dst="$2"
    if [[ -e "$dst" && "$src" -ef "$dst" ]]; then
        return 0
    fi
    cp -f "$src" "$dst"
}

# Copy binaries
for bin in capsem capsem-service capsem-process capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-gateway capsem-tray; do
    src="$BIN_SRC/$bin"
    dst="$INSTALL_DIR/$bin"
    if [[ ! -f "$src" ]]; then
        echo "ERROR: binary not found: $src" >&2
        exit 1
    fi
    # Replace existing paths atomically-ish: postinst may have left these as
    # symlinks to /usr/bin/*, and writing through those can hit ETXTBSY if a
    # service process still has the target mapped. Unlink first so we always
    # lay down a fresh inode in ~/.capsem/bin.
    rm -f "$dst"
    cp "$src" "$dst"
    chmod 755 "$dst"
done

for bin in capsem-admin; do
    src="$BIN_SRC/$bin"
    dst="$INSTALL_DIR/$bin"
    if [[ ! -f "$src" ]]; then
        continue
    fi
    rm -f "$dst"
    cp "$src" "$dst"
    chmod 755 "$dst"
done

if [[ -d "$BIN_SRC/capsem-admin-python" ]]; then
    rm -rf "$INSTALL_DIR/capsem-admin-python"
    cp -a "$BIN_SRC/capsem-admin-python" "$INSTALL_DIR/capsem-admin-python"
fi

# macOS local installs must mirror the package postinstall signing step.
# Apple Virtualization.framework rejects capsem-process without this
# entitlement, so an unsigned simulated install gives false release smoke
# failures even when the packaged payload is otherwise correct.
if [[ "$(uname -s)" == "Darwin" ]]; then
    ENTITLEMENTS_SRC="$(cd "$(dirname "$0")/.." && pwd)/entitlements.plist"
    if [[ ! -r "$ENTITLEMENTS_SRC" ]]; then
        echo "ERROR: entitlements.plist not found: $ENTITLEMENTS_SRC" >&2
        exit 1
    fi
    for bin in "$INSTALL_DIR"/capsem*; do
        [[ -f "$bin" ]] || continue
        if file "$bin" | grep -q 'Mach-O'; then
            codesign --sign - --entitlements "$ENTITLEMENTS_SRC" --force "$bin"
        fi
    done
fi

# Copy assets: manifest + the per-arch hash-named files. Matches the layout
# ManifestV2::resolve() actually reads: $ASSETS_DST/$ARCH/{hash_filename}.
if [[ -f "$ASSETS_SRC/manifest.json" ]]; then
    copy_if_different "$ASSETS_SRC/manifest.json" "$ASSETS_DST/manifest.json"
fi
if [[ -f "$ASSETS_SRC/manifest.json.minisig" ]]; then
    copy_if_different "$ASSETS_SRC/manifest.json.minisig" "$ASSETS_DST/manifest.json.minisig"
fi
if [[ -f "$ASSETS_SRC/manifest-sign.dev.pub" ]]; then
    copy_if_different "$ASSETS_SRC/manifest-sign.dev.pub" "$ASSETS_DST/manifest-sign.dev.pub"
fi

ARCH=$(uname -m)
[[ "$ARCH" == "aarch64" ]] && ARCH="arm64"

if [[ -d "$ASSETS_SRC/$ARCH" ]]; then
    mkdir -p "$ASSETS_DST/$ARCH"
    for src_file in "$ASSETS_SRC/$ARCH"/*; do
        [[ -f "$src_file" ]] || continue
        copy_if_different "$src_file" "$ASSETS_DST/$ARCH/$(basename "$src_file")"
    done
fi

PROFILE_SRC="$(cd "$(dirname "$0")" && pwd)/../config/profiles/base"
if [[ -d "$PROFILE_SRC" ]]; then
    find "$PROFILES_DST" -maxdepth 1 -type f -name '*.profile.toml' -delete
    if [[ -f "$ASSETS_SRC/manifest.json" ]]; then
        python3 "$(cd "$(dirname "$0")" && pwd)/materialize-install-profiles.py" \
            "$PROFILE_SRC" \
            "$ASSETS_SRC" \
            "$PROFILES_DST" \
            "$ASSETS_DST"
    else
        cp "$PROFILE_SRC"/*.profile.toml "$PROFILES_DST/"
    fi
else
    echo "ERROR: base profiles not found: $PROFILE_SRC" >&2
    exit 1
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
