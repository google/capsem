#!/bin/sh
# Claude Desktop; Xpra and the graphical session are inherited from images/base.
#
# The pins are the ones the v0.7 GUI spike proved in an Apple VZ guest: Claude
# Desktop 1.22209.0. Every vendor .deb
# is checked against its SHA-256, and a mismatch fails the build.
#
# Claude comes from its signed apt repository, whose key is checked against
# its fingerprint first. Xpra's exact package pins live in the shared installer.
#
set -eu
export DEBIAN_FRONTEND=noninteractive

arch="${1:?usage: install-gui.sh <arm64|amd64>}"
CLAUDE_VERSION=1.22209.0
case "$arch" in
    arm64) CLAUDE_SHA256=7323fe6c3ab6b7078e81a9bf0200806e3486e73bc5873420ee9d26f10b66e1e9 ;;
    amd64) CLAUDE_SHA256=6d18ae792c2bddad01edc97c2c3f4cf489004cefe8fed6760a696ed25c49bf61 ;;
    *) echo "unsupported Claude Desktop architecture: $arch" >&2; exit 1 ;;
esac
CLAUDE_KEY_FINGERPRINT=31DDDE24DDFAB679F42D7BD2BAA929FF1A7ECACE

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
echo "deb [arch=$arch signed-by=/usr/share/keyrings/claude-desktop.asc] https://downloads.claude.ai/claude-desktop/apt/stable stable main" \
    >/etc/apt/sources.list.d/claude-desktop.list
apt-get -o Acquire::Check-Valid-Until=false update

(cd "$debs" && apt-get download "claude-desktop=$CLAUDE_VERSION")
echo "$CLAUDE_SHA256  $debs/claude-desktop_${CLAUDE_VERSION}_${arch}.deb" | sha256sum -c -

# One transaction, so trash-cli satisfies Claude's `kde-cli-tools | ... |
# trash-cli` alternative instead of apt pulling the KDE closure (344 packages
# against ~200). Xpra and its session/keyring/CA prerequisites are already present.
apt-get install -y --no-install-recommends "$debs"/*.deb trash-cli

assert_version() {
    installed="$(dpkg-query -W -f='${Version}' "$1")"
    if [ "$installed" != "$2" ]; then
        echo "$1 version mismatch: expected $2, got $installed" >&2
        exit 1
    fi
}
assert_version claude-desktop "$CLAUDE_VERSION"

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

# The vendor repository moves on, and the image never updates from it.
apt-get purge -y --auto-remove gpg
rm -f /etc/apt/sources.list.d/claude-desktop.list /usr/share/keyrings/claude-desktop.asc
rm -rf /var/lib/apt/lists/*
