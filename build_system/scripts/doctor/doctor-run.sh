# Cross-platform checks and final verdict for doctor-common.sh.
# This fragment is sourced after doctor-common initializes the counters.
# shellcheck disable=SC2154

echo -e "${BOLD}Capsem Doctor${NC}"
echo "============================================"

section "System Tools"
for tool in cargo rustup node python3 uv pnpm sqlite3 git b3sum zstd cpio; do
    if command -v "$tool" &>/dev/null; then
        pass "$tool"
    else
        _hint=$(tool_hint "$tool")
        fail "$tool not found -- install: $_hint"
    fi
done

if command -v node &>/dev/null; then
    _required_node_major=$(capsem_linux_node_major \
        "$PROJECT_ROOT/config/docker/image/build.toml")
    _installed_node_major=$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || true)
    if [[ "$_installed_node_major" =~ ^[0-9]+$ ]] \
        && (( _installed_node_major >= _required_node_major )); then
        pass "Node.js major $_installed_node_major (required Node.js major $_required_node_major or newer)"
    else
        fail "Node.js major ${_installed_node_major:-unknown} is below required Node.js major $_required_node_major -- run bootstrap.sh"
    fi
fi

section "Rust Toolchain"
if _doctor_rust_version=$(rustup run "$CAPSEM_RUST_TOOLCHAIN" rustc --version 2>/dev/null) \
    && [[ "$_doctor_rust_version" == "rustc $CAPSEM_RUST_TOOLCHAIN "* ]]; then
    pass "Rust $CAPSEM_RUST_TOOLCHAIN (checked-in toolchain)"
else
    fail "Rust $CAPSEM_RUST_TOOLCHAIN missing or unusable -- run ./bootstrap.sh"
fi
for target in $(capsem_rust_targets "$PROJECT_ROOT/config/gate.toml"); do
    if rustup target list --toolchain "$CAPSEM_RUST_TOOLCHAIN" --installed \
        2>/dev/null | grep -q "$target"; then
        pass "target: $target"
    else
        fixable rustup-targets "target: $target missing"
    fi
done
if rustup component list --toolchain "$CAPSEM_RUST_TOOLCHAIN" --installed \
    2>/dev/null | grep -q llvm-tools; then
    pass "component: llvm-tools"
else
    fixable llvm-tools "component: llvm-tools missing"
fi

if declare -F check_linux_musl_toolchain >/dev/null; then
    check_linux_musl_toolchain
fi

section "Cargo Tools"
_check_cargo_tool() {
    local tool="$1" expected="$2" probe="$3" actual first_line
    local -a probe_argv
    if ! command -v "$tool" &>/dev/null; then
        fixable gate-cargo-tools "$tool not found"
        return
    fi
    read -r -a probe_argv <<< "$probe"
    actual=$("${probe_argv[@]}" 2>/dev/null || true)
    first_line=${actual%%$'\n'*}
    if [[ "$actual" == "$expected"* ]]; then
        pass "$tool ($expected)"
    else
        fixable gate-cargo-tools \
            "$tool version mismatch (expected $expected, found ${first_line:-no output})"
    fi
}
while IFS=$'\t' read -r tool expected probe; do
    _check_cargo_tool "$tool" "$expected" "$probe"
done <<EOF
$(capsem_gate_cargo_tool_versions "$PROJECT_ROOT/config/gate.toml")
EOF

section "Container Tools"
if command -v docker &>/dev/null; then
    pass "docker CLI ($(docker --version 2>/dev/null | head -1))"
else
    _hint=$(tool_hint docker)
    fail "docker CLI not found -- install: $_hint"
fi

if docker info &>/dev/null; then
    pass "docker daemon (running)"
else
    _hint=$(tool_hint docker-daemon)
    fail "docker daemon not reachable -- $_hint"
fi

if docker buildx version &>/dev/null; then
    pass "docker buildx ($(docker buildx version 2>/dev/null | head -1))"
else
    _hint=$(tool_hint docker-buildx)
    fail "docker buildx not working -- install: $_hint"
fi

# Platform-specific checks (colima, codesigning, KVM, etc.)
check_platform

section "VM Assets"
if [[ -z "${CAPSEM_SKIP_ASSET_CHECK:-}" ]]; then
    if [[ -f "$ASSETS_DIR/manifest.json" ]]; then
        _cargo_ver=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
        # v2 manifest: binaries.current holds the binary release that matches Cargo.toml.
        _manifest_ver=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("binaries",{}).get("current",""))' "$ASSETS_DIR/manifest.json" 2>/dev/null)
        if [[ "$_cargo_ver" == "$_manifest_ver" ]]; then
            pass "binary version ($_manifest_ver) matches Cargo.toml"
        else
            fixable build-assets "binary version mismatch: Cargo=$_cargo_ver, manifest.binaries.current=$_manifest_ver"
        fi

        if command -v b3sum &>/dev/null && [[ -f "$ASSETS_DIR/B3SUMS" ]]; then
            if (cd "$ASSETS_DIR" && b3sum --check B3SUMS >/dev/null 2>&1); then
                pass "asset integrity (B3SUMS match)"
            else
                fixable build-assets "asset integrity check failed"
            fi
        fi
    else
        fixable build-assets "manifest.json missing"
    fi
else
    skip "VM Assets (CAPSEM_SKIP_ASSET_CHECK set)"
fi

section "Guest Binaries"
if [[ -z "${CAPSEM_SKIP_ASSET_CHECK:-}" ]]; then
    arch=$(uname -m | sed 's/aarch64/arm64/')
    release_dir="target/linux-agent/$arch"
    for b in capsem-pty-agent capsem-net-proxy capsem-mcp-server; do
        if [[ -f "$release_dir/$b" ]]; then
            if file "$release_dir/$b" 2>/dev/null | grep -E -q "ELF 64-bit"; then
                pass "$b (Linux ELF)"
            else
                fixable pack-initrd "$b found but not Linux ELF"
            fi
        else
            fixable pack-initrd "$b missing"
        fi
    done
else
    skip "Guest Binaries (CAPSEM_SKIP_ASSET_CHECK set)"
fi

section "Release Tools"
for tool in gh openssl; do
    if command -v "$tool" &>/dev/null; then
        pass "$tool"
    else
        skip "$tool (only needed for releases)"
    fi
done

# Close final category and show recap.
if [[ -n "$_cur_cat" ]]; then
    CAT_NAMES+=("$_cur_cat")
    CAT_PASS+=("$_cur_pass")
    CAT_FAIL+=("$_cur_fail")
fi

echo ""
echo "============================================"
echo -e "${BOLD}  Capsem Doctor Results${NC}"
echo "============================================"
for i in "${!CAT_NAMES[@]}"; do
    _p="${CAT_PASS[$i]}"
    _f="${CAT_FAIL[$i]}"
    _total=$(( _p + _f ))
    if [[ "$_f" -eq 0 ]]; then
        _status="${GREEN}${_p}/${_total}${NC}"
    else
        _status="${RED}${_p}/${_total}${NC}"
    fi
    printf "  %-22s %b passed\n" "${CAT_NAMES[$i]}" "$_status"
done
echo "--------------------------------------------"
if [[ "$FAIL" -eq 0 ]]; then
    echo -e "  ${GREEN}${BOLD}$PASS passed${NC}, $SKIP skipped, $WARN warnings"
else
    echo -e "  ${GREEN}$PASS passed${NC}, ${RED}${BOLD}$FAIL failed${NC}, $SKIP skipped, $WARN warnings"
fi
echo "============================================"

# Collect needed fixes in registry order, then run them when requested.
_needed_count=0
for i in "${!FIX_IDS[@]}"; do
    if [[ "${FIX_NEEDED[$i]}" -eq 1 ]]; then
        _needed_count=$((_needed_count + 1))
    fi
done

if [[ "$_needed_count" -gt 0 ]]; then
    echo ""
    echo -e "${BOLD}${_needed_count} fix(es) available (in dependency order):${NC}"
    _n=1
    for i in "${!FIX_IDS[@]}"; do
        if [[ "${FIX_NEEDED[$i]}" -eq 1 ]]; then
            echo -e "  ${DIM}${_n}.${NC} ${FIX_DESCS[$i]} ${DIM}(${FIX_CMDS[$i]})${NC}"
            _n=$((_n + 1))
        fi
    done

    if [[ "${1:-}" == "--fix" ]]; then
        echo ""
        for i in "${!FIX_IDS[@]}"; do
            if [[ "${FIX_NEEDED[$i]}" -eq 1 ]]; then
                echo -e "${BOLD}Fixing:${NC} ${FIX_DESCS[$i]}"
                echo -e "  ${DIM}\$ ${FIX_CMDS[$i]}${NC}"
                if eval "${FIX_CMDS[$i]}"; then
                    echo -e "  ${GREEN}done${NC}"
                else
                    echo -e "  ${RED}failed -- stopping (later fixes depend on this)${NC}"
                    exit 1
                fi
                echo ""
            fi
        done
        echo -e "${BOLD}Re-running doctor to verify...${NC}"
        echo ""
        exec "$0"
    else
        echo ""
        echo -e "Run ${BOLD}just doctor-fix${NC} to auto-fix these issues."
    fi
fi

if [[ "$FAIL" -gt 0 ]]; then
    exit 1
fi
touch .dev-setup
