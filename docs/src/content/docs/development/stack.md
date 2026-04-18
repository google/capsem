---
title: Life of a Build
description: What gets built, by what tools, and in what order -- from clone to running VM.
sidebar:
  order: 5
---

When you run `just run`, Capsem cross-compiles guest binaries, repacks the initrd, builds the host binaries, codesigns them, and boots a VM -- all in ~10 seconds. This page explains what each stage produces and which tools do the work.

## The build pipeline

```mermaid
flowchart TD
    subgraph stage1["1. Guest binaries"]
        CARGO_CROSS["cargo build --target\naarch64-unknown-linux-musl"]
        AGENT["capsem-pty-agent"]
        NETPROXY["capsem-net-proxy"]
        MCP["capsem-mcp-server"]
        SYSUTIL["capsem-sysutil"]
        CARGO_CROSS --> AGENT & NETPROXY & MCP & SYSUTIL
    end

    subgraph stage2["2. Initrd repack"]
        INITRD_IN["initrd.img\n(from build-assets)"]
        SCRIPTS["capsem-init + doctor\n+ bench + snapshots"]
        REPACK["cpio + gzip repack"]
        INITRD_IN --> REPACK
        AGENT & NETPROXY & MCP & SYSUTIL --> REPACK
        SCRIPTS --> REPACK
        REPACK --> INITRD_OUT["initrd.img\n(repacked)"]
    end

    subgraph stage3["3. Host binaries"]
        PNPM["pnpm install + astro build"]
        DIST["frontend/dist/"]
        CARGO_HOST["cargo build\n(6 host binaries)"]
        PNPM --> DIST --> CARGO_HOST
        CARGO_HOST --> SIGN["codesign\n(com.apple.security.virtualization)"]
    end

    subgraph stage0["0. VM images (first-time only)"]
        TOML["guest/config/*.toml"]
        BUILDER["capsem-builder\n(Python CLI)"]
        DOCKER["Docker (via Colima)"]
        TOML --> BUILDER --> DOCKER
        DOCKER --> VMLINUZ["vmlinuz"]
        DOCKER --> ROOTFS["rootfs.squashfs"]
        DOCKER --> INITRD_BASE["initrd.img (base)"]
    end

    INITRD_BASE -.-> INITRD_IN

    subgraph stage4["4. Boot"]
        SIGN --> BOOT["capsem-service\n+ capsem-process"]
        INITRD_OUT --> BOOT
        VMLINUZ --> BOOT
        ROOTFS --> BOOT
        BOOT --> VM["Linux VM running"]
    end
```

## Stage 1: Guest binaries (compilation)

The guest agent crate (`crates/capsem-agent/`) produces four binaries that run inside the Linux VM, statically linked with musl:

| Binary | Purpose | Target |
|--------|---------|--------|
| `capsem-pty-agent` | Bridges terminal I/O over vsock | `aarch64-unknown-linux-musl` / `x86_64-unknown-linux-musl` |
| `capsem-net-proxy` | Relays HTTPS to host MITM proxy over vsock | same |
| `capsem-mcp-server` | MCP tool relay over vsock | same |
| `capsem-sysutil` | Lifecycle multi-call (shutdown/halt/poweroff/reboot/suspend) | same |

On **macOS**, `cross_compile_agent()` delegates to `container_compile_agent()` which builds natively inside a Linux container (docker). Per-arch named volumes (`capsem-agent-target-{arch}`) cache build artifacts. No host cross-compile toolchain needed.

On **Linux** (CI), cargo builds directly with the musl target. The linker config in `.cargo/config.toml` uses `rust-lld`:

```toml
[target.aarch64-unknown-linux-musl]
linker = "rust-lld"

[target.x86_64-unknown-linux-musl]
linker = "rust-lld"
```

### Verifying the full Linux build locally

`just cross-compile [arch]` builds everything in a container: agent binaries, frontend, and all host binaries (deb package). This catches system dep issues before CI.

```bash
just cross-compile           # Build for host arch (arm64 on Apple Silicon)
just cross-compile x86_64    # Build x86_64 deb
```

## Stage 2: Initrd repack

The initrd is a gzipped cpio archive that the kernel unpacks into RAM at boot. The `_pack-initrd` recipe:

1. Extracts the base initrd (produced by `just build-assets`)
2. Copies in the freshly cross-compiled guest binaries (chmod 555, read-only)
3. Copies in shell scripts: `capsem-init` (PID 1), `capsem-doctor`, `capsem-bench`, `snapshots`
4. Repacks with `cpio + gzip`
5. Regenerates BLAKE3 checksums (`B3SUMS` + `manifest.json`)

This is why `just run` is fast (~10s) -- it only rebuilds what changed, not the full rootfs.

## Stage 3: Host binaries

This stage has two parts: the frontend build and the Rust compilation.

### Frontend (`pnpm build`)

The UI lives in `frontend/` and is built by pnpm. The build chain:

1. **pnpm install** -- installs npm dependencies (Astro, Svelte, Tailwind, Preline, xterm.js, LayerChart, sql.js)
2. **astro build** -- compiles `.astro` and `.svelte` files into static HTML/JS/CSS in `frontend/dist/`
3. The built frontend is served by capsem-gateway over HTTP (and bundled into capsem-app as offline fallback)

The frontend stack:

| Technology | Role |
|------------|------|
| [Astro 5](https://astro.build) | Static site generator -- page routing, builds the app shell |
| [Svelte 5](https://svelte.dev) | Reactive components -- terminal view, stats charts, settings panels |
| [Tailwind v4](https://tailwindcss.com) + [Preline](https://preline.co) | Styling -- utility classes + themed CSS-only component library |
| [xterm.js 6](https://xtermjs.org) | Terminal emulator -- renders the in-VM shell |
| [LayerChart 2](https://layerchart.com) | Charts -- session stats, cost tracking (D3-based Svelte library) |
| [sql.js](https://sql.js.org) | SQLite in the browser -- queries session DBs client-side |

For frontend iteration without booting a VM, use `just ui` (Astro dev server with mock data on port 5173).

### Rust compilation (`cargo build`)

The Rust workspace produces multiple binaries. Six host binaries and the Tauri desktop app:

| Crate | Binary | Role |
|-------|--------|------|
| `capsem-core` | (lib) | All business logic: VM config, boot, vsock, MITM proxy, MCP gateway, network policy, telemetry |
| `capsem-service` | `capsem-service` | Background daemon: Axum HTTP over UDS, VM lifecycle |
| `capsem-process` | `capsem-process` | Per-VM: boots VM, bridges vsock, manages jobs |
| `capsem` | `capsem` | CLI: HTTP over UDS to service |
| `capsem-mcp` | `capsem-mcp` | MCP server: stdio, bridges AI agent tool calls to service |
| `capsem-gateway` | `capsem-gateway` | HTTP gateway: TCP:19222, proxies to service, WebSocket terminal |
| `capsem-tray` | `capsem-tray` | System tray: polls gateway, shows VM status |
| `capsem-app` | `capsem-app` | Thin Tauri webview: points at gateway, bundled frontend fallback |
| `capsem-proto` | (lib) | Shared protocol types (host-guest, service-process IPC) |
| `capsem-logger` | (lib) | Session DB schema and async writer (SQLite) |

On macOS, all binaries must be codesigned with the `com.apple.security.virtualization` entitlement or Virtualization.framework crashes. The justfile handles this automatically via the `_sign` recipe.

## Stage 4: Boot

The service loads three assets from `~/.capsem/assets/v{VERSION}/` (installed) or `assets/{arch}/` (development):

| Asset | Produced by | What it is |
|-------|-------------|------------|
| `vmlinuz` | `just build-assets` | Custom Linux kernel (no modules, no IP stack, 7MB) |
| `initrd.img` | `just run` (repacked each time) | Guest binaries + init scripts |
| `rootfs.squashfs` | `just build-assets` | Debian bookworm base + AI CLIs + tools |

Boot sequence: capsem-service spawns capsem-process, which loads the kernel + initrd into a VM. `capsem-init` (PID 1) sets up overlayfs, air-gapped networking, and launches the PTY agent + net proxy + MCP server + sysutil. The host connects over vsock.

## VM image builds (`just build-assets`)

The slow path (~10 min, first-time only). The [capsem-builder](/architecture/build-system/) Python CLI reads TOML configs from `guest/config/` and produces kernel + rootfs via Docker.

```bash
uv run capsem-builder build guest/ --arch arm64    # build everything
uv run capsem-builder validate guest/               # lint configs
uv run capsem-builder doctor guest/                  # check prerequisites
```

### Container runtime

The builder needs Docker.

**macOS** -- Docker runs inside a Colima VM. Minimum 4GB RAM, recommended 8GB.

```bash
# Colima setup (recommended on macOS)
brew install colima docker
colima start --vm-type vz --vz-rosetta --memory 8 --cpu 8
```

**Linux** -- Docker runs natively, no memory tuning needed.

```bash
# Debian/Ubuntu
sudo apt install docker.io
```

## CI release pipeline

When a `vX.Y.Z` tag is pushed, the release workflow runs. Jobs are parallelized to minimize wall-clock time (~18 min vs ~45 min sequential).

```mermaid
flowchart LR
    PF["preflight\n(macos-14, 30s)"]
    BA["build-assets\n(arm64 + x86_64\nubuntu, 10 min)"]
    T["test\n(macos-14, 8 min)"]
    BM["build-app-macos\n(macos-14, 15 min)"]
    BL["build-app-linux\n(arm64 + x86_64\nubuntu, 10 min)"]
    CR["create-release\n(ubuntu, 2 min)"]

    PF --> BA & T
    PF --> BM & BL
    BA --> BM & BL
    T --> CR
    BM --> CR
    BL --> CR
```

| Job | Runner | Produces |
|-----|--------|----------|
| `preflight` | macos-14 | Validates Apple cert, Tauri key, notarization creds |
| `build-assets` | ubuntu arm64 + x86_64 | vmlinuz, initrd.img, rootfs.squashfs per arch |
| `test` | macos-14 | Unit tests + coverage, frontend check, audit |
| `build-app-macos` | macos-14 | DMG (codesigned + notarized), host binaries, latest.json |
| `build-app-linux` | ubuntu arm64 + x86_64 | deb (both arches), latest.json |
| `create-release` | ubuntu | Merges latest.json, signs manifest, creates GitHub release |

**Key design decisions:**
- `test` runs in parallel with `build-assets` and app builds -- it gates `create-release` but doesn't block compilation
- arm64 Linux produces `.deb` only
- Each platform's `latest.json` is merged in `create-release` for the Tauri auto-updater

### Local vs CI

`just cross-compile` builds the Linux binaries inside a container and catches most issues, but the environments differ:

| Aspect | Local (container) | CI (bare runner) |
|--------|-------------------|------------------|
| Base | `rust:bookworm` | `ubuntu-24.04` |
| Node | nodesource script | `actions/setup-node` |
| Volumes | none (clean build) | none (fresh runner) |

## Tools summary

Everything below is checked by `bootstrap.sh` and `just doctor`. You don't need to install these manually -- the bootstrap script tells you exactly what's missing.

| Tool | What it does in the build |
|------|--------------------------|
| Rust (stable) | Compiles host + guest binaries |
| `rust-lld` | Linker for musl cross-compilation |
| just | Task runner -- single entry point for all workflows |
| Node.js 24+ / pnpm | Builds the Astro + Svelte frontend |
| Python 3.11+ / uv | Runs capsem-builder (image builds, schema generation) |
| Docker (via Colima on macOS) | Container runtime for kernel + rootfs builds |
| cargo-llvm-cov | Code coverage (`just test`) |
| cargo-audit | Dependency vulnerability scanning |
| cargo-tauri | Tauri CLI for desktop app builds |
| b3sum | BLAKE3 checksums for asset integrity |
| codesign (macOS) | Signs binaries with virtualization entitlement |
