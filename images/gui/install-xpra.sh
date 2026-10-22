#!/bin/sh
# Shared server-only Xpra surface. Vendor packages are immutable SHA-256 pins;
# Debian dependencies resolve through the base image's dated snapshot.
set -eu
export DEBIAN_FRONTEND=noninteractive
arch="${1:?usage: install-xpra.sh <arm64|amd64>}"
XPRA_VERSION=6.5.1-r0-1
case "$arch" in
    arm64)
        common=ce68e85a976df7e2e34c5fe19be2a071c9595b554e22011bda33cf0f502ed58b
        server=28540b648a31ef8d469249ec859598e8bd0368ad5a4046e701c47ca51a8235d0
        x11=41c8ed007f6dafb32e3128dd3703afec9c52d76bea8b3a20af766a3199e762b6
        codecs=8669a2edcfc37854ba4cc4e8858215024399121ba23bae19a4a5115381d5f1d7
        ;;
    amd64)
        common=3156b4855a44a9be227f0151e0beaca980ec5d9534885e8d7a9307473a4bef6c
        server=58a1cfb36d1d790fb5e4a6c11903ea6729418d762a58064aa4d12a2cc4ec810e
        x11=4a6704492c0ca625e05b3d11fde815df916c0bd8764d8145c7149ae1c8f08a7d
        codecs=930b653ea352dd672342e797619e67e3702f74c4a3c0b086a0e46e478358af01
        ;;
    *) echo "unsupported Xpra architecture: $arch" >&2; exit 1 ;;
esac
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
chmod 0755 "$work"
fetch() {
    deb="${1}_${XPRA_VERSION}_${arch}.deb"
    curl -fsSLo "$work/$deb" "https://xpra.org/dists/bookworm/main/binary-$arch/$deb"
    printf '%s  %s\n' "$2" "$work/$deb" | sha256sum -c -
}
fetch xpra-common "$common"
fetch xpra-server "$server"
fetch xpra-x11 "$x11"
fetch xpra-codecs "$codecs"
apt-get -o Acquire::Check-Valid-Until=false update
apt-get install -y --no-install-recommends "$work"/*.deb \
    dbus-x11 xauth fontconfig fonts-dejavu-core libnss3-tools gnome-keyring libsecret-tools \
    xserver-xorg-core xserver-xorg-video-dummy
for package in xpra-common xpra-server xpra-x11 xpra-codecs; do
    test "$(dpkg-query -W -f='${Version}' "$package")" = "$XPRA_VERSION"
done
# Xpra 6.5.1's ServerBase no-op shadows XSettingsServer's implementation.
# Remove that obsolete override so initial attachment and resize update DPI
# through the existing X11 settings owner. Fail closed when the pinned source
# changes; re-evaluate this patch when upgrading Xpra.
python3 - <<'PY'
import hashlib
from pathlib import Path

path = Path('/usr/lib/python3/dist-packages/xpra/server/base.py')
source = path.read_bytes()
assert hashlib.sha256(source).hexdigest() == '0c31a16622d13030b3d4feea335e611986843f4aa9db4c411794ff63e1be8546'
obsolete = b'    def update_all_server_settings(self, reset: bool = False) -> None:\n        pass  # may be overridden in subclasses (ie: x11 server)\n\n'
assert source.count(obsolete) == 1
path.write_bytes(source.replace(obsolete, b''))
PY
# Xvfb cannot update hardware DPI. Use Xpra's rootless Xdummy backend and
# require the direct, non-setuid Xorg binary (never the legacy Xorg wrapper).
test -x /usr/lib/xorg/Xorg
test ! -u /usr/lib/xorg/Xorg
dpkg --compare-versions "$(dpkg-query -W -f='${Version}' xserver-xorg-video-dummy)" ge 0.4.0
# Never bake xpra-common's post-install-generated private TLS key into an image.
rm -f /etc/xpra/ssl/*.pem
install -d -m 1777 /tmp/.X11-unix
rm -rf /var/lib/apt/lists/*
