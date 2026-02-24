# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-02-24

### Added
- PTY-over-vsock terminal communication replacing serial broadcast channel
- Guest PTY agent (`capsem-pty-agent`) for high-throughput terminal I/O with full PTY support
- Terminal resize support (`stty size` reflects window dimensions)
- vsock control channel with MessagePack framing for structured commands (resize, heartbeat)
- Kernel vsock support (`CONFIG_VSOCKETS`, `CONFIG_VIRTIO_VSOCKETS`)
- Multi-VM-ready app state architecture (`vm_id`-keyed `HashMap`)
- Output coalescing (10ms/64KB) to prevent frontend IPC saturation
- Boot-time command execution via vsock (`Exec`/`ExecDone` control messages)
- CLI mode (`capsem "command"`) routes commands through vsock PTY agent with exit code propagation

### Changed
- Terminal input now routes through vsock when connected, falling back to serial
- Guest init script (`capsem-init`) launches PTY agent instead of direct bash/setsid
- CLI mode rewritten from serial I/O to vsock-based execution with proper exit codes
- `just repack` now cross-compiles and bundles the PTY agent into the initrd for fast iteration
- Serial forwarding stops once vsock connects, eliminating duplicate output
- M5 redesigned: zero-trust network boundaries with SNI proxy domain filtering, AI provider domain blocking, and real-time file telemetry via fanotify
- M6 redesigned: active AI audit gateway with 9-stage event lifecycle (PII scrubbing, tool call interception, secret scanning), replaces passive proxy approach
- M7 redesigned: hybrid MCP architecture -- local tools run sandboxed in-VM, remote tools route through host gateway with credential injection
- M8 redesigned: per-session audit databases with zstd-compressed blobs, OverlayFS config write-back, enterprise observability (Prometheus, OTLP, corporate policy via MDM)

### Fixed
- Shell prompt not appearing after command execution (stderr was redirected to /dev/hvc0, sending readline prompt through buffered serial path instead of vsock PTY)

### Security
- Removed serial console fallback: missing PTY agent halts boot instead of opening an unprotected shell
- Replaced scattered `unsafe { File::from_raw_fd }` + `mem::forget` with centralized `borrow_fd` helper using `ManuallyDrop`
- Added T13 threat (AI Traffic Audit Bypass) documenting the enforcement chain: iptables -> vsock bridge -> SNI proxy -> audit gateway
- Updated T3 (Data Exfiltration) with fswatch telemetry, PII engine, and secret scanning mitigations
- Updated T5 (Credential Theft) with gateway key injection and PII scrubbing on model calls
- Updated T11 (Network Exfiltration) with AI domain blocking at SNI proxy and 9-stage lifecycle enforcement
- Added Corporate Security Profile section with MDM-distributable policy.toml for enterprise deployments

## [0.2.0] - 2026-02-24

### Added
- blake3 integrity checking of VM assets (B3SUMS)
- Kernel hardening configuration for guest VM
- Proper terminal signal handling (setsid for controlling tty)
- Boot-up tracing spans for timing diagnostics
- Utility helpers for VM lifecycle

### Fixed
- Utility module fixes

## [0.1.0] - 2026-02-23

### Added
- Native macOS app using Tauri 2.0 with Astro frontend
- Linux VM sandboxing via Apple Virtualization.framework
- Virtio serial console with bidirectional I/O (xterm.js <-> guest /dev/hvc0)
- Custom capsem-init (PID 1) with chroot and setsid
- Docker/Podman-based VM asset build pipeline (kernel, initrd, rootfs)
- `just` task runner workflows (build, repack, dev, run, release, install)
- Codesigning with com.apple.security.virtualization entitlement
- xterm.js terminal web component
- Tauri auto-updater plugin integration

### Changed
- Complete rewrite from Python proxy architecture (v1) to native Rust/Tauri VM app
