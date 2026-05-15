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
real_rustc="$(rustup which --toolchain "$toolchain" rustc)"
real_rustdoc="$(rustup which --toolchain "$toolchain" rustdoc)"

shim_dir="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/capsem-cargo-bin"
mkdir -p "$shim_dir"
for tool in cargo rustc rustdoc; do
    cat > "$shim_dir/$tool" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
toolchain="${RUSTUP_TOOLCHAIN:-stable}"
tool="$(basename "$0")"
exec rustup run "$toolchain" "$tool" "$@"
EOF
    chmod +x "$shim_dir/$tool"
done

if [[ -n "${GITHUB_PATH:-}" ]]; then
    echo "$shim_dir" >> "$GITHUB_PATH"
fi

echo "cargo shim: $shim_dir/cargo"
echo "rustup cargo: $real_cargo"
echo "rustup rustc: $real_rustc"
echo "rustup rustdoc: $real_rustdoc"
"$shim_dir/cargo" --version
"$shim_dir/rustc" -vV | sed -n '1,4p'
