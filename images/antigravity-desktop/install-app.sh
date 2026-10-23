#!/bin/sh
# Official Antigravity 2 App archives from https://antigravity.google/download.
# Verify bytes before extracting, then verify the product and target ELF.
set -eu
export DEBIAN_FRONTEND=noninteractive
arch="${1:?usage: install-app.sh <arm64|amd64>}"
APP_VERSION=2.26.0
APP_RELEASE=2.26.0-6546645232517120
case "$arch" in
    arm64)
        platform=linux-arm
        sha256=89ebda5b8ef96f806837aefce9c90678fbf1580f8cc8dc489bdf49a0f8611ef3
        machine=183
        ;;
    amd64)
        platform=linux-x64
        sha256=9f79730372f685d849b935187231edea5b38c5269e855e345e68b5c6071f7884
        machine=62
        ;;
    *) echo "unsupported Antigravity App architecture: $arch" >&2; exit 1 ;;
esac
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -fsSLo "$work/app.tar.gz" \
    "https://storage.googleapis.com/antigravity-public/antigravity-hub/$APP_RELEASE/$platform/Antigravity.tar.gz"
printf '%s  %s\n' "$sha256" "$work/app.tar.gz" | sha256sum -c -
package="$work/package"
install -d "$package/opt/antigravity" "$package/DEBIAN" "$package/usr/bin"
tar -xzf "$work/app.tar.gz" -C "$package/opt/antigravity" --strip-components=1 --no-same-owner
python3 - "$package/opt/antigravity" "$APP_VERSION" "$machine" <<'PY'
import json
import pathlib
import struct
import sys

root = pathlib.Path(sys.argv[1])
# Electron's root version file is the Electron version. The product identity
# lives in ASAR's JSON header/package.json; inspect it only after archive hash
# verification, without executing upstream JavaScript or extracting the ASAR.
with (root / 'resources/app.asar').open('rb') as archive:
    size, header_size, _, json_size = struct.unpack('<4I', archive.read(16))
    assert size == 4 and 0 < json_size <= header_size <= 4 << 20
    files = json.loads(archive.read(json_size))['files']
    entry = files['package.json']
    assert not entry.get('unpacked') and 0 < entry['size'] <= 1 << 20
    archive.seek(8 + header_size + int(entry['offset']))
    product = json.loads(archive.read(entry['size']))
assert product['name'] == 'antigravity' and product['productName'] == 'Antigravity', product
assert product['version'] == sys.argv[2], product
assert product['author']['name'] == 'Google', product
for name in ('antigravity', 'resources/bin/language_server'):
    with (root / name).open('rb') as binary:
        header = binary.read(20)
    assert header[:6] == b'\x7fELF\x02\x01', (name, header)
    assert int.from_bytes(header[18:20], 'little') == int(sys.argv[3]), (name, header)
PY
# Install a real dpkg-owned application package so the rootfs inventory records
# the verified App release as well as its upstream JavaScript components.
cat >"$package/DEBIAN/control" <<EOF
Package: antigravity
Version: $APP_VERSION
Architecture: $arch
Maintainer: Elie Bursztein <github@elie.net>
Section: editors
Priority: optional
Homepage: https://antigravity.google/
X-Capsem-Upstream-SHA256: $sha256
Depends: libasound2, libatk1.0-0, libatk-bridge2.0-0, libatspi2.0-0, libcairo2, libcups2,
 libdrm2, libgbm1, libglib2.0-0, libgtk-3-0, libnspr4, libnss3, libpango-1.0-0,
 libx11-6, libxcb1, libxcomposite1, libxdamage1, libxext6, libxfixes3,
 libxkbcommon0, libxrandr2, libxshmfence1, libxss1, libxtst6, xdg-utils
Description: Google Antigravity agent-first desktop application
 Official stable archive, packaged by Capsem after integrity and product verification.
EOF
ln -s /opt/antigravity/antigravity "$package/usr/bin/antigravity"
# Chromium uses its user-namespace sandbox under the runtime's no_new_privs.
# Its setuid helper must not grant privilege even outside Capsem.
chmod 0755 "$package/opt/antigravity/chrome-sandbox"
# This temporary package never ships; avoid compressing it before the final
# OCI/EROFS compression of the installed files.
dpkg-deb -Znone --build --root-owner-group "$package" "$work/antigravity.deb"
apt-get -o Acquire::Check-Valid-Until=false update
apt-get install -y --no-install-recommends "$work/antigravity.deb"
test "$(dpkg-query -W -f='${Version}' antigravity)" = "$APP_VERSION"
rm -rf /var/lib/apt/lists/*
