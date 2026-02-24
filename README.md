# Capsem

Native macOS app that sandboxes AI agents in Linux VMs using Apple's Virtualization.framework.

Built with Rust, Tauri 2.0, Svelte 5, and TailwindCSS.

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
- Podman (`brew install podman` or [podman.io](https://podman.io/))

### Project Structure

```
crates/capsem-core/    Rust VM library (config, boot, serial, machine)
crates/capsem-app/     Tauri 2.0 binary (GUI, CLI, updater, IPC commands)
frontend/              Svelte 5 + TailwindCSS 4 + Skeleton UI
images/                VM image build tooling (Dockerfile + build.py)
assets/                Built VM assets (vmlinuz, initrd, rootfs -- gitignored)
docs/                  Architecture and security documentation
```

### Build the VM Image

The VM needs three assets: an ARM64 Linux kernel, an initrd, and a root filesystem.

```sh
podman machine init   # first time only
podman machine start
cd images && python3 build.py
```

### Install Dependencies

```sh
cd frontend && pnpm install
```

### Development Build

```sh
make run
```

This builds the Rust binary, signs it with the virtualization entitlement, and launches the app with assets from `./assets/`.

### Release Build

```sh
make release-sign
```

Produces a signed `Capsem.app` bundle at `target/release/bundle/macos/Capsem.app` with assets embedded in `Contents/Resources/`.

### Tests

```sh
cargo test --workspace
```

### Entitlements

The binary must be signed with `com.apple.security.virtualization` or Virtualization.framework calls crash at runtime. The Makefile handles this automatically.

## Documentation

- [Architecture](docs/architecture.md) -- how the system works
- [Security](docs/security.md) -- threat model, isolation guarantees, supply chain
- [Status](docs/status.md) -- milestone progress

## Auto-Update

Release builds include Tauri's updater plugin. When a new version is published to GitHub Releases, the app shows a native dialog offering to download and install the update.

## License

See [LICENSE](LICENSE).
