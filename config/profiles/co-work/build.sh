#!/bin/sh
set -eu

install_exact_binary() {
    url="$1"
    sha256="$2"
    destination="$3"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    payload="$tmp/payload"
    curl -fsSL "$url" -o "$payload"
    printf '%s  %s\n' "$sha256" "$payload" | sha256sum -c -
    install -m 555 "$payload" "$destination"
    rm -rf "$tmp"
    trap - EXIT
}

install_exact_ollama() {
    url="$1"
    sha256="$2"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    archive="$tmp/ollama.tar.zst"
    curl -fsSL "$url" -o "$archive"
    printf '%s  %s\n' "$sha256" "$archive" | sha256sum -c -
    zstd -dc "$archive" | tar -xf - -C /usr
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

: "${CAPSEM_CLAUDE_URL:?}"
: "${CAPSEM_CLAUDE_SHA256:?}"
: "${CAPSEM_CLAUDE_VERSION:?}"
: "${CAPSEM_OLLAMA_URL:?}"
: "${CAPSEM_OLLAMA_SHA256:?}"
: "${CAPSEM_OLLAMA_VERSION:?}"

install_exact_binary "$CAPSEM_CLAUDE_URL" "$CAPSEM_CLAUDE_SHA256" /usr/local/bin/claude
claude --version | grep -F "$CAPSEM_CLAUDE_VERSION"
install_agy

install_exact_ollama "$CAPSEM_OLLAMA_URL" "$CAPSEM_OLLAMA_SHA256"
command -v ollama >/dev/null 2>&1
ollama --version 2>&1 | grep -F "$CAPSEM_OLLAMA_VERSION"
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
