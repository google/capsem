#!/bin/sh
# capsem-debug's downloaded tools: Node (and npm), uv, Ollama on CPU, Claude
# Code and Antigravity (agy). Every download is pinned by version and SHA-256,
# the same pins as images/base/install-tools.sh and the claude-code and agy
# images. A mismatch fails the build.
set -eu

arch="${1:?usage: install-tools.sh <arm64|amd64>}"

NODE_VERSION=24.19.0
UV_VERSION=0.12.3
OLLAMA_VERSION=0.32.9
CLAUDE_VERSION=2.1.229
AGY_VERSION=1.1.3

case "$arch" in
    arm64)
        node_asset="node-v$NODE_VERSION-linux-arm64.tar.xz"
        node_sha256=01443c1e1a29e531ccad5a46fefa6df490d2189c49f7955904aecdbb0fe86fdc
        uv_asset="uv-aarch64-unknown-linux-gnu.tar.gz"
        uv_sha256=bb66cb52e7b1823aed1183630d8d8e5c958840d584a4c55ec10a4cfc168dcca2
        ollama_asset="ollama-linux-arm64.tar.zst"
        ollama_sha256=79617139521db251c716d6229505d7530171ff4d68e476d0c22dabc15726a237
        claude_platform=linux-arm64
        claude_sha256=ba53130acbda3ddb00b3dd5641f11733867f5d837a258b7e6ccef4927cc1a509
        agy_asset=agy_cli_linux_arm64.tar.gz
        agy_sha256=453f9c5530877ab6369e2536e576cfab2bbbcb45923a9bc776678142538e419d
        ;;
    amd64)
        node_asset="node-v$NODE_VERSION-linux-x64.tar.xz"
        node_sha256=14b342e71204f811bde6153be8e04b62aef63c236fef92b55f9c83154b409647
        uv_asset="uv-x86_64-unknown-linux-gnu.tar.gz"
        uv_sha256=600cf9a742aca00d292673b16b5acffaa7b8c269a364ad0c2e79498dcb1fe101
        ollama_asset="ollama-linux-amd64.tar.zst"
        ollama_sha256=5d747a43369f61e38f20b5a39fcc5c90647e562cdc61e2e56034f1c5b113d540
        claude_platform=linux-x64
        claude_sha256=200338139a3df04a9ad22233837d1fb53fb6dffa21cd82e47559bfaa115acc1b
        agy_asset=agy_cli_linux_x64.tar.gz
        agy_sha256=7a7239a69b65d3cf3af7e75f27b2ff4e9cce696a7b9a9e5c37c695f1c74eec34
        ;;
    *)
        echo "unsupported architecture: $arch" >&2
        exit 1
        ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

fetch() {
    curl -fsSL "$1" -o "$work/$2"
    printf '%s  %s\n' "$3" "$work/$2" | sha256sum -c -
}

fetch "https://nodejs.org/dist/v$NODE_VERSION/$node_asset" "$node_asset" "$node_sha256"
tar -xJf "$work/$node_asset" -C /usr/local --strip-components=1 --no-same-owner \
    --exclude='*/CHANGELOG.md' --exclude='*/README.md' --exclude='*/include'

fetch "https://github.com/astral-sh/uv/releases/download/$UV_VERSION/$uv_asset" "$uv_asset" "$uv_sha256"
tar -xzf "$work/$uv_asset" -C "$work"
install -m 555 "$work"/uv-*/uv "$work"/uv-*/uvx /usr/local/bin/

fetch "https://downloads.claude.ai/claude-code-releases/$CLAUDE_VERSION/$claude_platform/claude" claude "$claude_sha256"
install -m 555 "$work/claude" /usr/local/bin/claude

fetch "https://github.com/ollama/ollama/releases/download/v$OLLAMA_VERSION/$ollama_asset" "$ollama_asset" "$ollama_sha256"
zstd -dc "$work/$ollama_asset" | tar -xf - -C /usr --no-same-owner
# CPU only, as in the base image: a GPU runtime is dead weight in a VM.
rm -rf /usr/lib/ollama/cuda* /usr/lib/ollama/hip* /usr/lib/ollama/jetpack* \
    /usr/lib/ollama/oneapi* /usr/lib/ollama/opencl* /usr/lib/ollama/rocm* \
    /usr/lib/ollama/vulkan* /usr/lib/ollama/mlx*

fetch "https://github.com/google-antigravity/antigravity-cli/releases/download/$AGY_VERSION/$agy_asset" "$agy_asset" "$agy_sha256"
tar -xzf "$work/$agy_asset" -C "$work"
install -m 555 "$work/antigravity" /usr/local/bin/agy

node --version | grep -Fx "v$NODE_VERSION"
uv --version | grep -F "$UV_VERSION"
ollama --version 2>&1 | grep -F "$OLLAMA_VERSION"
claude --version | grep -F "$CLAUDE_VERSION"
