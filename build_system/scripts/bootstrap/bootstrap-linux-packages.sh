#!/bin/sh
# Linux bootstrap package and Node provisioning helpers.

capsem_linux_install_apt_packages() {
    CAPSEM_APT_PROJECT_ROOT=$1
    CAPSEM_APT_ASSUME_YES=$2
    CAPSEM_APT_BINFMT_PACKAGE=$(capsem_linux_apt_binfmt_package)
    CAPSEM_APT_WORKSPACE_PACKAGES=$(python3 \
        "$CAPSEM_APT_PROJECT_ROOT/build_system/scripts/bootstrap/provision-linux-workspace.py" --packages apt)
    CAPSEM_APT_DOCKER_PACKAGES=$("$CAPSEM_APT_PROJECT_ROOT/build_system/scripts/build/select-docker-packages.sh")
    CAPSEM_APT_BASE_PACKAGES="
        acl
        ca-certificates
        cpio
        python3
        python3-venv
        sqlite3
        util-linux
        xvfb
        xz-utils
        zstd
        $CAPSEM_APT_WORKSPACE_PACKAGES
        $CAPSEM_APT_BINFMT_PACKAGE
        $CAPSEM_APT_DOCKER_PACKAGES
    "

    CAPSEM_APT_NEEDS_INSTALL=0
    for CAPSEM_APT_PACKAGE in $CAPSEM_APT_BASE_PACKAGES; do
        if ! dpkg-query -W -f='${Status}' "$CAPSEM_APT_PACKAGE" 2>/dev/null \
            | grep -q '^install ok installed$'; then
            CAPSEM_APT_NEEDS_INSTALL=1
            break
        fi
    done
    if [ "$CAPSEM_APT_NEEDS_INSTALL" -eq 0 ]; then
        python3 "$CAPSEM_APT_PROJECT_ROOT/build_system/scripts/bootstrap/provision-linux-workspace.py" --verify
        printf "  [ok]   Linux system packages\n"
        return 0
    fi

    if ! capsem_linux_confirm "required Linux system packages" "$CAPSEM_APT_ASSUME_YES"; then
        printf "  [FAIL] required Linux system packages were declined\n" >&2
        return 1
    fi

    # Bounded, retried, and over IPv4. On a hosted runner `apt-get update`
    # reached `archive.ubuntu.com` and never returned: two release attempts
    # sat there until the job's 120-minute timeout killed them, having
    # published nothing, and the log's last line both times was the same
    # `InRelease` fetch. The runner advertises IPv6 that does not route to
    # that host, and apt's default is to wait rather than fall back.
    #
    # The timeout is the part that matters most. Whatever the cause next time,
    # a package fetch that stalls should cost minutes and say so, not two
    # hours of silence followed by a cancelled release.
    capsem_linux_apt() {
        capsem_linux_as_root env DEBIAN_FRONTEND=noninteractive timeout 600 apt-get \
            -o Acquire::ForceIPv4=true \
            -o Acquire::Retries=3 \
            -o Acquire::http::Timeout=30 \
            -o Acquire::https::Timeout=30 \
            "$@"
    }
    capsem_linux_apt update
    capsem_linux_apt install -y \
        --no-install-recommends \
        $CAPSEM_APT_BASE_PACKAGES
    python3 "$CAPSEM_APT_PROJECT_ROOT/build_system/scripts/bootstrap/provision-linux-workspace.py" --verify
    printf "  [ok]   Linux system packages installed\n"
}
capsem_linux_install_dnf_packages() {
    CAPSEM_DNF_PROJECT_ROOT=$1
    CAPSEM_DNF_ASSUME_YES=$2
    CAPSEM_DNF_BINFMT_PACKAGE=$(capsem_linux_dnf_binfmt_package)
    CAPSEM_DNF_WORKSPACE_PACKAGES=$(python3 \
        "$CAPSEM_DNF_PROJECT_ROOT/build_system/scripts/bootstrap/provision-linux-workspace.py" --packages dnf)
    CAPSEM_DNF_PACKAGES="
        acl
        cpio
        docker
        docker-buildx-plugin
        python3
        sqlite
        util-linux
        xz
        zstd
        $CAPSEM_DNF_WORKSPACE_PACKAGES
        $CAPSEM_DNF_BINFMT_PACKAGE
    "

    CAPSEM_DNF_NEEDS_INSTALL=0
    for CAPSEM_DNF_PACKAGE in $CAPSEM_DNF_PACKAGES; do
        if ! rpm -q "$CAPSEM_DNF_PACKAGE" >/dev/null 2>&1; then
            CAPSEM_DNF_NEEDS_INSTALL=1
            break
        fi
    done
    if [ "$CAPSEM_DNF_NEEDS_INSTALL" -eq 0 ]; then
        python3 "$CAPSEM_DNF_PROJECT_ROOT/build_system/scripts/bootstrap/provision-linux-workspace.py" --verify
        printf "  [ok]   Linux system packages\n"
        return 0
    fi
    if ! capsem_linux_confirm "required Linux system packages" "$CAPSEM_DNF_ASSUME_YES"; then
        printf "  [FAIL] required Linux system packages were declined\n" >&2
        return 1
    fi
    capsem_linux_as_root dnf install -y $CAPSEM_DNF_PACKAGES
    python3 "$CAPSEM_DNF_PROJECT_ROOT/build_system/scripts/bootstrap/provision-linux-workspace.py" --verify
    printf "  [ok]   Linux system packages installed\n"
}

capsem_linux_install_node() {
    CAPSEM_NODE_PROJECT_ROOT=$1
    CAPSEM_NODE_ASSUME_YES=$2
    CAPSEM_NODE_MAJOR=$(capsem_linux_node_major \
        "$CAPSEM_NODE_PROJECT_ROOT/config/docker/image/build.toml")
    CAPSEM_NODE_INSTALL_ROOT="$HOME/.local/share/capsem/node"
    CAPSEM_NODE_COMMAND=$(command -v node 2>/dev/null || true)
    CAPSEM_NODE_CURRENT_MAJOR=""
    if [ -n "$CAPSEM_NODE_COMMAND" ]; then
        CAPSEM_NODE_CURRENT_MAJOR=$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || true)
    fi
    CAPSEM_NODE_IS_MANAGED=0
    if [ "$CAPSEM_NODE_COMMAND" = "$HOME/.local/bin/node" ] \
        && [ -L "$HOME/.local/bin/node" ]; then
        case "$(readlink "$HOME/.local/bin/node")" in
            "$CAPSEM_NODE_INSTALL_ROOT"/*) CAPSEM_NODE_IS_MANAGED=1 ;;
        esac
    fi
    if [ "$CAPSEM_NODE_CURRENT_MAJOR" = "$CAPSEM_NODE_MAJOR" ] \
        && [ "$CAPSEM_NODE_IS_MANAGED" -eq 1 ]; then
        printf "  [ok]   Node.js %s (required major %s)\n" \
            "$(node --version)" "$CAPSEM_NODE_MAJOR"
        return 0
    fi

    if ! capsem_linux_confirm \
        "Node.js $CAPSEM_NODE_MAJOR (verified official Linux archive)" \
        "$CAPSEM_NODE_ASSUME_YES"; then
        printf "  [FAIL] Node.js major %s is required; found %s\n" \
            "$CAPSEM_NODE_MAJOR" "${CAPSEM_NODE_CURRENT_MAJOR:-none}" >&2
        return 1
    fi

    case "$(uname -m)" in
        x86_64|amd64) CAPSEM_NODE_ARCH=x64 ;;
        aarch64|arm64) CAPSEM_NODE_ARCH=arm64 ;;
        *)
            printf "  [FAIL] unsupported Linux architecture for Node.js: %s\n" "$(uname -m)" >&2
            return 1 ;;
    esac

    CAPSEM_NODE_TMPDIR=$(mktemp -d "${TMPDIR:-/tmp}/capsem-node.XXXXXX")
    CAPSEM_NODE_SUMS="$CAPSEM_NODE_TMPDIR/SHASUMS256.txt"
    curl --proto '=https' --tlsv1.2 -fsSL \
        "https://nodejs.org/dist/latest-v${CAPSEM_NODE_MAJOR}.x/SHASUMS256.txt" \
        -o "$CAPSEM_NODE_SUMS"
    CAPSEM_NODE_ARCHIVE_NAME=$(awk -v arch="$CAPSEM_NODE_ARCH" '
        $2 ~ ("^node-v[0-9.]+-linux-" arch "[.]tar[.]xz$") { print $2; exit }
    ' "$CAPSEM_NODE_SUMS")
    if [ -z "$CAPSEM_NODE_ARCHIVE_NAME" ]; then
        printf "  [FAIL] Node.js checksum manifest has no Linux %s archive\n" \
            "$CAPSEM_NODE_ARCH" >&2
        rm -f "$CAPSEM_NODE_SUMS"
        rmdir "$CAPSEM_NODE_TMPDIR"
        return 1
    fi

    CAPSEM_NODE_ARCHIVE="$CAPSEM_NODE_TMPDIR/$CAPSEM_NODE_ARCHIVE_NAME"
    curl --proto '=https' --tlsv1.2 -fsSL \
        "https://nodejs.org/dist/latest-v${CAPSEM_NODE_MAJOR}.x/$CAPSEM_NODE_ARCHIVE_NAME" \
        -o "$CAPSEM_NODE_ARCHIVE"
    CAPSEM_NODE_EXPECTED_SHA=$(awk -v archive="$CAPSEM_NODE_ARCHIVE_NAME" \
        '$2 == archive { print $1; exit }' "$CAPSEM_NODE_SUMS")
    CAPSEM_NODE_ACTUAL_SHA=$(sha256sum "$CAPSEM_NODE_ARCHIVE" | awk '{print $1}')
    if [ -z "$CAPSEM_NODE_EXPECTED_SHA" ] \
        || [ "$CAPSEM_NODE_ACTUAL_SHA" != "$CAPSEM_NODE_EXPECTED_SHA" ]; then
        printf "  [FAIL] Node.js archive SHA256 does not match the official manifest\n" >&2
        rm -f "$CAPSEM_NODE_ARCHIVE" "$CAPSEM_NODE_SUMS"
        rmdir "$CAPSEM_NODE_TMPDIR"
        return 1
    fi

    CAPSEM_NODE_TREE=${CAPSEM_NODE_ARCHIVE_NAME%.tar.xz}
    mkdir -p "$CAPSEM_NODE_INSTALL_ROOT" "$HOME/.local/bin"
    tar -xJf "$CAPSEM_NODE_ARCHIVE" -C "$CAPSEM_NODE_INSTALL_ROOT"
    CAPSEM_NODE_BIN="$CAPSEM_NODE_INSTALL_ROOT/$CAPSEM_NODE_TREE/bin"
    for CAPSEM_NODE_TOOL in node npm npx corepack; do
        [ -e "$CAPSEM_NODE_BIN/$CAPSEM_NODE_TOOL" ] || continue
        CAPSEM_NODE_LINK="$HOME/.local/bin/$CAPSEM_NODE_TOOL"
        if [ -L "$CAPSEM_NODE_LINK" ]; then
            case "$(readlink "$CAPSEM_NODE_LINK")" in
                "$CAPSEM_NODE_INSTALL_ROOT"/*) ;;
                *)
                    printf "  [FAIL] refusing to replace unmanaged symlink %s\n" \
                        "$CAPSEM_NODE_LINK" >&2
                    return 1 ;;
            esac
        elif [ -e "$CAPSEM_NODE_LINK" ]; then
            printf "  [FAIL] refusing to replace unmanaged file %s\n" \
                "$CAPSEM_NODE_LINK" >&2
            return 1
        fi
        ln -sfn "$CAPSEM_NODE_BIN/$CAPSEM_NODE_TOOL" "$CAPSEM_NODE_LINK"
    done
    rm -f "$CAPSEM_NODE_ARCHIVE" "$CAPSEM_NODE_SUMS"
    rmdir "$CAPSEM_NODE_TMPDIR"
    hash -r 2>/dev/null || true

    CAPSEM_NODE_INSTALLED_MAJOR=$(node -p 'process.versions.node.split(".")[0]')
    if [ "$CAPSEM_NODE_INSTALLED_MAJOR" != "$CAPSEM_NODE_MAJOR" ]; then
        printf "  [FAIL] installed Node.js major %s, expected %s\n" \
            "$CAPSEM_NODE_INSTALLED_MAJOR" "$CAPSEM_NODE_MAJOR" >&2
        return 1
    fi
    printf "  [ok]   Node.js %s installed from verified official archive\n" \
        "$(node --version)"
}
