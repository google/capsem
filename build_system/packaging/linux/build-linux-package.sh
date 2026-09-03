#!/bin/bash
# Build the Linux release cohort inside the sealed package helper, for one target.
#
# Lifted verbatim out of a `docker run ... bash -c "..."` argument in the
# justfile, where every quote was escaped twice and the whole program was one
# logical line. As a file it can be read, syntax-checked by
# build_system/scripts/audit/check-source-syntax.py, and edited without counting backslashes.
#
# Everything it needs arrives in the environment, set by capsem.gate.crosscompile:
#
#   TARGET_ARCH        capsem's name for the target (arm64, x86_64)
#   RUST_TARGET        the cargo/rustup triple
#   DPKG_ARCH          the Debian architecture name
#   RUST_TOOLCHAIN     the pinned toolchain, read from rust-toolchain.toml
#   HOST_UID/HOST_GID  the user that must own anything written to the bind mount
#   CAPSEM_INSTALL_MANIFEST_URL   baked into the package as its default channel
#   TAURI_SIGNING_PRIVATE_KEY[_PASSWORD]   optional; a dev key is made if absent
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

: "${TARGET_ARCH:?}" "${RUST_TARGET:?}" "${DPKG_ARCH:?}" "${RUST_TOOLCHAIN:?}"
: "${CARGO_HOME:?}" "${CARGO_TARGET_DIR:?}" "${CAPSEM_PNPM_STORE:?}"
: "${HOST_UID:?}" "${HOST_GID:?}" "${CAPSEM_INSTALL_MANIFEST_URL:?}"

# Where this build's artifacts go. A container path by default, copied out with
# `docker cp` afterwards: writing the package back through the bind mount is
# what let a host step churning the same tree kill a release on an intermittent
# EACCES, and it made "the builder produced this" and "the host can read it"
# the same event. Overridable so a caller outside the gate can still aim it at
# the mount.
OUT="${CAPSEM_PACKAGE_OUTPUT_DIR:-/src/dist}"
RELEASE_DIR="$CARGO_TARGET_DIR/$RUST_TARGET/release"
AGENT_DIR="$CARGO_TARGET_DIR/build/linux-agent/$TARGET_ARCH"
mkdir -p "$OUT"

# Only the output. `/src` is mounted read-only and the paths this build writes
# into it -- node_modules, the frontend bundle, Tauri's generated ACLs -- are
# container-local scratch, so there is nothing of the host's left owned by root.
trap 'chown -R "$HOST_UID:$HOST_GID" "$OUT" 2>/dev/null || true' EXIT

echo "--- Verify pinned Rust target ---"
if ! rustup show active-toolchain | grep -F "$RUST_TOOLCHAIN-" >/dev/null; then
    echo "ERROR: package helper does not carry pinned Rust $RUST_TOOLCHAIN" >&2
    exit 1
fi
if ! rustup target list --toolchain "$RUST_TOOLCHAIN" --installed \
    | grep -Fx "$RUST_TARGET" >/dev/null; then
    echo "ERROR: pinned Rust $RUST_TOOLCHAIN target $RUST_TARGET is unavailable in the capsem-rustup cache" >&2
    exit 1
fi

echo "--- Build frontend ---"
(cd web/app && CI=true pnpm install --offline --frozen-lockfile \
    --store-dir "$CAPSEM_PNPM_STORE")
bash build_system/scripts/web/check-web-surface.sh frontend-build

echo "--- Build agent binaries ---"
cargo build --release --locked --offline --target "$RUST_TARGET" -p capsem-agent
mkdir -p "$AGENT_DIR"
cp "$RELEASE_DIR/capsem-pty-agent" \
   "$RELEASE_DIR/capsem-mcp-server" \
   "$RELEASE_DIR/capsem-net-proxy" \
   "$RELEASE_DIR/capsem-dns-proxy" \
   "$RELEASE_DIR/capsem-sysutil" \
   "$AGENT_DIR/"

echo "--- Build companion host binaries ---"
cargo build --release --locked --offline --target "$RUST_TARGET" \
    -p capsem -p capsem-service -p capsem-process -p capsem-tui -p capsem-mcp \
    -p capsem-mcp-aggregator -p capsem-mcp-builtin -p capsem-gateway \
    -p capsem-tray -p capsem-admin -p capsem-mock-server -p capsem-bench
bash build_system/scripts/build/check-build-provenance.sh "$RELEASE_DIR/capsem" \
    "${CAPSEM_BUILD_REVISION:-}"

echo "--- Resolve Tauri signing key ---"
# The authoritative release keys live in GitHub Actions secrets and are applied
# only on publish. A local build just needs SOME key for `cargo tauri build` to
# complete, so one throwaway dev key is generated and reused.
DEV_KEY="$CARGO_TARGET_DIR/dev-tauri-private"
if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    if [ ! -f "$DEV_KEY" ]; then
        echo "    no host signing key; generating dev-only key (not for release distribution)"
        cargo tauri signer generate --ci --force -w "$DEV_KEY" -p 'dev' >/dev/null
    else
        echo "    reusing dev key at $DEV_KEY"
    fi
    TAURI_SIGNING_PRIVATE_KEY=$(cat "$DEV_KEY")
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD='dev'
    export TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD
else
    echo "    using host-injected signing key"
fi

echo "--- Build Tauri app ---"
rm -rf "$RELEASE_DIR/bundle/deb"
(cd crates/capsem-app && cargo tauri build --target "$RUST_TARGET" --bundles deb \
    -- --locked --offline)

echo "--- Repack Debian package ---"
DEB=$(ls -t "$RELEASE_DIR/bundle/deb/"*.deb | head -n1)
bash "$SCRIPT_DIR/repack-deb.sh" --manifest "$CAPSEM_INSTALL_MANIFEST_URL" "$DEB" \
    "$RELEASE_DIR" "cache/target/config" "assets"

echo "--- Validate artifacts ---"
dpkg-deb --info "$DEB"
dpkg-deb --contents "$DEB" | grep -E 'usr/bin/(capsem|capsem-service|capsem-process|capsem-tui|capsem-mcp|capsem-mcp-aggregator|capsem-mcp-builtin|capsem-gateway|capsem-tray|capsem-admin|capsem-mock-server|capsem-bench-rs)$'

cp "$DEB" "$OUT/"
# Record the exact package this run produced, so a stale cache/target/packages entry
# from an earlier build can never be the one that gets proved and published.
basename "$DEB" > "$OUT/.cross-compile-$TARGET_ARCH-deb"
cp "$AGENT_DIR/"* "$OUT/"
