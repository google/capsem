#!/bin/sh
# Claude Desktop and the Xpra server that streams it, on the Capsem base.
#
# The pins are the ones the v0.7 GUI spike proved in an Apple VZ guest: Claude
# Desktop 1.22209.0 and the server-only Xpra 6.5.1 subset (not the `xpra`
# metapackage, which hard-depends on the unused GTK client). Every vendor .deb
# is checked against its SHA-256, and a mismatch fails the build.
#
# Claude comes from its signed apt repository, whose key is checked against
# its fingerprint first. Xpra's debs are fetched from their pool path: the
# signed xpra.org bookworm InRelease lists main/binary-arm64/Packages as empty
# (0 bytes, beside a populated Packages.gz), so apt finds no arm64 package in
# it. The spike took the SHA-256 pins below from that index, signature
# checked against key B4993B57323148E37977E5D873254CAD17978FAF, when it was
# whole.
#
# arm64 only: the spike never built x86_64, and these pins are arm64 debs.
set -eu
export DEBIAN_FRONTEND=noninteractive

arch="${1:?usage: install-gui.sh <arm64>}"
if [ "$arch" != arm64 ]; then
    echo "images/claude-desktop supports arm64 only, not $arch" >&2
    exit 1
fi

CLAUDE_VERSION=1.22209.0
CLAUDE_SHA256=7323fe6c3ab6b7078e81a9bf0200806e3486e73bc5873420ee9d26f10b66e1e9
CLAUDE_KEY_FINGERPRINT=31DDDE24DDFAB679F42D7BD2BAA929FF1A7ECACE
XPRA_VERSION=6.5.1-r0-1

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
debs="$work/debs"
mkdir -p "$debs"
# apt downloads as _apt, which must be able to reach and write the directory.
chmod 0755 "$work"
chown _apt "$debs"

apt-get -o Acquire::Check-Valid-Until=false update
# gpg only to read the vendor key's fingerprint; purged again below.
apt-get install -y --no-install-recommends gpg

curl -fsSLo "$work/claude.asc" https://downloads.claude.ai/claude-desktop/key.asc
actual="$(gpg --batch --with-colons --show-keys "$work/claude.asc" | awk -F: '$1 == "fpr" { print $10; exit }')"
if [ "$actual" != "$CLAUDE_KEY_FINGERPRINT" ]; then
    echo "Claude signing key fingerprint mismatch: expected $CLAUDE_KEY_FINGERPRINT, got $actual" >&2
    exit 1
fi
install -m 0644 "$work/claude.asc" /usr/share/keyrings/claude-desktop.asc
echo "deb [arch=arm64 signed-by=/usr/share/keyrings/claude-desktop.asc] https://downloads.claude.ai/claude-desktop/apt/stable stable main" \
    >/etc/apt/sources.list.d/claude-desktop.list
apt-get -o Acquire::Check-Valid-Until=false update

(cd "$debs" && apt-get download "claude-desktop=$CLAUDE_VERSION")
echo "$CLAUDE_SHA256  $debs/claude-desktop_${CLAUDE_VERSION}_arm64.deb" | sha256sum -c -

xpra_fetch() {
    deb="${1}_${XPRA_VERSION}_arm64.deb"
    curl -fsSLo "$debs/$deb" "https://xpra.org/dists/bookworm/main/binary-arm64/$deb"
    echo "$2  $debs/$deb" | sha256sum -c -
}
xpra_fetch xpra-common ce68e85a976df7e2e34c5fe19be2a071c9595b554e22011bda33cf0f502ed58b
xpra_fetch xpra-server 28540b648a31ef8d469249ec859598e8bd0368ad5a4046e701c47ca51a8235d0
xpra_fetch xpra-x11 41c8ed007f6dafb32e3128dd3703afec9c52d76bea8b3a20af766a3199e762b6
xpra_fetch xpra-codecs 8669a2edcfc37854ba4cc4e8858215024399121ba23bae19a4a5115381d5f1d7

# One transaction, so trash-cli satisfies Claude's `kde-cli-tools | ... |
# trash-cli` alternative instead of apt pulling the KDE closure (344 packages
# against ~200). The rest is the single-application X11 surface: the D-Bus
# session, the Secret Service Claude keeps its login in, and certutil for
# Chromium's NSS trust store. Xpra renders into Xvfb, which xpra-server
# depends on and its packaged configuration starts; no Xorg driver is needed.
apt-get install -y --no-install-recommends \
    "$debs"/*.deb \
    trash-cli dbus-x11 xauth fontconfig fonts-dejavu-core libnss3-tools gnome-keyring libsecret-tools

assert_version() {
    installed="$(dpkg-query -W -f='${Version}' "$1")"
    if [ "$installed" != "$2" ]; then
        echo "$1 version mismatch: expected $2, got $installed" >&2
        exit 1
    fi
}
assert_version claude-desktop "$CLAUDE_VERSION"
for package in xpra-common xpra-server xpra-x11 xpra-codecs; do
    assert_version "$package" "$XPRA_VERSION"
done

# No desktop, window manager, browser or remote-desktop stack: Xpra serves the
# one application, and the gateway serves the HTML5 client.
for forbidden in kde-cli-tools chromium firefox-esr epiphany-browser gnome-shell plasma-desktop \
    xfce4-session openbox tigervnc-standalone-server x11vnc websockify socat openssh-server xpra-html5; do
    if dpkg-query -W -f='${Status}' "$forbidden" 2>/dev/null | grep -q "install ok installed"; then
        echo "forbidden GUI package installed: $forbidden" >&2
        exit 1
    fi
done

for tool in claude-desktop xpra gnome-keyring-daemon secret-tool certutil dbus-run-session; do
    command -v "$tool" >/dev/null
done

# xpra-common's postinst generates a TLS key pair. The surface serves no TLS,
# and a private key baked into a published image is everyone's key.
rm -f /etc/xpra/ssl/*.pem

# Only root may create the X socket directory, and Xvfb runs as the user.
install -d -m 1777 /tmp/.X11-unix

# The vendor repository moves on, and the image never updates from it.
apt-get purge -y --auto-remove gpg
rm -f /etc/apt/sources.list.d/claude-desktop.list /usr/share/keyrings/claude-desktop.asc
rm -rf /var/lib/apt/lists/*
