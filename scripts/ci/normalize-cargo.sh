#!/usr/bin/env bash
set -euo pipefail

cargo_home="${CARGO_HOME:-$HOME/.cargo}"
cargo_bin="$cargo_home/bin"

if command -v cargo >/dev/null 2>&1 && cargo --version >/dev/null 2>&1; then
    cargo --version
    exit 0
fi

if ! command -v rustup >/dev/null 2>&1; then
    echo "rustup is required to repair the cargo proxy" >&2
    exit 1
fi

toolchain="${RUSTUP_TOOLCHAIN:-stable}"
if ! rustup which --toolchain "$toolchain" cargo >/dev/null 2>&1; then
    toolchain="stable"
    rustup which --toolchain "$toolchain" cargo >/dev/null
fi

mkdir -p "$cargo_bin"
cat > "$cargo_bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
toolchain="${RUSTUP_TOOLCHAIN:-stable}"
exec rustup run "$toolchain" cargo "$@"
EOF
chmod +x "$cargo_bin/cargo"

if [[ -n "${GITHUB_PATH:-}" ]]; then
    echo "$cargo_bin" >> "$GITHUB_PATH"
fi

"$cargo_bin/cargo" --version
