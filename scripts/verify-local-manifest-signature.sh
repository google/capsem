#!/bin/bash
# Verify a local assets/ manifest using the release key or sibling dev key.
#
# Usage: verify-local-manifest-signature.sh [assets_dir] [release_pubkey]
set -euo pipefail

ASSETS_DIR="${1:-assets}"
RELEASE_PUBKEY="${2:-config/manifest-sign.pub}"
MANIFEST="$ASSETS_DIR/manifest.json"
SIGNATURE="$ASSETS_DIR/manifest.json.minisig"
DEV_PUBKEY="$ASSETS_DIR/manifest-sign.dev.pub"

if ! command -v minisign >/dev/null 2>&1; then
    echo "minisign not found"
    exit 2
fi

if [[ ! -f "$MANIFEST" ]]; then
    echo "manifest.json missing at $MANIFEST"
    exit 3
fi

if [[ ! -f "$SIGNATURE" ]]; then
    echo "manifest.json.minisig missing at $SIGNATURE"
    exit 4
fi

if [[ -f "$RELEASE_PUBKEY" ]] \
    && minisign -Vm "$MANIFEST" -x "$SIGNATURE" -p "$RELEASE_PUBKEY" >/dev/null 2>&1; then
    echo "manifest signature verifies with release key"
    exit 0
fi

if [[ -f "$DEV_PUBKEY" ]] \
    && minisign -Vm "$MANIFEST" -x "$SIGNATURE" -p "$DEV_PUBKEY" >/dev/null 2>&1; then
    echo "manifest signature verifies with dev key"
    exit 0
fi

if [[ ! -f "$DEV_PUBKEY" ]]; then
    echo "manifest-sign.dev.pub missing at $DEV_PUBKEY and release key did not verify"
    exit 5
fi

echo "manifest signature did not verify with release key or dev key"
exit 6
