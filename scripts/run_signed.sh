#!/bin/bash
# Find the workspace root based on the script's location
DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
ROOT_DIR="$(dirname "$DIR")"

codesign --sign - --entitlements "$ROOT_DIR/entitlements.plist" --force "$1"
binary="$1"
shift
CAPSEM_ASSETS_DIR="$ROOT_DIR/assets" "$binary" "$@"
