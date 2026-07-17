#!/bin/sh
set -eu

CLAUDE_VERSION="1.22209.0"
CLAUDE_SHA256="7323fe6c3ab6b7078e81a9bf0200806e3486e73bc5873420ee9d26f10b66e1e9"
CLAUDE_KEY_FINGERPRINT="31DDDE24DDFAB679F42D7BD2BAA929FF1A7ECACE"
XPRA_VERSION="6.5.1-r0-1"
XPRA_KEY_FINGERPRINT="B4993B57323148E37977E5D873254CAD17978FAF"

if [ "$(dpkg --print-architecture)" != "arm64" ]; then
    echo "GUI spike profile supports arm64 only" >&2
    exit 1
fi

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT INT TERM
package_dir="$work_dir/packages"
mkdir -p "$package_dir" /usr/share/keyrings /etc/apt/sources.list.d

verify_key() {
    key_path="$1"
    expected="$2"
    actual="$(gpg --batch --with-colons --show-keys "$key_path" \
        | awk -F: '$1 == "fpr" { print $10; exit }')"
    if [ "$actual" != "$expected" ]; then
        echo "signing key fingerprint mismatch: expected $expected, got $actual" >&2
        exit 1
    fi
}

curl -fsSLo "$work_dir/claude-key.asc" \
    https://downloads.claude.ai/claude-desktop/key.asc
verify_key "$work_dir/claude-key.asc" "$CLAUDE_KEY_FINGERPRINT"
install -m 0644 "$work_dir/claude-key.asc" \
    /usr/share/keyrings/claude-desktop-archive-keyring.asc

curl -fsSLo "$work_dir/xpra-key.asc" https://xpra.org/gpg.asc
verify_key "$work_dir/xpra-key.asc" "$XPRA_KEY_FINGERPRINT"
install -m 0644 "$work_dir/xpra-key.asc" /usr/share/keyrings/xpra.asc

cat >/etc/apt/sources.list.d/claude-desktop.list <<'EOF'
deb [arch=arm64 signed-by=/usr/share/keyrings/claude-desktop-archive-keyring.asc] https://downloads.claude.ai/claude-desktop/apt/stable stable main
EOF

cat >/etc/apt/sources.list.d/xpra.sources <<'EOF'
Types: deb
URIs: https://xpra.org
Suites: bookworm
Components: main
Signed-By: /usr/share/keyrings/xpra.asc
Architectures: arm64
EOF

apt-get -o Acquire::Check-Valid-Until=false -o Acquire::Check-Date=false update

download_package() {
    package="$1"
    version="$2"
    expected_sha256="$3"
    filename="$4"
    (
        cd "$package_dir"
        apt-get download "$package=$version"
    )
    echo "$expected_sha256  $package_dir/$filename" | sha256sum -c -
}

download_package \
    claude-desktop "$CLAUDE_VERSION" "$CLAUDE_SHA256" \
    "claude-desktop_${CLAUDE_VERSION}_arm64.deb"
download_package \
    xpra-common "$XPRA_VERSION" \
    ce68e85a976df7e2e34c5fe19be2a071c9595b554e22011bda33cf0f502ed58b \
    "xpra-common_${XPRA_VERSION}_arm64.deb"
download_package \
    xpra-server "$XPRA_VERSION" \
    28540b648a31ef8d469249ec859598e8bd0368ad5a4046e701c47ca51a8235d0 \
    "xpra-server_${XPRA_VERSION}_arm64.deb"
download_package \
    xpra-x11 "$XPRA_VERSION" \
    41c8ed007f6dafb32e3128dd3703afec9c52d76bea8b3a20af766a3199e762b6 \
    "xpra-x11_${XPRA_VERSION}_arm64.deb"
download_package \
    xpra-codecs "$XPRA_VERSION" \
    8669a2edcfc37854ba4cc4e8858215024399121ba23bae19a4a5115381d5f1d7 \
    "xpra-codecs_${XPRA_VERSION}_arm64.deb"

apt-get install -y --no-install-recommends \
    "$package_dir/claude-desktop_${CLAUDE_VERSION}_arm64.deb" \
    "$package_dir/xpra-common_${XPRA_VERSION}_arm64.deb" \
    "$package_dir/xpra-server_${XPRA_VERSION}_arm64.deb" \
    "$package_dir/xpra-x11_${XPRA_VERSION}_arm64.deb" \
    "$package_dir/xpra-codecs_${XPRA_VERSION}_arm64.deb"

assert_version() {
    package="$1"
    expected="$2"
    actual="$(dpkg-query -W -f='${Version}' "$package")"
    if [ "$actual" != "$expected" ]; then
        echo "$package version mismatch: expected $expected, got $actual" >&2
        exit 1
    fi
}

assert_version claude-desktop "$CLAUDE_VERSION"
assert_version xpra-common "$XPRA_VERSION"
assert_version xpra-server "$XPRA_VERSION"
assert_version xpra-x11 "$XPRA_VERSION"
assert_version xpra-codecs "$XPRA_VERSION"

command -v claude-desktop >/dev/null
command -v xpra >/dev/null
xpra showconfig | grep -q "bind-vsock"

# Claude Desktop is Electron. Run the application as a dedicated unprivileged
# identity and keep Chromium's normal SUID sandbox available; never paper over
# a broken image with a runtime sandbox-bypass flag.
sandbox_helper=/usr/lib/claude-desktop/chrome-sandbox
chown root:root "$sandbox_helper"
chmod 4755 "$sandbox_helper"
if [ "$(stat -c '%u:%g:%a' "$sandbox_helper")" != "0:0:4755" ]; then
    echo "Claude Chromium sandbox helper must be root:root mode 4755" >&2
    exit 1
fi

for forbidden in \
    kde-cli-tools chromium firefox-esr epiphany-browser \
    gnome-shell plasma-desktop xfce4-session openbox \
    tigervnc-standalone-server x11vnc websockify socat openssh-server; do
    if dpkg-query -W -f='${Status}' "$forbidden" 2>/dev/null \
        | grep -q "install ok installed"; then
        echo "forbidden GUI package installed: $forbidden" >&2
        exit 1
    fi
done

rm -rf /var/lib/apt/lists/* "$package_dir"
trap - EXIT INT TERM
rm -rf "$work_dir"
