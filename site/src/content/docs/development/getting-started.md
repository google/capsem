---
title: Developer Quick Start
description: Clone, bootstrap, and boot a Capsem VM in three commands.
sidebar:
  order: 1
---

## Platform requirements

| Requirement | Detail |
|-------------|--------|
| **macOS 13+** (Ventura) | Required for Virtualization.framework |
| **Apple Silicon** (arm64) | Primary target. Intel Macs are not supported for VM features. |
| **Xcode Command Line Tools** | Provides `codesign`, `cc`, and system headers. Install: `xcode-select --install` |
| **Docker or Podman** | Needed for `just build-assets` (kernel + rootfs builds) |

:::note[Linux developers]
You can build VM images (`just build-assets`), run tests (`just test`), and audit dependencies (`just audit`) on Linux. VM features (`just run`, `just dev`, `just bench`) require macOS with Apple Silicon.
:::

## Clone and bootstrap

```bash
git clone https://github.com/google/capsem.git && cd capsem
sh scripts/bootstrap.sh
```

The bootstrap script detects your OS (macOS or Linux), checks every required tool, and prints platform-specific install commands for anything missing. On macOS it also verifies Xcode Command Line Tools are installed. Once all tools are present it installs Python and frontend dependencies and runs `just doctor` to validate the full environment.

The only prerequisite is a POSIX shell. See [Life of a Build](./stack) for what gets built by what and how the tools fit together.

## Build VM assets

```bash
just build-assets
```

Builds the Linux kernel and rootfs via Docker/Podman (~10 min on first run). Assets are gitignored and must be built locally. See [Life of a Build > Container runtime](./stack#container-runtime) if you need to configure Docker or Podman memory.

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

The compiled binary must be codesigned with Apple's `com.apple.security.virtualization` entitlement or Virtualization.framework calls crash at runtime. The justfile handles this automatically -- every `just run` re-signs the binary before booting.

**Prerequisites:**
- Xcode Command Line Tools (`xcode-select --install`)
- `entitlements.plist` in the repo root (checked into git)

**Validation:** `just doctor` runs a four-step codesigning check:

| Check | What it validates | Fix if it fails |
|-------|-------------------|-----------------|
| Xcode CLTools | `xcode-select -p` returns a path | `xcode-select --install` |
| `codesign` binary | The tool exists in PATH | Install Xcode CLTools (see above) |
| `entitlements.plist` | The file exists and is readable | `git checkout entitlements.plist` |
| Test sign | Compiles a tiny binary + signs it with entitlements | See [troubleshooting](#codesign-fails) below |

No Apple Developer ID certificate is needed for local development -- ad-hoc signing (`--sign -`) is sufficient.

## Customizing the VM image

To add packages, AI providers, or change security policy, edit the TOML configs in `guest/config/` and rebuild. See [Customizing VM Images](./custom-images) for the workflow.

## API keys (optional)

Needed for `just full-test` (integration tests exercise real AI API calls) and interactive AI sessions inside the VM.

Create `~/.capsem/user.toml`:

```toml
[providers.anthropic]
api_key = "sk-ant-..."

[providers.google]
api_key = "AIza..."
```

## Troubleshooting

### `just doctor` fails

Install missing tools as indicated -- every `[FAIL]` line includes the fix command. Run `just _install-tools` to auto-install Rust components and cargo tools.

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
- Podman: `podman machine stop && podman machine start`
- Docker: restart Docker Desktop

### `just run` fails with "assets not found"

Run `just build-assets` first. Assets are gitignored and must be built locally.

### Cross-compile linker errors

```
ld: unknown options: --as-needed -Bstatic ...
```

The system `cc` is being used instead of `rust-lld`. Fix:

```bash
rustup component add llvm-tools
```

Verify `.cargo/config.toml` sets `linker = "rust-lld"` for both musl targets.

### VM boot hangs

- Check codesigning: `codesign -dvv target/debug/capsem 2>&1 | grep entitlements`
- Check assets exist: `ls assets/arm64/vmlinuz assets/arm64/rootfs.squashfs`
- Debug logs: `RUST_LOG=capsem=debug just run`
