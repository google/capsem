# Key Source Files

## Guest

- `guest/artifacts/capsem-init` -- PID 1 init script. Sets up networking, mounts, launches daemons.
- `guest/artifacts/capsem-bashrc` -- guest shell config (baked into rootfs)
- `config/profiles/<id>/` -- profile-owned packages, rules, MCP declarations, tips, and root seed files
- `crates/capsem-agent/src/main.rs` -- PTY agent (vsock bridge, cross-compiled)
- `crates/capsem-agent/src/net_proxy.rs` -- TCP-to-vsock relay (cross-compiled)

## Network

- `crates/capsem-core/src/net/mitm_proxy.rs` -- async MITM proxy (rustls + hyper): TLS termination, HTTP inspection, upstream bridging
- `crates/capsem-core/src/net/cert_authority.rs` -- CA loader + on-demand domain cert minting with RwLock cache
- `crates/capsem-core/src/security_engine/` -- shared CEL rule/plugin/decision rail over `SecurityEvent`
- `crates/capsem-core/src/net/sni.rs` -- SNI parser for TLS ClientHello

## VM

- `crates/capsem-core/src/vm/machine.rs` -- VZVirtualMachine wrapper (serial + vsock + VirtioFS)
- `crates/capsem-core/src/vm/config.rs` -- VmConfig builder (VirtioFsShare, block devices, validation)
- `crates/capsem-core/src/vm/serial.rs` -- serial console pipe setup (boot logs)
- `crates/capsem-core/src/vm/vsock.rs` -- vsock manager, control messages, coalescing buffer
- `crates/capsem-core/src/fs_monitor.rs` -- host-side FSEvents file monitor
- `crates/capsem-core/src/auto_snapshot.rs` -- rolling auto-snapshot scheduler (APFS clonefile ring buffer)

## Gateway

- `crates/capsem-gateway/src/main.rs` -- TCP listener, router setup, health endpoint, graceful shutdown
- `crates/capsem-gateway/src/auth.rs` -- Bearer token auth middleware, runtime file lifecycle (token/port/pid)
- `crates/capsem-gateway/src/proxy.rs` -- UDS reverse proxy (method/header/body forwarding, 10MB limit, 30s timeout)
- `crates/capsem-gateway/src/status.rs` -- Aggregated status with 2s thundering-herd-safe cache
- `crates/capsem-gateway/src/terminal.rs` -- WebSocket relay from TCP to per-VM UDS for terminal I/O

## App (thin Tauri webview shell)

- `crates/capsem-app/src/main.rs` -- Tauri setup, gateway URL, 2 IPC commands (open_url, check_for_app_update)
- `crates/capsem-app/tauri.conf.json` -- Tauri config (bundle targets, updater endpoint, entitlements)

## Config

- `config/settings/ui-metadata.toml` -- settings UI metadata (embedded at compile time)
- `crates/capsem-config/src/` -- config types, validation, provider/MCP identity, and resolution
- `crates/capsem-credentials/src/` -- provider contracts and durable credential storage
- `crates/capsem-assets/src/asset_manager.rs` -- asset resolution, download, and verification
- `crates/capsem-assets/src/manifest_compat.rs` -- manifest compatibility contract
- `crates/capsem-core/resources/ca/capsem-ca.key` + `crates/capsem-core/resources/ca/capsem-ca.crt` -- static MITM CA keypair (ECDSA P-256)

## Frontend

- `web/app/src/lib/components/terminal/TerminalFrame.svelte` -- xterm.js terminal frame
- `web/app/src/lib/components/shell/App.svelte` -- root layout
- `web/app/src/lib/api.ts` -- HTTP client for explicit gateway API routes
- `web/app/src/lib/mock-settings.ts` -- fake settings data for browser dev mode
- `web/app/src/lib/types.ts` -- TS types mirroring Rust IPC structs

## MCP

- `crates/capsem-mcp/src/` -- host MCP server and service-facing tool handlers
- `crates/capsem-mcp-aggregator/src/` -- external-server lifecycle and transport
- `crates/capsem-mcp-builtin/src/main.rs` -- built-in HTTP and file/snapshot tools
- `crates/capsem-core/src/mcp/` -- VM/session-side MCP runtime integration

## Shared host plumbing

- `crates/capsem-foundation/src/paths.rs` -- canonical host paths and test redirection
- `crates/capsem-foundation/src/uds.rs` -- HTTP over Unix-domain sockets
- `crates/capsem-foundation/src/poll.rs` -- bounded asynchronous polling
- `crates/capsem-proto/src/` -- shared wire types and framing contracts
