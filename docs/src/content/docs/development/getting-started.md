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
| **Docker (via Colima on macOS)** | Needed for `just build-assets` (kernel + rootfs builds) |

### Linux

| Requirement | Detail |
|-------------|--------|
| **Debian/Ubuntu** | apt-based distro (for .deb install) |
| **x86_64 or arm64** | Both architectures supported |
| **KVM** | `/dev/kvm` must be accessible. Load `kvm-intel` or `kvm-amd` module. |
| **Docker** | Needed for `just build-assets` (kernel + rootfs builds) |

## Clone and bootstrap

```bash
git clone https://github.com/google/capsem.git && cd capsem
sh scripts/bootstrap.sh
```

The bootstrap script checks bare-minimum tools (bash, git, curl, rustup, just), installs Python and frontend dependencies, then runs `just doctor --fix` to validate the full environment and auto-fix any fixable issues (missing Rust targets, cargo tools, config files).

The only prerequisite is a POSIX shell. See [Life of a Build](./stack) for what gets built by what and how the tools fit together.

## Build VM assets

```bash
just build-assets
```

Builds the Linux kernel and rootfs via Docker (~10 min on first run). Assets are gitignored and must be built locally. See [Life of a Build > Container runtime](./stack#container-runtime) if you need to configure Colima resources.

## Verify

```bash
just run "echo hello from capsem"
```

If this prints "hello from capsem" and exits cleanly, you're set. See [Life of a Build](./stack) for what `just run` does under the hood.

## Daily workflow

```bash
just run              # Build + boot VM interactively (~10s)
just run "CMD"        # Boot + run command + exit
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

To add packages, AI providers, or change security policy, edit the TOML configs in `guest/config/` and rebuild. See [Customizing VM Images](./custom-images) for the workflow.

## API keys (optional)

Needed for `just full-test` (integration tests exercise real AI API calls) and interactive AI sessions inside the VM.

Create `~/.capsem/user.toml`:

```toml
[ai.anthropic]
api_key = "sk-ant-..."

[ai.google]
api_key = "AIza..."
```

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

### `just build-assets` fails with exit code 137

The container runtime ran out of memory. See [Life of a Build > Container runtime](./stack#container-runtime) for how to increase memory to 8GB.

### `just build-assets` fails with "Release file not valid yet"

The container VM's clock has drifted:
- Colima: `colima stop && colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8`
- Docker Desktop: restart Docker Desktop

### `just run` fails with "assets not found"

Run `just build-assets` first. Assets are gitignored and must be built locally.

For runtime issues (disk full, boot hangs, cross-compile errors, network problems), see [Troubleshooting](/debugging/troubleshooting/).
