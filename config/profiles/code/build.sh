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

install_agy() {
    AGY_VERSION="1.1.3"
    case "$(uname -m)" in
        aarch64|arm64)
            asset="agy_cli_linux_arm64.tar.gz"
            sha256="453f9c5530877ab6369e2536e576cfab2bbbcb45923a9bc776678142538e419d"
            ;;
        x86_64|amd64)
            asset="agy_cli_linux_x64.tar.gz"
            sha256="7a7239a69b65d3cf3af7e75f27b2ff4e9cce696a7b9a9e5c37c695f1c74eec34"
            ;;
        *)
            echo "unsupported AGY architecture: $(uname -m)" >&2
            exit 1
            ;;
    esac
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    archive="$tmp/$asset"
    curl -fsSL \
        "https://github.com/google-antigravity/antigravity-cli/releases/download/$AGY_VERSION/$asset" \
        -o "$archive"
    printf '%s  %s\n' "$sha256" "$archive" | sha256sum -c -
    tar -xzf "$archive" -C "$tmp"
    install -m 555 "$tmp/antigravity" /usr/local/bin/agy
    rm -rf "$tmp"
    trap - EXIT
}

install_from_url "https://claude.ai/install.sh" "claude"
install_agy

curl -fsSL https://ollama.com/install.sh | sh
command -v ollama >/dev/null 2>&1
rm -rf /usr/local/lib/ollama/cuda_*

cleanup_agent_runtime_state() {
    rm -rf \
        /root/.antigravity/*oauth* \
        /root/.antigravity/*token* \
        /root/.antigravity/cache \
        /root/.antigravity/history \
        /root/.antigravity/logs \
        /root/.claude/cache \
        /root/.claude/history \
        /root/.claude/logs \
        /root/.codex/cache \
        /root/.codex/history \
        /root/.codex/logs \
        /root/.gemini/cache \
        /root/.gemini/history \
        /root/.gemini/logs \
        /root/.gemini/tmp
}

if [ ! -x /usr/local/bin/agy-real ]; then
    install -m 555 /usr/local/bin/agy /usr/local/bin/agy-real
fi
cat >/usr/local/bin/agy <<'EOF'
#!/bin/sh
exec /usr/local/bin/agy-real --dangerously-skip-permissions "$@"
EOF
chmod 555 /usr/local/bin/agy

gemini_path="$(command -v gemini)"
gemini_dir="$(dirname "$gemini_path")"
gemini_target="$(readlink -f "$gemini_path")"
ln -sfn "$gemini_target" "$gemini_dir/gemini-real"
rm -f "$gemini_path"
cat >"$gemini_path" <<EOF
#!/bin/sh
cleanup_gemini_runtime_state() {
    rm -rf /root/.gemini/cache /root/.gemini/history /root/.gemini/logs /root/.gemini/tmp
}
trap cleanup_gemini_runtime_state EXIT INT TERM
"$gemini_target" "\$@"
status=$?
cleanup_gemini_runtime_state
exit "\$status"
EOF
chmod 555 "$gemini_path"

cleanup_agent_runtime_state
