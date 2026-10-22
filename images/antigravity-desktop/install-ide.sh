#!/bin/sh
# Official standalone IDE archives from https://antigravity.google/download.
# Verify bytes before extracting, then verify the product and target ELF.
set -eu
export DEBIAN_FRONTEND=noninteractive
arch="${1:?usage: install-ide.sh <arm64|amd64>}"
IDE_VERSION=2.5.5
IDE_RELEASE=2.5.5-4923483625488384
IDE_COMMIT=ecfbad74d93962fc8ca485d93ab9b4f3d4cb6cf8
case "$arch" in
    arm64)
        platform=linux-arm
        sha256=88c167108980c33a223a8d7f0aa6aaf4dec61f0cc3950235a2698e9ffc38a49e
        machine=183
        ;;
    amd64)
        platform=linux-x64
        sha256=0c5233b297d2b3aebb61af49f8944012c2953d361a5ebb16978490636917f831
        machine=62
        ;;
    *) echo "unsupported Antigravity IDE architecture: $arch" >&2; exit 1 ;;
esac
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -fsSLo "$work/ide.tar.gz" \
    "https://edgedl.me.gvt1.com/edgedl/release2/j0qc3/antigravity/stable/$IDE_RELEASE/$platform/Antigravity%20IDE.tar.gz"
printf '%s  %s\n' "$sha256" "$work/ide.tar.gz" | sha256sum -c -
package="$work/package"
install -d "$package/opt/antigravity-ide" "$package/DEBIAN" "$package/usr/bin"
tar -xzf "$work/ide.tar.gz" -C "$package/opt/antigravity-ide" --strip-components=1 --no-same-owner
python3 - "$package/opt/antigravity-ide" "$IDE_VERSION" "$IDE_COMMIT" "$machine" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
product = json.loads((root / 'resources/app/product.json').read_text())
assert product['ideVersion'] == sys.argv[2], product
assert product['commit'] == sys.argv[3], product
assert product['applicationName'] == 'antigravity-ide', product
assert product['quality'] == 'stable', product
with (root / 'antigravity-ide').open('rb') as binary:
    header = binary.read(20)
assert header[:6] == b'\x7fELF\x02\x01', header
assert int.from_bytes(header[18:20], 'little') == int(sys.argv[4]), header
PY
# The archive's package.json carries VS Code's base version, not ideVersion.
# Install a real dpkg-owned application package so the rootfs inventory records
# the verified IDE release as well as those upstream JavaScript components.
cat >"$package/DEBIAN/control" <<EOF
Package: antigravity-ide
Version: $IDE_VERSION
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
Description: Google Antigravity IDE standalone coding editor
 Official stable archive, packaged by Capsem after integrity and product verification.
EOF
ln -s /opt/antigravity-ide/bin/antigravity-ide "$package/usr/bin/antigravity-ide"
# Chromium uses its user-namespace sandbox under the runtime's no_new_privs.
# Its setuid helper must not grant privilege even outside Capsem.
chmod 0755 "$package/opt/antigravity-ide/chrome-sandbox"
# This temporary package never ships; avoid compressing it before the final
# OCI/EROFS compression of the installed files.
dpkg-deb -Znone --build --root-owner-group "$package" "$work/antigravity-ide.deb"
apt-get -o Acquire::Check-Valid-Until=false update
apt-get install -y --no-install-recommends "$work/antigravity-ide.deb"
test "$(dpkg-query -W -f='${Version}' antigravity-ide)" = "$IDE_VERSION"
rm -rf /var/lib/apt/lists/*
