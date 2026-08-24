#!/usr/bin/env bash
# Build small, parseable package fixtures for the binary-channel staging workflow.
set -euo pipefail

if [ "$#" -ne 3 ]; then
    echo "usage: $0 <version> <artifact-dir> <work-dir>" >&2
    exit 2
fi

VERSION=$1
ARTIFACT_ROOT=$2
WORK_ROOT=$3
if [[ ! "$VERSION" =~ ^[0-9A-Za-z][0-9A-Za-z._-]*$ || "$VERSION" == v* ]]; then
    echo "invalid staging version: $VERSION" >&2
    exit 2
fi

DEB_ROOT="$WORK_ROOT/deb"
PKG_ROOT="$WORK_ROOT/pkg"
DEB="$ARTIFACT_ROOT/Capsem_${VERSION}_arm64.deb"
PKG="$ARTIFACT_ROOT/Capsem-${VERSION}.pkg"
SBOM="$ARTIFACT_ROOT/capsem-sbom.spdx.json"
for path in "$WORK_ROOT" "$DEB" "$PKG" "$SBOM"; do
    if [ -e "$path" ]; then
        echo "refusing stale binary staging path: $path" >&2
        exit 1
    fi
done

mkdir -p \
    "$ARTIFACT_ROOT" \
    "$DEB_ROOT/DEBIAN" \
    "$DEB_ROOT/usr/bin" \
    "$PKG_ROOT/Capsem.pkg/Payload/Applications/Capsem.app/Contents/MacOS"
printf '%s\n' \
    'Package: capsem' \
    "Version: $VERSION" \
    'Section: utils' \
    'Priority: optional' \
    'Architecture: arm64' \
    'Maintainer: Capsem Release Staging <release@capsem.org>' \
    'Description: deterministic Capsem binary channel staging fixture' \
    > "$DEB_ROOT/DEBIAN/control"
for binary in capsem-app capsem-tray; do
    printf '#!/bin/sh\nexit 0\n' > "$DEB_ROOT/usr/bin/$binary"
    printf '#!/bin/sh\nexit 0\n' \
        > "$PKG_ROOT/Capsem.pkg/Payload/Applications/Capsem.app/Contents/MacOS/$binary"
    chmod 0755 \
        "$DEB_ROOT/usr/bin/$binary" \
        "$PKG_ROOT/Capsem.pkg/Payload/Applications/Capsem.app/Contents/MacOS/$binary"
done
find "$WORK_ROOT" -exec touch -h -d @0 {} +

SOURCE_DATE_EPOCH=0 dpkg-deb --build --root-owner-group "$DEB_ROOT" "$DEB"
tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
    -czf "$PKG" -C "$PKG_ROOT" .
SCRIPT_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
python3 "$SCRIPT_ROOT/generate-host-binary-sbom.py" --output "$SBOM" "$DEB"
