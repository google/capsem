#!/bin/bash
# Capsem Doctor -- macOS-specific checks
# Sourced by doctor-common.sh, do not run directly.

tool_hint() {
    case "$1" in
        rustup)        echo "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" ;;
        cargo)         echo "installed with rustup" ;;
        just)          echo "cargo install just" ;;
        node)          echo "brew install node" ;;
        pnpm)          echo "npm i -g pnpm" ;;
        python3)       echo "brew install python" ;;
        uv)            echo "curl -LsSf https://astral.sh/uv/install.sh | sh" ;;
        sqlite3)       echo "brew install sqlite" ;;
        git)           echo "brew install git" ;;
        b3sum)         echo "cargo install b3sum --locked" ;;
        docker)        echo "brew install colima docker && colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8" ;;
        docker-buildx) echo "brew install docker-buildx && ln -sf \$(brew --prefix docker-buildx)/bin/docker-buildx ~/.docker/cli-plugins/docker-buildx" ;;
    esac
}

check_platform() {
    section "Container Runtime (macOS)"

    # Colima
    if command -v colima &>/dev/null; then
        if colima status 2>&1 | grep -qi "running"; then
            pass "colima (running)"
        else
            fail "colima not running -- start: colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8"
        fi

        # Rosetta
        local colima_yaml="$HOME/.colima/default/colima.yaml"
        if [[ -f "$colima_yaml" ]]; then
            if grep -q 'rosetta: true' "$colima_yaml" && grep -q 'vmType: vz' "$colima_yaml"; then
                pass "colima rosetta (enabled, vz)"
            else
                fail "colima rosetta not enabled -- fix: colima stop && colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8"
            fi
        else
            fail "colima config not found at $colima_yaml"
        fi

        # Resources
        if command -v docker &>/dev/null; then
            local mem_mb cpus
            mem_mb=$(docker info --format json 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('MemTotal',0) // 1024 // 1024)" 2>/dev/null || echo 0)
            cpus=$(docker info --format json 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('NCPU',0))" 2>/dev/null || echo 0)
            if [[ "$mem_mb" -gt 0 ]]; then
                if [[ "$mem_mb" -lt 4096 ]]; then
                    fail "Colima: ${mem_mb}MB RAM, ${cpus} CPUs (minimum 4096MB)"
                elif [[ "$mem_mb" -lt 8192 ]]; then
                    pass "Colima: ${mem_mb}MB RAM, ${cpus} CPUs (8192MB recommended)"
                else
                    pass "Colima: ${mem_mb}MB RAM, ${cpus} CPUs"
                fi
            fi
        fi
    else
        fail "colima not found -- install: brew install colima"
    fi

    # Docker credential helper
    if [[ -f "$HOME/.docker/config.json" ]]; then
        local creds_store
        creds_store=$(python3 -c "import json; c=json.load(open('$HOME/.docker/config.json')); print(c.get('credsStore',''))" 2>/dev/null || echo "")
        if [[ -n "$creds_store" ]]; then
            local helper="docker-credential-$creds_store"
            if command -v "$helper" &>/dev/null; then
                pass "Docker credential helper ($helper)"
            else
                fail "Docker config references '$helper' but it is not installed -- set credsStore to \"\" in ~/.docker/config.json"
            fi
        fi
    fi

    # Codesigning
    section "Codesigning (macOS)"

    if xcode-select -p &>/dev/null; then
        pass "Xcode Command Line Tools ($(xcode-select -p))"
    else
        fixable "xcode-select --install" "Xcode Command Line Tools not installed"
    fi

    if command -v codesign &>/dev/null; then
        pass "codesign"
    else
        fail "codesign not found -- install Xcode Command Line Tools"
    fi

    if [[ -r "$ENTITLEMENTS" ]]; then
        pass "$ENTITLEMENTS"
    else
        fixable "git checkout $ENTITLEMENTS" "$ENTITLEMENTS missing"
    fi

    if [[ -f ".cargo/config.toml" ]] && grep -q 'runner.*run_signed' .cargo/config.toml; then
        pass ".cargo/config.toml (cargo runner)"
    else
        fixable "git checkout .cargo/config.toml" ".cargo/config.toml missing or misconfigured"
    fi

    if [[ -x "scripts/run_signed.sh" ]]; then
        pass "scripts/run_signed.sh"
    elif [[ -f "scripts/run_signed.sh" ]]; then
        fixable "chmod +x scripts/run_signed.sh" "scripts/run_signed.sh not executable"
    else
        fixable "git checkout scripts/run_signed.sh && chmod +x scripts/run_signed.sh" "scripts/run_signed.sh missing"
    fi

    # Test sign
    if command -v codesign &>/dev/null && [[ -r "$ENTITLEMENTS" ]]; then
        local sign_test
        sign_test=$(mktemp /tmp/capsem-sign-test.XXXXXX)
        if cc -x c -o "$sign_test" - <<< 'int main(){return 0;}' 2>/dev/null; then
            if codesign --sign - --entitlements "$ENTITLEMENTS" --force "$sign_test" 2>/dev/null; then
                pass "test sign (ad-hoc + entitlements)"
            else
                fail "test sign failed -- check SIP: csrutil status"
            fi
        else
            fail "test sign skipped -- cc failed -- reinstall: sudo rm -rf /Library/Developer/CommandLineTools && xcode-select --install"
        fi
        rm -f "$sign_test"
    fi
}
