# Capsem

Native macOS app that sandboxes AI agents in Linux VMs using Apple's Virtualization.framework.

Built with Rust, Tauri 2.0, and Astro.

## Install

Download the latest release from [Releases](https://github.com/google/capsem/releases) and drag Capsem.app to your Applications folder.

Or build from source:

```sh
bash install.sh
```

Requires macOS 13+ on Apple Silicon.

## Usage

### GUI

```sh
open /Applications/Capsem.app
```

### CLI

Run a command inside the sandboxed Linux VM:

```sh
capsem uname -a
capsem echo hello
capsem 'ls -la /proc/cpuinfo'
```

The CLI binary lives at `/Applications/Capsem.app/Contents/MacOS/capsem`.

## Development

### Prerequisites

- macOS 13+ on Apple Silicon
- Rust via [rustup](https://rustup.rs/)
- Node.js 20+ and pnpm (`npm install -g pnpm`)
- [just](https://github.com/casey/just) (`brew install just`)
- Podman (`brew install podman` or [podman.io](https://podman.io/))

### Project Structure

```
crates/capsem-core/    Rust VM library (config, boot, serial, machine)
crates/capsem-app/     Tauri 2.0 binary (GUI, CLI, updater, IPC commands)
frontend/              Astro + xterm.js (shadow DOM web component)
images/                VM image build tooling (Dockerfile + build.py + capsem-init)
assets/                Built VM assets (vmlinuz, initrd, rootfs -- gitignored)
docs/                  Architecture and security documentation
```

### Just Commands

All build workflows use `just`. Run `just --list` to see all targets.

| Command | What it does |
|---------|-------------|
| `just build` | Build VM assets from scratch (kernel, initrd, rootfs) via Docker/Podman |
| `just repack` | Repack initrd with current `capsem-init`, rebuild binary, and boot to test (~5s) |
| `just dev` | Run the app in development mode with hot-reloading |
| `just compile` | Build the debug Rust binary (includes frontend) |
| `just sign` | Compile + codesign with virtualization entitlement |
| `just run` | Sign + run the debug binary |
| `just release` | Build the release `.app` bundle and codesign it |
| `just install` | Release build + install to `/Applications` + launch |
| `just rebuild` | Full rebuild: VM assets + app + sign + smoke test |
| `just check` | Check Rust + frontend for errors |
| `just clean` | Remove all build artifacts |

### First-Time Setup

```sh
podman machine init && podman machine start   # first time only
cd frontend && pnpm install
just build                                     # build VM assets (~10 min)
```

### Development Workflow

```sh
just dev        # hot-reloading dev server
just run        # debug build + run (no hot-reload)
just repack     # iterate on capsem-init without full asset rebuild
```

### Release

```sh
just install    # build, sign, install to /Applications, launch
```

### Testing

Testing has two layers: host-side Rust tests and in-VM diagnostics.

**Host-side (out of VM)** -- standard Rust unit and integration tests that run on macOS without booting a VM:

```sh
cargo test --workspace
```

**In-VM diagnostics** -- a pytest suite that runs inside the guest VM to verify the sandbox actually works end-to-end. It checks sandbox security (read-only rootfs, no kernel modules, no networking), unix utilities, dev runtimes (Python, Node.js, git), AI CLI availability, and file I/O workflows.

```sh
just smoke-test                       # boot VM, run all diagnostics, exit
just run                              # or boot interactively, then:
capsem-doctor                         # run all diagnostics
capsem-doctor -k sandbox              # run only sandbox tests
capsem-doctor -x                      # stop on first failure
```

The diagnostic suite lives in `images/diagnostics/` and is baked into the rootfs via `Dockerfile.rootfs`. `capsem-doctor` (aliased as `capsem-test`) is the entry point. It returns a non-zero exit code on failure, so `just smoke-test` fails the build when tests fail.

### Entitlements

The binary must be signed with `com.apple.security.virtualization` or Virtualization.framework calls crash at runtime. The justfile handles this automatically.

## Security

Capsem assumes the AI agent inside the VM is adversarial. The sandbox is hardened at every layer:

- **Hardware VM isolation** -- Apple Silicon Stage 2 page tables, no shared memory
- **Custom hardened kernel** -- compiled from source with `CONFIG_MODULES=n` (no rootkits), `CONFIG_INET=n` (no IP stack), KASLR, stack protector, FORTIFY_SOURCE. 7MB vs 30MB stock Debian. See `images/defconfig` for the full config.
- **No network interface** -- no NIC exists in the VM. DNS, HTTP, and all IP traffic are physically impossible.
- **Read-only rootfs** -- system binaries are immutable. Only `/root`, `/tmp`, and `/run` are writable (tmpfs, wiped on reboot).
- **Boot asset integrity** -- BLAKE3 hashes of kernel, initrd, and rootfs are compiled into the binary. Tampered assets are rejected before the VM boots.
- **No systemd, no services** -- PID 1 is our init script. No cron, no sshd, no background processes.

Full threat model and security analysis: **[docs/security.md](docs/security.md)**

## Documentation

- [Architecture](docs/architecture.md) -- how the system works
- [Security](docs/security.md) -- threat model, isolation guarantees, supply chain
- [Status](docs/status.md) -- milestone progress

## Auto-Update

Release builds include Tauri's updater plugin. When a new version is published to GitHub Releases, the app shows a native dialog offering to download and install the update.

## License

See [LICENSE](LICENSE).
