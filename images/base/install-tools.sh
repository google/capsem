#!/bin/sh
# The Capsem base image's toolchain: Node, uv, and Ollama without any GPU
# runtime. Every download is pinned by version and SHA-256; the pins match
# config/docker/image/build.toml, which the VM rootfs uses until profiles are
# removed. A mismatch fails the build.
set -eu

arch="${1:?usage: install-tools.sh <arm64|amd64>}"

NODE_VERSION=24.19.0
UV_VERSION=0.12.3
OLLAMA_VERSION=0.32.9

case "$arch" in
    arm64)
        node_asset="node-v$NODE_VERSION-linux-arm64.tar.xz"
        node_sha256=01443c1e1a29e531ccad5a46fefa6df490d2189c49f7955904aecdbb0fe86fdc
        uv_asset="uv-aarch64-unknown-linux-gnu.tar.gz"
        uv_sha256=bb66cb52e7b1823aed1183630d8d8e5c958840d584a4c55ec10a4cfc168dcca2
        ollama_asset="ollama-linux-arm64.tar.zst"
        ollama_sha256=79617139521db251c716d6229505d7530171ff4d68e476d0c22dabc15726a237
        ;;
    amd64)
        node_asset="node-v$NODE_VERSION-linux-x64.tar.xz"
        node_sha256=14b342e71204f811bde6153be8e04b62aef63c236fef92b55f9c83154b409647
        uv_asset="uv-x86_64-unknown-linux-gnu.tar.gz"
        uv_sha256=600cf9a742aca00d292673b16b5acffaa7b8c269a364ad0c2e79498dcb1fe101
        ollama_asset="ollama-linux-amd64.tar.zst"
        ollama_sha256=5d747a43369f61e38f20b5a39fcc5c90647e562cdc61e2e56034f1c5b113d540
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

fetch "https://github.com/ollama/ollama/releases/download/v$OLLAMA_VERSION/$ollama_asset" "$ollama_asset" "$ollama_sha256"
zstd -dc "$work/$ollama_asset" | tar -xf - -C /usr --no-same-owner
# CPU only: no image carries a GPU runtime it cannot use.
rm -rf /usr/lib/ollama/cuda* /usr/lib/ollama/hip* /usr/lib/ollama/jetpack* \
    /usr/lib/ollama/oneapi* /usr/lib/ollama/opencl* /usr/lib/ollama/rocm* \
    /usr/lib/ollama/vulkan* /usr/lib/ollama/mlx*
if find /usr/lib/ollama -mindepth 1 -maxdepth 1 -type d | grep -q .; then
    echo "a GPU runtime survived in /usr/lib/ollama:" >&2
    find /usr/lib/ollama -mindepth 1 -maxdepth 1 -type d >&2
    exit 1
fi

node --version | grep -Fx "v$NODE_VERSION"
uv --version | grep -F "$UV_VERSION"
ollama --version 2>&1 | grep -F "$OLLAMA_VERSION"
