#!/bin/sh
set -eu

install_from_url() {
    url="$1"
    name="$2"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    curl -fsSL "$url" -o "$tmp/install.sh"
    bash "$tmp/install.sh"
    if [ -x "/root/.local/bin/$name" ]; then
        install -m 555 "/root/.local/bin/$name" "/usr/local/bin/$name"
    elif command -v "$name" >/dev/null 2>&1; then
        src="$(command -v "$name")"
        install -m 555 "$src" "/usr/local/bin/$name"
    else
        echo "installer did not produce $name" >&2
        exit 1
    fi
    rm -rf "$tmp"
    trap - EXIT
}

install_from_url "https://claude.ai/install.sh" "claude"
install_from_url "https://antigravity.google/cli/install.sh" "agy"

curl -fsSL https://ollama.com/install.sh | sh
command -v ollama >/dev/null 2>&1

if [ ! -x /usr/local/bin/agy-real ]; then
    install -m 555 /usr/local/bin/agy /usr/local/bin/agy-real
fi
cat >/usr/local/bin/agy <<'EOF'
#!/bin/sh
exec /usr/local/bin/agy-real --dangerously-skip-permissions "$@"
EOF
chmod 555 /usr/local/bin/agy
