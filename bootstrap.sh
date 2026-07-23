#!/bin/sh
# Capsem developer bootstrap -- installs deps, runs doctor with auto-fix.
# Only prerequisite: sh, git, curl.
# Usage: sh bootstrap.sh [-y|--yes]
#   -y, --yes   Non-interactive: assume "yes" to every install prompt.
#               Use in CI / unattended setup.
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DOCKER_DISK_GIB=$(awk -F= '
    /^\[docker\]$/ { in_docker=1; next }
    /^\[/ { in_docker=0 }
    in_docker && $1 ~ /^[[:space:]]*recommended_disk_gib[[:space:]]*$/ {
        gsub(/[[:space:]]/, "", $2); print $2; exit
    }
' "$SCRIPT_DIR/config/storage-policy.toml")
case "$DOCKER_DISK_GIB" in
    ''|*[!0-9]*)
        printf "invalid docker.recommended_disk_gib in config/storage-policy.toml\n" >&2
        exit 2
        ;;
esac

ASSUME_YES=0
for arg in "$@"; do
    case "$arg" in
        -y|--yes) ASSUME_YES=1 ;;
        -h|--help)
            sed -n '2,6p' "$0" | sed 's/^# \?//'
            exit 0 ;;
        *) printf "unknown arg: %s\n" "$arg" >&2; exit 2 ;;
    esac
done

check_bootstrap_shape() {
    cd "$SCRIPT_DIR"
    for link in .agents/skills .claude/skills .codex/skills .cursor/skills .gemini/skills; do
        [ "$(readlink "$link" 2>/dev/null || true)" = "../skills" ] || {
            printf "  [FAIL] %s must be a symlink to ../skills\n" "$link" >&2
            exit 1
        }
    done
    for file in \
        skills/dev-sprint/SKILL.md \
        skills/dev-testing/SKILL.md \
        skills/dev-capsem/SKILL.md \
        skills/ironbank/SKILL.md \
        skills/frontend-design/SKILL.md \
        site/package.json \
        site/astro.config.mjs \
        site/src/components/FAQ.svelte \
        site/src/lib/data.ts; do
        [ -f "$file" ] || { printf "  [FAIL] missing %s\n" "$file" >&2; exit 1; }
    done
    SKILL_COUNT=$(find skills -mindepth 2 -name SKILL.md | wc -l | tr -d ' ')
    [ "$SKILL_COUNT" -ge 25 ] || { printf "  [FAIL] expected at least 25 project skills, found %s\n" "$SKILL_COUNT" >&2; exit 1; }
    printf "  [ok]   project skills symlinks, key skills, and site surface\n"
}

check_bootstrap_shape

# Ask the developer "Install <tool>? [Y/n]". Returns 0 on yes, 1 on no.
# Default is YES (just press enter). Auto-yes when -y is set; auto-yes when
# stdin isn't a tty either (CI/pipelines should bootstrap fully -- pass an
# explicit "n" answer or skip the call site if you don't want the install).
confirm() {
    if [ "$ASSUME_YES" = 1 ] || [ ! -t 0 ]; then
        return 0
    fi
    printf "  Install %s? [Y/n] " "$1"
    read -r answer
    case "$answer" in
        n|N|no|NO) return 1 ;;
        *) return 0 ;;
    esac
}

printf "Capsem Bootstrap (%s)\n" "$(uname -s)"
echo "========================"
echo ""

# --- Phase 1: Bare minimum tools ---
# bash/git/curl are hard prereqs (we can't even curl an installer without
# curl). rustup and just are auto-installed via official installers --
# rustup from sh.rustup.rs, just from just.systems (no cargo dependency
# so it works on a fresh machine before any rust toolchain exists).
MISSING_HARD=0
for tool in bash git curl; do
    if command -v "$tool" >/dev/null 2>&1; then
        printf "  [ok]   %s\n" "$tool"
    else
        printf "  [MISS] %s -- install via your OS package manager\n" "$tool"
        MISSING_HARD=$((MISSING_HARD + 1))
    fi
done

if [ "$MISSING_HARD" -gt 0 ]; then
    echo ""
    echo "Install the hard prereqs above, then re-run: sh bootstrap.sh"
    exit 1
fi

# Make sure ~/.cargo/bin and ~/.local/bin are visible for the rest of this
# script -- both installers drop binaries there but don't reload PATH.
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

if command -v rustup >/dev/null 2>&1; then
    printf "  [ok]   rustup\n"
elif confirm "rustup (Rust toolchain manager, via sh.rustup.rs)"; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --default-toolchain 1.97.1 --profile minimal
    # shellcheck disable=SC1091
    [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
fi

if command -v just >/dev/null 2>&1; then
    printf "  [ok]   just\n"
elif confirm "just (command runner, via just.systems -> ~/.local/bin)"; then
    mkdir -p "$HOME/.local/bin"
    curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh \
        | bash -s -- --to "$HOME/.local/bin"
fi

# Final check: rustup + just are non-negotiable for the rest of the script.
for tool in rustup just; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf "  [FAIL] %s missing -- bootstrap cannot continue without it\n" "$tool"
        exit 1
    fi
done

# --- Phase 2: Install dependencies ---
echo ""
echo "== Installing dependencies =="

if ! command -v uv >/dev/null 2>&1; then
    if confirm "uv (Python package manager, via astral.sh -> ~/.local/bin)"; then
        curl --proto '=https' --tlsv1.2 -LsSf https://astral.sh/uv/install.sh \
            | env INSTALLER_NO_MODIFY_PATH=1 sh
    fi
fi
if command -v uv >/dev/null 2>&1; then
    printf "  Python deps (uv sync)...\n"
    uv sync
else
    printf "  [SKIP] Python deps (uv not installed -- some just recipes will fail)\n"
fi

# flock: multi-agent coordination lock for heavy just recipes.
# Linux ships it in util-linux; macOS needs brew install flock.
if ! command -v flock >/dev/null 2>&1; then
    case "$(uname -s)" in
        Darwin)
            if command -v brew >/dev/null 2>&1; then
                if confirm "flock (multi-agent recipe lock, via brew)"; then
                    brew install flock
                fi
            else
                printf "  [SKIP] flock (Homebrew not installed -- install brew, then: brew install flock)\n"
            fi ;;
        Linux)
            printf "  [SKIP] flock (missing -- install util-linux via your package manager)\n" ;;
    esac
fi

# The production host-SBOM generator reads the exact Debian packages emitted
# by the Linux release rail. Ubuntu dpkg currently writes data.tar.zst, while
# macOS bsdtar delegates that member to the external zstd executable. Install
# it during canonical bootstrap so `just test` cannot discover the missing
# decoder only after both release packages and the macOS package are built.
if [ "$(uname -s)" = "Darwin" ] && ! command -v zstd >/dev/null 2>&1; then
    if command -v brew >/dev/null 2>&1; then
        if confirm "zstd (Debian package/SBOM archive support, via brew)"; then
            brew install zstd
        fi
    else
        printf "  [SKIP] zstd (Homebrew not installed -- install brew, then: brew install zstd)\n"
    fi
fi

# The canonical macOS gate installs the exact package in a disposable Tart
# guest. Bootstrap installs only the small host-side tools; the 25 GB macOS
# image remains an explicit `just test` cost and is cached by Tart thereafter.
if [ "$(uname -s)" = "Darwin" ] \
    && { ! command -v tart >/dev/null 2>&1 || ! command -v sshpass >/dev/null 2>&1; }; then
    if command -v brew >/dev/null 2>&1; then
        if confirm "Tart + sshpass (clean macOS package install gate, via brew)"; then
            # Tart 2.32 depends on softnet from the same official Cirrus Labs
            # tap. Current Homebrew requires that dependency to be trusted
            # explicitly before it will evaluate the Tart formula.
            brew trust --formula cirruslabs/cli/softnet
            brew install cirruslabs/cli/tart cirruslabs/cli/sshpass
        fi
    else
        printf "  [SKIP] Tart install gate (Homebrew not installed -- install brew, then Tart + sshpass)\n"
    fi
fi

# Bootstrap owns the one-time OCI pull and proves the VM really boots. Doctor
# repeats the clone/boot/SSH proof from the cache and fails if the base is
# missing, so release qualification never discovers a dead Tart setup late.
if [ "$(uname -s)" = "Darwin" ] \
    && command -v tart >/dev/null 2>&1 \
    && command -v sshpass >/dev/null 2>&1; then
    printf "  Tart base image + boot readiness...\n"
    uv run python "$SCRIPT_DIR/scripts/tart_readiness.py"
    export CAPSEM_BOOTSTRAP_TART_PROVEN=1
fi

if command -v pnpm >/dev/null 2>&1; then
    printf "  Frontend deps (pnpm install)...\n"
    (cd frontend && CI=true pnpm install --frozen-lockfile)
else
    case "$(uname -s)" in
        Darwin)
            if command -v brew >/dev/null 2>&1 && confirm "pnpm (Node package manager, via brew)"; then
                brew install pnpm
            fi ;;
        Linux)
            # Official installer; no npm or sudo required. Drops to ~/.local/share/pnpm.
            if confirm "pnpm (Node package manager, via get.pnpm.io)"; then
                curl --proto '=https' --tlsv1.2 -fsSL https://get.pnpm.io/install.sh \
                    | env SHELL=/bin/sh ENV="" PNPM_HOME="$HOME/.local/share/pnpm" sh -
                export PNPM_HOME="$HOME/.local/share/pnpm"
                export PATH="$PNPM_HOME:$PATH"
            fi ;;
    esac
    if command -v pnpm >/dev/null 2>&1; then
        printf "  Frontend deps (pnpm install)...\n"
        (cd frontend && CI=true pnpm install --frozen-lockfile)
    else
        printf "  [SKIP] Frontend deps (pnpm not installed -- doctor will catch this)\n"
    fi
fi

# Container runtime. Required by `just build-assets` (kernel/rootfs are built
# in Docker) which doctor's auto-fix step runs next.
#   macOS: install colima + docker + docker-buildx via brew, start the VM.
#   Linux: docker is system-managed (apt/dnf, sudo, group membership, daemon
#          setup). Auto-installing it here would be invasive and distro-
#          specific -- print a clear hint instead and let doctor surface it.
case "$(uname -s)" in
    Darwin)
        if ! command -v brew >/dev/null 2>&1; then
            printf "  [SKIP] container runtime (Homebrew not installed)\n"
        else
            if ! command -v colima >/dev/null 2>&1; then
                if confirm "colima + docker CLI (container runtime, via brew)"; then
                    brew install colima docker docker-buildx
                    # Wire docker-buildx into ~/.docker/cli-plugins so `docker buildx` works.
                    mkdir -p "$HOME/.docker/cli-plugins"
                    ln -sf "$(brew --prefix docker-buildx)/bin/docker-buildx" \
                        "$HOME/.docker/cli-plugins/docker-buildx"
                fi
            fi
            # Start Colima if installed but not running. Doctor's fix can't
            # do this -- it would just print the suggestion and fail.
            if command -v colima >/dev/null 2>&1 && ! colima status >/dev/null 2>&1; then
                if confirm "start Colima now (vz, 16 GB RAM, 8 CPU, ${DOCKER_DISK_GIB} GB disk -- release-gate profile)"; then
                    colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8 --disk "$DOCKER_DISK_GIB"
                fi
            fi
            # The persistent config can say Rosetta is enabled while the
            # running VM predates that config and has no live binfmt handler.
            # Repair that stale runtime before the Docker probe or expensive
            # cross-architecture builds. Intel Macs do not need Rosetta.
            if command -v colima >/dev/null 2>&1 \
                && colima status >/dev/null 2>&1 \
                && [ "$(uname -m)" = "arm64" ]; then
                colima_yaml="$HOME/.colima/default/colima.yaml"
                if [ -f "$colima_yaml" ] \
                    && grep -q 'rosetta: true' "$colima_yaml" \
                    && grep -q 'vmType: vz' "$colima_yaml"; then
                    if ! colima ssh -- test -f /proc/sys/fs/binfmt_misc/rosetta >/dev/null 2>&1; then
                        if confirm "restart Colima to register Rosetta for amd64 release builds"; then
                            colima restart
                        fi
                    fi
                elif confirm "restart Colima with VZ Rosetta for amd64 release builds"; then
                    colima stop
                    colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8 --disk "$DOCKER_DISK_GIB"
                fi
            fi
            if command -v docker >/dev/null 2>&1; then
                docker info >/dev/null
                docker_dns_ready=0
                for attempt in $(seq 1 30); do
                    if docker run --rm --pull=missing alpine:3.20 getent hosts ghcr.io >/dev/null 2>&1; then
                        docker_dns_ready=1
                        break
                    fi
                    sleep 1
                done
                if [ "$docker_dns_ready" -ne 1 ]; then
                    printf "  [FAIL] Docker DNS did not become ready after Colima startup\n" >&2
                    exit 1
                fi
                printf "  [ok]   docker VM probe (info + registry DNS)\n"
            fi
        fi ;;
    Linux)
        if ! command -v docker >/dev/null 2>&1; then
            printf "  [SKIP] docker (install via your package manager: 'apt install docker.io' or 'dnf install docker', then 'sudo usermod -aG docker \$USER' and re-login)\n"
        elif ! docker info >/dev/null 2>&1; then
            printf "  [SKIP] docker daemon (run 'sudo systemctl enable --now docker' and ensure your user is in the docker group)\n"
        fi ;;
esac

# --- Phase 3: Run doctor with auto-fix ---
echo ""
echo "== Running doctor (with auto-fix) =="
echo ""
"$SCRIPT_DIR/scripts/doctor-common.sh" --fix

echo ""
echo "========================"
echo "Bootstrap complete. Verify with:"
echo ""
echo "  just exec \"echo hi\"  # Boot VM + run echo + exit"
echo "  just shell           # Interactive VM shell"
echo "  just test            # Unit tests + cross-compile + frontend check"
echo ""
