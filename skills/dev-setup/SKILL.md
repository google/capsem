---
name: dev-setup
description: Setting up a Capsem dev environment from scratch. Use when onboarding, provisioning a machine, or troubleshooting environment and container-runtime issues.
---

# Developer Setup

## Prerequisites

- **Linux**: supported Debian/Ubuntu or dnf-based host, x86_64/arm64, sudo/root for initial packages, and `/dev/kvm` plus `/dev/vhost-vsock` for VM execution and guest communication
- **macOS 13+** (Ventura or later) -- required for Virtualization.framework
- **Apple Silicon** (arm64) -- primary macOS target. Intel Macs are not supported for VM features.
- **Docker** (native on Linux, Colima on macOS) -- needed for `just _build-assets` (kernel + rootfs builds)
- **Tart + sshpass (macOS)** -- needed for the clean-macOS package install owned by `just test`

## Required tools

Run `just doctor` to check all of these:

| Tool | Purpose | Install |
|------|---------|---------|
| Rust (stable) | Host + guest binaries | `rustup` |
| just | Task runner | `cargo install just` |
| pnpm | Frontend package manager | `npm i -g pnpm` |
| Node.js 24+ | Frontend build | `nvm` or `brew install node` |
| uv | Python package manager | `curl -LsSf https://astral.sh/uv/install.sh \| sh` |
| Docker (via Colima on macOS) | VM image builds | `brew install colima docker` (macOS) or `sudo apt install docker.io` (Linux) |
| Docker BuildKit (buildx) | Cross-arch container builds | `brew install docker-buildx` (macOS); bootstrap selects `docker-buildx` from Ubuntu/Debian archives or the discovered `docker-buildx-plugin` fallback |
| Bubblewrap (Linux) | Kernel-enforced loopback-only host gate | `sudo apt install bubblewrap` (bootstrap-owned) |
| cpio | Initrd unpack/repack | system on macOS; `sudo apt install cpio` or `sudo dnf install cpio` on Linux (bootstrap-owned) |
| Tart + sshpass (macOS) | Disposable clean-Mac package install gate | `brew trust --formula cirruslabs/cli/softnet && brew install cirruslabs/cli/tart cirruslabs/cli/sshpass` |

Rust targets (auto-installed by `just doctor fix`):
- `aarch64-unknown-linux-musl` -- guest binaries (arm64)
- `x86_64-unknown-linux-musl` -- guest binaries (x86_64)

Cargo tools (auto-installed by `just doctor fix`):
- `cargo-nextest` -- native Rust test runner
- `cargo-llvm-cov` -- coverage
- `cargo-audit` -- vulnerability scanner
- `cargo-tauri` -- Tauri CLI
- `cargo-sbom` -- Rust SBOM generation
- `b3sum` -- BLAKE3 checksums

## Container runtime setup

On macOS, Docker runs inside a Colima VM. The default memory allocation may be too small -- the rootfs build runs apt installs, npm installs, and curl-based CLI installers concurrently, which can OOM-kill the build (exit code 137).

**Minimum**: 12GB RAM. **Recommended**: 16GB RAM, 8 CPUs (Tauri's GTK/webkit2gtk dep chain pushes the install-test build past 8GB on cold caches; 12GB is the floor that doesn't OOM).

### Colima (macOS)

```bash
# First-time setup
brew install colima docker
colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8

# Restart with new resources
colima stop
colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8

# Verify
docker info | grep -E 'Total Memory|CPUs'
```

### Colima recovery discipline

If Docker-dependent recipes fail on macOS, do not report Docker/Colima as
unavailable until you have checked for the common half-running Colima state.
The signature is:

- `colima list` says the profile is `Running`
- `docker version` / `docker info` cannot connect to
  `~/.colima/default/docker.sock`
- `colima ssh -- docker ps` fails with `kex_exchange_identification`,
  `Connection reset by peer`, or `colima status` reports
  `error retrieving current runtime: empty value`

First recovery attempt:

```bash
colima stop
colima start
docker version
```

If the profile needs its expected resources restored, start with the explicit
Capsem defaults instead:

```bash
colima stop
colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8
docker version
```

Only after that restart fails should you treat Colima as a real environment
blocker. Record the exact failed command and the Docker/Colima output.

### Linux

Docker runs natively on Linux -- no Colima or memory tuning needed. Canonical
bootstrap installs the native build libraries, Docker/Buildx, and Bubblewrap;
installs the distro's QEMU user/binfmt package and proves the fix-binary
registration Docker needs for the other architecture; enables the daemon;
loads KVM and vhost-vsock; and provisions both device nodes for repeated VM
lifecycles in the current shell. KVM uses the same durable mode as Linux release
CI because systemd-logind can remove a named ACL after the first VM exits.
vhost-vsock remains group-owned with a narrow current-user ACL. Do not
hand-provision binfmt or either device, and do not weaken the separate macOS
VZ/Seatbelt path; rerun the checked-in bootstrap so distro registration and
udev rules remain the authority.

```bash
./bootstrap.sh --yes
```

`just doctor` checks these resources automatically and fails if below minimum.
On Linux bootstrap and hosted CI run the checked-in Bubblewrap proof: only
`lo`, usable loopback/devices, and no direct egress. An ephemeral GitHub Ubuntu
runner may lift only the exact AppArmor user-namespace switch responsible for
`RTM_NEWADDR`, after which the complete proof must pass again. Local bootstrap
never applies that hosted repair. When doctor is already inside a gate, it
verifies that only `lo` is visible instead of attempting a nested namespace.

## First-time setup

```bash
# 1. Clone and enter
git clone <repo> && cd capsem

# 2. Bootstrap (interactive: prompts [Y/n] before each install; --yes for CI)
./bootstrap.sh
#   ./bootstrap.sh --yes    # non-interactive

# 3. Boot the VM to verify everything works
just exec "echo hello from capsem"
```

`bootstrap.sh` lives at the **repo root** (not under `scripts/`). It runs `just _build-assets` as part of doctor's auto-fix, so step 3 just confirms the VM boots.

### What bootstrap installs

Three phases. Default at every prompt is **Yes** (Enter accepts; type `n` to decline). `--yes` and non-tty input both auto-accept.

| Phase | Tool | Channel |
|-------|------|---------|
| 1 (hard prereqs) | `bash`, `git`, `curl` | system package manager (you install) |
| 1 | `rustup` (stable, minimal profile) | `sh.rustup.rs` |
| 1 | `just` | `just.systems` -> `~/.local/bin` |
| 2 | `uv` | `astral.sh/uv` -> `~/.local/bin` |
| 2 | Python deps | `uv sync --frozen` |
| 2 | Rust workspace deps | `cargo fetch --locked` before sandboxed qualification |
| 2 (Linux) | native compiler/Tauri libs, `cpio`, Docker/Buildx, Bubblewrap | apt or dnf |
| 2 (Linux) | configured Node major + pnpm 10 | SHA256-verified official Node archive + npm |
| 2 (Linux) | Docker/KVM/vhost-vsock current-session access | groups + checked-in udev policy and narrow socket/vhost ACL |
| 2 (macOS) | `flock`, `pnpm` | `brew` |
| 2 (macOS) | `tart`, `sshpass` | `brew` |
| 2 (macOS) | `colima`, `docker`, `docker-buildx` | `brew` (+ symlink into `~/.docker/cli-plugins`) |
| 2 (macOS) | Colima VM | `colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8` |
| 2 | Frontend, docs, site, and release-site deps | config-driven `capsem-gate install-node` with frozen lockfiles |
| 3 | Doctor `--fix` | `scripts/doctor-common.sh --fix` -- Rust targets, exact config-owned Cargo tools (`cargo-nextest`, `cargo-llvm-cov`, `cargo-audit`, `b3sum`, `cargo-tauri`, `cargo-sbom`), build VM assets, pack initrd |

The VM asset rail materializes its digest-pinned OBOM tools inside its own
architecture-matched helper; do not install a parallel global cdxgen.

### Kernel version

Kernel selection is part of the profile-derived image build, not a standalone
developer setting. The build backend reads the exact kernel release and
SHA-256 from the checked-in build contract, then verifies the source archive
before extraction while building profile assets through `capsem-admin`/`just`.
Do not add a parallel kernel setting or a mutable latest-release lookup.

Or step by step:

```bash
just doctor          # Check tools (colored output, structured recap)
just doctor fix      # Auto-fix missing targets, cargo tools, config files
just _build-assets    # Build kernel + rootfs (~10 min)
just exec "echo hi"   # Verify VM boots
```

If step 4 prints "hello from capsem" and exits cleanly, you're set.

## Daily workflow

```bash
just shell            # Build + boot VM interactively (~10s)
just exec "CMD"        # Build + boot + run command + exit
just test             # Full release gate, including clean Tart .pkg install on macOS
just dev ui               # Frontend dev server (mock mode, no VM)
just dev              # Full Tauri app with hot-reload
```

See `/dev-just` for the complete recipe reference.

## Credentials

Do not create `~/.capsem/user.toml`. Credentials are captured and replayed by
the credential broker plugin through profile/corp policy. Hermetic tests use
the local mock server and Ironbank fixtures; real OAuth/API-key manual runs are
debug evidence, not release proof.

Do not add setup-time admin or guest config roots. Runtime behavior is
profile/corp-owned; settings are UI/application preferences only. Generated
settings UI metadata may render controls, but it is not a product config
authority.

## Claude Code permissions

To avoid repeated permission prompts when using `just` and `capsem` commands, add these to your Claude Code settings. Run `/update-config` or edit `.claude/settings.local.json`:

```json
{
  "permissions": {
    "allow": [
      "Bash(just *)",
      "Bash(uv run *)",
      "Bash(cargo *)",
      "Bash(pnpm *)",
      "Bash(cd frontend && pnpm *)",
      "Bash(npx *)",
      "Bash(python3 scripts/*)",
      "Bash(rustup *)"
    ]
  }
}
```

This allows:
- `just *` -- all recipes (run, test, build-assets, query-session, list-sessions, doctor, etc.)
- `uv run *` -- capsem-builder CLI and Python scripts
- `cargo *` -- Rust builds, tests, checks
- `pnpm *` -- frontend package management and builds
- `npx *` -- skills CLI and other npx tools
- `python3 scripts/*` -- project scripts (check_session, list_sessions, etc.)
- `rustup *` -- target/component management

## Codesigning

The app binary must be codesigned with `com.apple.security.virtualization` entitlement or
Virtualization.framework calls crash. The justfile handles this automatically via `_sign` recipe.

**Prerequisites** (macOS only):
- Xcode Command Line Tools: `xcode-select --install`
- `entitlements.plist` must exist in the repo root (checked into git)

**Verification**: `just doctor` includes a signing test that compiles a tiny binary, signs it with
the entitlements, and verifies the operation succeeds. Run `just doctor` after initial setup to
confirm signing works.

**Linux developers**: codesign is not available and not needed on Linux. VM features use the
KVM backend when `/dev/kvm` and `/dev/vhost-vsock` are available. Use `just test`
for the same artifact-recording performance suite as macOS.

## Troubleshooting

### `just exec` fails with codesign error
- Run `just doctor` -- it will diagnose the specific signing issue
- Ensure Xcode CLTools are installed: `xcode-select --install`
- Check entitlements file exists: `cat entitlements.plist`
- Try manual sign: `codesign --sign - --entitlements entitlements.plist --force target/debug/capsem`
- Check SIP status: `csrutil status`

### `just doctor` fails
Run `just doctor fix` to auto-fix all fixable issues. Fixes run in dependency order (rustup targets before cargo tools before build-assets before pack-initrd). Non-fixable issues show install hints.

### `just _build-assets` or `just _gate-install` fails with exit code 137 (or 143 mid-cargo-build)
The container runtime VM ran out of memory. Bump Colima to at least 12GB (16GB recommended):
- Colima: `colima stop && colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8`
- Linux: Docker runs natively, no memory tuning needed

### `just _build-assets` fails with "Release file not valid yet"
The container VM's clock has drifted. The builder uses `Acquire::Check-Valid-Until=false` to work around this, but if you see this error on an old builder version:
- Colima: `colima stop && colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8` (resets clock)
- Docker Desktop: restart Docker Desktop

### `just _build-assets` fails (other)
- Check Docker is running: `docker info`
- Check the profile contract is valid: `capsem-admin profile check config/profiles/code/profile.toml --config-root config`
- On first run, Docker image pulls can be slow

### `just exec` fails with "assets not found"
Run `just _build-assets` first. Assets are gitignored and must be built locally.

### `cargo run` or `cargo test` crashes with signing error
- `.cargo/config.toml` must exist and be tracked in git -- it configures the custom runner (`scripts/run_signed.sh`) that signs binaries with Virtualization.framework entitlements before execution
- If missing: `git checkout .cargo/config.toml`
- The justfile `_sign` recipe signs separately, so `just exec` works even without the cargo runner -- but direct `cargo run`/`cargo test` and IDE integrations will crash
- **Lesson:** bare `.gitignore` patterns (no `/` prefix) match at any depth. Always anchor with `/` when you mean root-only (e.g., `/config.toml` not `config.toml`), or you risk silently ignoring files in subdirectories like `.cargo/`

### Cross-compile errors
- Check `.cargo/config.toml` has linker config for musl targets
- Run `rustup target add aarch64-unknown-linux-musl x86_64-unknown-linux-musl`
- Platform-specific type issues: use `as _` for libc calls (see `/dev-rust-patterns`)

### Disk full / Colima eating all disk space
Docker builds accumulate images, build cache, and stopped containers inside the Colima VM. The VM uses a Virtualization.framework raw disk that only grows, never shrinks on its own -- even after `docker system prune`, macOS doesn't get the space back.

The `_docker-gc` recipe runs automatically after `build-assets`, `cross-compile`, and `test-install` to prevent this. It prunes containers, images >72h, build cache >72h, and runs `fstrim` to release freed blocks back to macOS. If disk is already full:

```bash
# One-time recovery
docker system prune -af --volumes           # free space inside VM
colima ssh -- sudo fstrim /mnt/lima-colima  # release it to macOS
```

To check current state: `colima ssh -- docker system df` (inside VM) and `du -sh ~/.colima` (host).

### Docker credential helper error (`docker-credential-osxkeychain not found`)
When Colima is installed standalone (without Docker Desktop), `~/.docker/config.json` may reference a credential helper that doesn't exist. The symptom is `docker run` failing to pull images with `exec: "docker-credential-osxkeychain": executable file not found`.

Fix: set `credsStore` to empty string in `~/.docker/config.json`:
```json
{ "credsStore": "" }
```

`just doctor` checks for this under "Container Runtime" and will flag the mismatch.

### VM boot hangs
- Check codesigning: `codesign -dvv target/debug/capsem 2>&1 | grep entitlements`
- Check assets exist: `ls assets/arm64/vmlinuz assets/arm64/rootfs.erofs`
- Check kernel architecture matches host: wrong-arch kernel causes silent hang. `VmConfig::build()` now rejects mismatched kernels at config time.
- Try with debug logs: `RUST_LOG=capsem=debug just exec`

## Doctor architecture

The doctor system is three bash scripts:

```
scripts/
  doctor-common.sh    # Entry point, cross-platform checks, fix registry, recap
  doctor-macos.sh     # macOS: Tart, Colima, Rosetta, codesigning, brew hints
  doctor-linux.sh     # Linux: KVM, apt/dnf hints
```

`just doctor` calls `doctor-common.sh`. `just doctor fix` calls `doctor-common.sh --fix`.

### Fix registry

All fixable issues use an **ordered fix registry** defined at the top of `doctor-common.sh`. Each entry has an ID, command, and description. Checks call `fixable <id> <label>` to mark a fix as needed. Fixes run in registry order (dependency order), deduped by design.

Registry order (each depends on the ones above it):
1. `rustup-targets` -- cross-compile targets
2. `llvm-tools` -- rust-lld linker
3. `cargo-nextest`, `cargo-llvm-cov`, `cargo-audit`, `b3sum`, `cargo-tauri`, `cargo-sbom` -- exact config-owned Cargo tools
4. `entitlements`, `cargo-config`, `run-signed` -- git checkout config files
5. `pnpm-install` -- every locked Node workspace, through `capsem-gate install-node`
6. `build-assets` -- VM kernel + rootfs (needs docker)
7. `pack-initrd` -- guest binaries (needs assets)

### Design rules

- **Fixable checks use `fixable <id> <label>`**, not raw `fail()`. This registers the fix in the ordered registry.
- **Non-fixable checks use `fail()` with an install hint.** System tools (node, docker, etc.) can't be auto-installed safely.
- **Platform-specific checks live in `doctor-macos.sh` / `doctor-linux.sh`.** Each defines `check_platform()` and `tool_hint()`.
- **Test, don't just check.** The codesigning section compiles and signs a test binary. `docker buildx version` tests functionality, not just file existence.
- **Recover Colima before declaring Docker dead.** On macOS, a stale Colima VM
  can leave the Docker socket present but unusable. Use the Colima recovery
  discipline above before filing or reporting a Docker/Colima blocker.
- **Bootstrap calls doctor.** `bootstrap.sh` checks bare minimums (bash, git, curl), provisions the complete platform host (including Linux Docker/KVM/Bubblewrap and configured Node), installs Rust/Python/frontend tools, then runs `doctor-common.sh --fix`.
