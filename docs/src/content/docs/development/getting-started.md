---
title: Developer Quick Start
description: Clone, bootstrap, and boot a Capsem VM in three commands.
sidebar:
  order: 1
---

## Platform requirements

### macOS

| Requirement | Detail |
|-------------|--------|
| **macOS 13+** (Ventura) | Required for Virtualization.framework |
| **Apple Silicon** (arm64) | Intel Macs are not supported |
| **Xcode Command Line Tools** | Provides `codesign`, `cc`, and system headers. Install: `xcode-select --install` |
| **Docker (via Colima on macOS)** | Needed for `just build-assets code` (kernel + rootfs builds) |

### Linux

| Requirement | Detail |
|-------------|--------|
| **Debian/Ubuntu** | apt-based distro (for .deb install) |
| **x86_64 or arm64** | Both architectures supported |
| **KVM** | `/dev/kvm` must be accessible. Load `kvm-intel` or `kvm-amd` module. |
| **Docker** | Needed for `just build-assets code` (kernel + rootfs builds) |

## Clone and bootstrap

```bash
git clone https://github.com/google/capsem.git && cd capsem
./bootstrap.sh            # interactive: prompts [Y/n] before each install
./bootstrap.sh --yes      # non-interactive: auto-yes (use in CI)
```

`bootstrap.sh` lives at the repo root. It walks the dependency tree top-down, asking before installing anything, and exits clean when everything's already in place.

### What bootstrap installs

| Phase | Tool | How it's installed | Why |
|-------|------|---------------------|-----|
| 1 (hard prereqs) | `bash`, `git`, `curl` | system package manager (you install) | Without curl we can't fetch any installer |
| 1 | `rustup` (stable, minimal profile) | `sh.rustup.rs` official installer | Source of `cargo` |
| 1 | `just` | `just.systems` installer → `~/.local/bin` | Recipe runner — used by every other build step |
| 2 | `uv` | `astral.sh/uv` installer → `~/.local/bin` | Python deps for `capsem-builder` |
| 2 | Python deps | `uv sync` | Locked via `uv.lock` |
| 2 (macOS) | `flock`, `pnpm` | `brew` | flock = multi-agent recipe lock; pnpm = frontend deps |
| 2 (macOS) | `colima`, `docker`, `docker-buildx` | `brew` + symlink into `~/.docker/cli-plugins` | Container runtime for `just build-assets code` |
| 2 (macOS) | Colima VM | `colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8` | Runs Docker; Rosetta enables x86_64 cross-builds |
| 2 | Frontend deps | `pnpm install --frozen-lockfile` (in `frontend/`) | Tauri UI dependencies |
| 3 | Doctor `--fix` | `scripts/doctor-common.sh --fix` | Installs Rust targets, `cargo-llvm-cov`, `cargo-audit`, `b3sum`, `cargo-tauri` (= `tauri-cli` crate), `cargo-sbom`, builds VM assets, packs initrd |

Pressing **Enter** at any prompt accepts the install (Y is the default). Type `n` to skip — bootstrap continues and surfaces the missing tool in the doctor report at the end.

## Build VM assets

```bash
just build-assets code
```

Builds the Linux kernel and rootfs via Docker (~10 min on first run). The code
profile currently builds against the stable 7.0 kernel lane and EROFS/LZ4HC
rootfs contract. Kernel branch changes are backend image-spec changes made
through the profile-derived build rail, then verified by `capsem-admin image
build` and the Linux handoff gate. Assets are gitignored and must be built
locally. See [Life of a Build > Container runtime](./stack#container-runtime)
if you need to retune Colima resources.

The build is profile-derived. `code` is the default coding-agent profile, and
the runtime profile for the current local build is generated under
`target/config/` by `capsem-admin profile materialize` during `just shell`,
`just exec`, `just smoke`, `just test`, and release packaging.

## Verify

```bash
just exec "echo hello from capsem"
```

If this prints "hello from capsem" and exits cleanly, you're set. See [Life of a Build](./stack) for what `just run` does under the hood.

## Daily workflow

```bash
just shell            # Build + boot VM interactively (~10s)
just exec "CMD"        # Boot + run command + exit
just test             # Unit tests + cross-compile + frontend check
just dev              # Hot-reloading Tauri app (frontend + Rust)
just ui               # Frontend-only dev server (mock mode, no VM)
```

See [Just Recipes](./just-recipes) for the complete reference and dependency chains.

## Codesigning

On macOS, the compiled binary must be codesigned with Apple's `com.apple.security.virtualization` entitlement or Virtualization.framework calls crash at runtime. The justfile handles this automatically -- every `just run` re-signs the binary before booting. This is not required on Linux.

**Prerequisites:**
- Xcode Command Line Tools (`xcode-select --install`)
- `entitlements.plist` in the repo root (checked into git)

**Validation:** `just doctor` runs a six-step codesigning check (macOS only):

| Check | What it validates | Fix if it fails |
|-------|-------------------|-----------------|
| Xcode CLTools | `xcode-select -p` returns a path | `xcode-select --install` |
| `codesign` binary | The tool exists in PATH | Install Xcode CLTools (see above) |
| `entitlements.plist` | The file exists and is readable | `just doctor-fix` (auto-restores from git) |
| `.cargo/config.toml` | Cargo runner configured | `just doctor-fix` (auto-restores from git) |
| `run_signed.sh` | Script exists and is executable | `just doctor-fix` (auto-restores from git) |
| Test sign | Compiles a tiny binary + signs it with entitlements | See [troubleshooting](#codesign-fails) below |

No Apple Developer ID certificate is needed for local development -- ad-hoc signing (`--sign -`) is sufficient.

## Customizing the VM image

To add packages or guest tools, edit the profile-owned files under
`config/profiles/code/` and rebuild through `just build-assets code`.
Profile/corp files own security rules and provider access. See
[Customizing VM Images](./custom-images) for the workflow.

## API keys (optional)

Interactive AI sessions can configure credentials inside the VM or let the
credential broker capture/materialize them at a supported boundary. Raw API keys
are not settings-owned boot secrets; logs and profile state use BLAKE3
references.

## Troubleshooting

### `just doctor` fails

Run `just doctor-fix` to auto-fix all fixable issues. Fixes run in dependency order (Rust targets, cargo tools, config files, build assets, guest binaries). Non-fixable issues (system tools like node, docker) show platform-specific install hints.

### Codesign fails

If `just run` or `just doctor` reports a codesign failure:

1. **Xcode CLTools missing:** `xcode-select --install` (opens the system installer)
2. **`entitlements.plist` missing:** `git checkout entitlements.plist`
3. **Test sign fails but tools are present:**
   - Try manually: `codesign --sign - --entitlements entitlements.plist --force target/debug/capsem`
   - Check SIP status: `csrutil status` (should be "enabled")
   - Verify `cc` works: `echo 'int main(){return 0;}' | cc -x c -o /tmp/test -` -- if this fails, reinstall CLTools: `sudo rm -rf /Library/Developer/CommandLineTools && xcode-select --install`

### `just build-assets code` or `just test-install` fails with exit 137 (or 143 mid-cargo-build)

The container runtime ran out of memory. The Tauri install-test cold build needs >12GB. See [Life of a Build > Container runtime](./stack#container-runtime) for how to bump Colima to 16GB.

### `just build-assets code` fails with "Release file not valid yet"

The container VM's clock has drifted:
- Colima: `colima stop && colima start --vm-type vz --vz-rosetta --memory 16 --cpu 8`
- Docker Desktop: restart Docker Desktop

### `just run` fails with "assets not found"

Run `just build-assets code` first. Assets are gitignored and must be built locally.

For runtime issues (disk full, boot hangs, cross-compile errors, network problems), see [Troubleshooting](/debugging/troubleshooting/).
