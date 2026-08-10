#!/bin/sh
# Rust setup helpers shared by bootstrap and doctor. Sourcing this file is
# inert: it only defines functions, so the checked-in pin parser can be tested
# without downloading or changing a developer's toolchain.

capsem_rust_toolchain() {
    CAPSEM_RUST_PIN_FILE=$1
    awk -F= '
        /^[[:space:]]*\[toolchain\][[:space:]]*$/ {
            in_toolchain = 1
            next
        }
        /^[[:space:]]*\[/ {
            in_toolchain = 0
        }
        in_toolchain && $1 ~ /^[[:space:]]*channel[[:space:]]*$/ {
            value = $2
            sub(/[[:space:]]*#.*/, "", value)
            gsub(/^[[:space:]]*"|"[[:space:]]*$/, "", value)
            if (value !~ /^[0-9]+[.][0-9]+[.][0-9]+$/) {
                printf "invalid pinned Rust channel in %s: %s\n", FILENAME, value > "/dev/stderr"
                invalid = 1
                next
            }
            pin = value
            count++
        }
        END {
            if (invalid || count != 1) {
                if (!invalid) {
                    printf "expected one [toolchain] channel in %s\n", FILENAME > "/dev/stderr"
                }
                exit 2
            }
            print pin
        }
    ' "$CAPSEM_RUST_PIN_FILE"
}

capsem_gate_cargo_tools() {
    CAPSEM_GATE_CONFIG=$1
    awk -F= '
        /^[[:space:]]*\[\[toolchain[.]crates\]\][[:space:]]*$/ {
            in_crate = 1
            next
        }
        /^[[:space:]]*\[/ {
            in_crate = 0
        }
        in_crate && $1 ~ /^[[:space:]]*name[[:space:]]*$/ {
            value = $2
            sub(/[[:space:]]*#.*/, "", value)
            gsub(/^[[:space:]]*"|"[[:space:]]*$/, "", value)
            if (value !~ /^[A-Za-z0-9_-]+$/ || seen[value]) {
                printf "invalid or duplicate toolchain crate name in %s: %s\n", FILENAME, value > "/dev/stderr"
                invalid = 1
                next
            }
            seen[value] = 1
            names[++count] = value
        }
        END {
            if (invalid || count == 0) {
                if (!invalid) {
                    printf "no [[toolchain.crates]] names in %s\n", FILENAME > "/dev/stderr"
                }
                exit 2
            }
            for (position = 1; position <= count; position++) {
                print names[position]
            }
        }
    ' "$CAPSEM_GATE_CONFIG"
}

capsem_ensure_rust_toolchain() {
    CAPSEM_RUST_TOOLCHAIN=$1
    if ! rustup run "$CAPSEM_RUST_TOOLCHAIN" rustc --version >/dev/null 2>&1; then
        rustup toolchain install "$CAPSEM_RUST_TOOLCHAIN" --profile minimal
    fi

    CAPSEM_RUSTC_VERSION=$(rustup run "$CAPSEM_RUST_TOOLCHAIN" rustc --version)
    case "$CAPSEM_RUSTC_VERSION" in
        "rustc $CAPSEM_RUST_TOOLCHAIN "*) ;;
        *)
            printf "  [FAIL] pinned Rust %s did not provide the expected compiler: %s\n" \
                "$CAPSEM_RUST_TOOLCHAIN" "$CAPSEM_RUSTC_VERSION" >&2
            return 1 ;;
    esac
    printf "  [ok]   Rust %s (checked-in toolchain)\n" "$CAPSEM_RUST_TOOLCHAIN"
}

_capsem_rustup_bin_dir() {
    CAPSEM_RUSTUP_BIN=$(command -v rustup)
    if [ "$(uname -s)" = "Linux" ]; then
        CAPSEM_RUSTUP_BIN=$(readlink -f "$CAPSEM_RUSTUP_BIN")
    fi
    dirname "$CAPSEM_RUSTUP_BIN"
}

_capsem_cargo_bin_dir() {
    printf '%s\n' "${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}/bin"
}

_capsem_expose_managed_tools() {
    CAPSEM_RUSTUP_BIN_DIR=$1
    shift
    mkdir -p "$HOME/.local/bin"

    for CAPSEM_RUST_TOOL in "$@"; do
        CAPSEM_RUST_SOURCE="$CAPSEM_RUSTUP_BIN_DIR/$CAPSEM_RUST_TOOL"
        CAPSEM_RUST_LINK="$HOME/.local/bin/$CAPSEM_RUST_TOOL"
        if [ ! -f "$CAPSEM_RUST_SOURCE" ] || [ ! -x "$CAPSEM_RUST_SOURCE" ]; then
            printf "  [FAIL] managed Rust tool missing or not executable: %s\n" \
                "$CAPSEM_RUST_SOURCE" >&2
            return 1
        fi
        if [ -L "$CAPSEM_RUST_LINK" ]; then
            case "$(readlink "$CAPSEM_RUST_LINK")" in
                "$CAPSEM_RUSTUP_BIN_DIR"/*) ;;
                *)
                    printf "  [FAIL] refusing to replace unmanaged symlink %s\n" \
                        "$CAPSEM_RUST_LINK" >&2
                    return 1 ;;
            esac
        elif [ -e "$CAPSEM_RUST_LINK" ]; then
            printf "  [FAIL] refusing to replace unmanaged file %s\n" \
                "$CAPSEM_RUST_LINK" >&2
            return 1
        fi
        ln -sfn "$CAPSEM_RUST_SOURCE" "$CAPSEM_RUST_LINK"
    done
    hash -r 2>/dev/null || true
}

capsem_expose_rustup_tools() {
    CAPSEM_RUSTUP_BIN_DIR=$(_capsem_rustup_bin_dir)

    # Bootstrap itself prepends ~/.cargo/bin, but a child script cannot change
    # the invoking agent's PATH. ~/.local/bin is already Capsem's user-level
    # tool home, so expose the Rustup proxies there for the immediately
    # following private-copy gate and for later shells.
    _capsem_expose_managed_tools "$CAPSEM_RUSTUP_BIN_DIR" rustup rustc cargo
}

capsem_expose_gate_cargo_tools() {
    CAPSEM_GATE_CONFIG=$1
    CAPSEM_CARGO_BIN_DIR=$(_capsem_cargo_bin_dir)
    CAPSEM_GATE_CARGO_TOOL_LIST=$(capsem_gate_cargo_tools "$CAPSEM_GATE_CONFIG")
    # Names are restricted to shell-safe words by capsem_gate_cargo_tools.
    # shellcheck disable=SC2086
    _capsem_expose_managed_tools \
        "$CAPSEM_CARGO_BIN_DIR" $CAPSEM_GATE_CARGO_TOOL_LIST
}
