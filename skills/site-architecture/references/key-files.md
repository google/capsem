# Key Source Files

## Guest

- `images/capsem-init` -- PID 1 init script. Sets up networking, mounts, launches daemons.
- `images/capsem-bashrc` -- guest shell config (baked into rootfs)
- `images/README.md` -- guest VM environment docs (packages, banner, tips, AI status)
- `crates/capsem-agent/src/main.rs` -- PTY agent (vsock bridge, cross-compiled)
- `crates/capsem-agent/src/net_proxy.rs` -- TCP-to-vsock relay (cross-compiled)

## Network

- `crates/capsem-core/src/net/mitm_proxy.rs` -- async MITM proxy (rustls + hyper): TLS termination, HTTP inspection, upstream bridging
- `crates/capsem-core/src/net/cert_authority.rs` -- CA loader + on-demand domain cert minting with RwLock cache
- `crates/capsem-core/src/net/http_policy.rs` -- method+path policy engine
- `crates/capsem-core/src/net/domain_policy.rs` -- domain allow/block evaluation
- `crates/capsem-core/src/net/sni.rs` -- SNI parser for TLS ClientHello
- `crates/capsem-core/src/net/policy_config.rs` -- user.toml + corp.toml merge logic

## VM

- `crates/capsem-core/src/vm/machine.rs` -- VZVirtualMachine wrapper (serial + vsock + VirtioFS)
- `crates/capsem-core/src/vm/config.rs` -- VmConfig builder (VirtioFsShare, block devices, validation)
- `crates/capsem-core/src/vm/serial.rs` -- serial console pipe setup (boot logs)
- `crates/capsem-core/src/vm/vsock.rs` -- vsock manager, control messages, coalescing buffer
- `crates/capsem-core/src/fs_monitor.rs` -- host-side FSEvents file monitor
- `crates/capsem-core/src/auto_snapshot.rs` -- rolling auto-snapshot scheduler (APFS clonefile ring buffer)

## App

- `crates/capsem-app/src/commands.rs` -- Tauri IPC commands (serial_input, vm_status, terminal_resize, net_events)
- `crates/capsem-app/src/state.rs` -- per-VM state (serial + vsock fds)

## Config

- `config/defaults.toml` -- settings registry (embedded at compile time)
- `config/capsem-ca.key` + `config/capsem-ca.crt` -- static MITM CA keypair (ECDSA P-256)

## Frontend

- `frontend/src/components/capsem-terminal.ts` -- xterm.js web component
- `frontend/src/lib/components/App.svelte` -- root layout
- `frontend/src/lib/api.ts` -- typed Tauri IPC wrappers with auto-mock fallback
- `frontend/src/lib/mock.ts` -- fake data for browser dev mode
- `frontend/src/lib/types.ts` -- TS types mirroring Rust IPC structs

## MCP

- `crates/capsem-core/src/mcp/file_tools.rs` -- MCP built-in tools: list_changed_files, revert_file
