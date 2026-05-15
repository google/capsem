#!/usr/bin/env bash
set -euo pipefail

if ! command -v rustup >/dev/null 2>&1; then
    echo "rustup is required to repair the cargo proxy" >&2
    exit 1
fi

toolchain="${RUSTUP_TOOLCHAIN:-stable}"
if ! real_cargo="$(rustup which --toolchain "$toolchain" cargo 2>/dev/null)"; then
    toolchain="stable"
    real_cargo="$(rustup which --toolchain "$toolchain" cargo)"
fi

shim_dir="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/capsem-cargo-bin"
mkdir -p "$shim_dir"
cat > "$shim_dir/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
toolchain="${RUSTUP_TOOLCHAIN:-stable}"
exec rustup run "$toolchain" cargo "$@"
EOF
chmod +x "$shim_dir/cargo"

if [[ -n "${GITHUB_PATH:-}" ]]; then
    echo "$shim_dir" >> "$GITHUB_PATH"
fi

echo "cargo shim: $shim_dir/cargo"
echo "rustup cargo: $real_cargo"
"$shim_dir/cargo" --version
