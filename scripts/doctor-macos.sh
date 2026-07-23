#!/bin/bash
# Capsem Doctor -- macOS-specific checks
# Sourced by doctor-common.sh, do not run directly.

recommended_docker_disk_gib() {
    uv run python "$PROJECT_ROOT/scripts/docker-storage-policy.py" shell --rail default \
        | awk -F= '/CAPSEM_DOCKER_RECOMMENDED_DISK_GIB/ { print $2 }'
}

minimum_docker_disk_gib() {
    uv run python "$PROJECT_ROOT/scripts/docker-storage-policy.py" shell --rail default \
        | awk -F= '/CAPSEM_DOCKER_MINIMUM_DISK_GIB/ { print $2 }'
}

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
        flock)         echo "brew install flock (multi-agent lock on ~/.capsem/run/execution.lock)" ;;
        zstd)          echo "brew install zstd" ;;
        tart)          echo "brew trust --formula cirruslabs/cli/softnet && brew install cirruslabs/cli/tart" ;;
        sshpass)       echo "brew install cirruslabs/cli/sshpass" ;;
        docker)        echo "brew install colima docker (CLI + Colima backend) && colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8 --disk $(recommended_docker_disk_gib)" ;;
        docker-daemon) echo "start Colima: colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8 --disk $(recommended_docker_disk_gib)" ;;
        docker-buildx) echo "brew install docker-buildx && ln -sf \$(brew --prefix docker-buildx)/bin/docker-buildx ~/.docker/cli-plugins/docker-buildx" ;;
    esac
}

check_platform() {
    section "Tart macOS Install VM"

    if [[ "$(uname -m)" = "arm64" ]]; then
        pass "Apple Silicon host (Tart supported)"
    else
        fail "Tart requires an Apple Silicon macOS host"
    fi
    if command -v tart &>/dev/null; then
        local tart_version
        tart_version=$(tart --version 2>&1 || true)
        if [[ -n "$tart_version" ]]; then
            pass "tart ($tart_version)"
        else
            fail "tart --version failed -- reinstall: $(tool_hint tart)"
        fi
    else
        fail "tart not found -- install: $(tool_hint tart)"
    fi
    if command -v sshpass &>/dev/null; then
        pass "sshpass (Tart noninteractive SSH)"
    else
        fail "sshpass not found -- install: $(tool_hint sshpass)"
    fi
    if command -v tart &>/dev/null; then
        local tart_snapshot
        tart_snapshot=$(uv run python "$PROJECT_ROOT/scripts/docker-storage-policy.py" \
            tart-snapshot --label doctor 2>&1 || true)
        if printf '%s\n' "$tart_snapshot" | grep -q "retain-base-image-cache"; then
            pass "Tart base image cache present"
        else
            pass "Tart base image cache not present (first glow-up will pull it)"
        fi
        if printf '%s\n' "$tart_snapshot" | grep -q "delete-owned-working-vm"; then
            fail "stale Capsem-owned Tart VM found -- run: uv run python scripts/docker-storage-policy.py tart-clean --label doctor"
        else
            pass "no leaked Capsem-owned Tart VMs"
        fi
    fi

    section "Container Runtime (macOS)"

    # Colima
    if command -v colima &>/dev/null; then
        local colima_status
        colima_status=$(colima status 2>&1 || true)
        if printf '%s\n' "$colima_status" | grep -qi "running"; then
            pass "colima (running)"
        else
            fail "colima not running -- start: colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8"
        fi

        # Rosetta. The config bit is not sufficient: an already-running Colima
        # VM can predate that setting and have no registered amd64 emulator.
        # This exact stale-runtime state makes Docker fail later with
        # `exec /bin/sh: exec format error`, so require the live binfmt rail.
        local colima_yaml="$HOME/.colima/default/colima.yaml"
        if [[ -f "$colima_yaml" ]]; then
            if grep -q 'rosetta: true' "$colima_yaml" && grep -q 'vmType: vz' "$colima_yaml"; then
                if colima ssh -- test -f /proc/sys/fs/binfmt_misc/rosetta &>/dev/null; then
                    pass "colima rosetta (enabled, registered, vz)"
                else
                    fail "colima rosetta configured but not registered -- fix: colima restart"
                fi
            else
                fail "colima rosetta not enabled -- fix: colima stop && colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8"
            fi
        else
            fail "colima config not found at $colima_yaml"
        fi

        # Resources
        if command -v docker &>/dev/null; then
            local mem_mb cpus disk_total_kib disk_total_gib minimum_disk_gib recommended_disk_gib
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
            minimum_disk_gib=$(minimum_docker_disk_gib)
            recommended_disk_gib=$(recommended_docker_disk_gib)
            disk_total_kib=$(colima ssh -- sh -c "df -Pk /var/lib/docker | awk 'NR == 2 { print \$2 }'" 2>/dev/null || echo 0)
            if [[ "$disk_total_kib" =~ ^[0-9]+$ ]] && [[ "$disk_total_kib" -gt 0 ]]; then
                disk_total_gib=$((disk_total_kib / 1024 / 1024))
                if [[ "$disk_total_gib" -lt "$minimum_disk_gib" ]]; then
                    fail "Colima Docker disk: ${disk_total_gib}GiB (minimum ${minimum_disk_gib}GiB) -- expand: colima stop && colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8 --disk ${recommended_disk_gib}"
                elif [[ "$disk_total_gib" -lt "$recommended_disk_gib" ]]; then
                    pass "Colima Docker disk: ${disk_total_gib}GiB (supported; ${recommended_disk_gib}GiB recommended for new runtimes)"
                else
                    pass "Colima Docker disk: ${disk_total_gib}GiB"
                fi
            else
                fail "could not measure Colima Docker disk capacity"
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
        fail "Xcode Command Line Tools not installed -- run: xcode-select --install"
    fi

    if command -v codesign &>/dev/null; then
        pass "codesign"
    else
        fail "codesign not found -- install Xcode Command Line Tools"
    fi

    if [[ -r "$ENTITLEMENTS" ]]; then
        pass "$ENTITLEMENTS"
    else
        fixable entitlements "$ENTITLEMENTS missing"
    fi

    if [[ -f ".cargo/config.toml" ]] && grep -q 'runner.*run_signed' .cargo/config.toml; then
        pass ".cargo/config.toml (cargo runner)"
    else
        fixable cargo-config ".cargo/config.toml missing or misconfigured"
    fi

    if [[ -x "scripts/run_signed.sh" ]]; then
        pass "scripts/run_signed.sh"
    elif [[ -f "scripts/run_signed.sh" ]]; then
        fixable run-signed-chmod "scripts/run_signed.sh not executable"
    else
        fixable run-signed "scripts/run_signed.sh missing"
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
