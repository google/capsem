# Crate and Privilege Model

Read this reference before moving responsibilities between crates, changing
capsem-process permissions or environment, changing session/socket/share
boundaries, or changing MITM CA-key handling.

## Crate architecture

Reusable code belongs to the lowest-dependency crate that owns its domain.
Sharing alone is not a reason to put code in `capsem-core`.

- **`capsem-foundation`**: dependency-light host primitives: paths, UDS HTTP,
  polling, telemetry/log setup, and IPC handshakes.
- **`capsem-archive`**: block-compressed body archive for session ledgers.
  Pure Rust (`miniz_oxide`); SQLite keeps the index, this crate keeps the
  bytes, and every block is blake3-verified before a body is returned.
- **`capsem-ledger`**: confined session-ledger worker and the sole runtime
  executor of body archive codecs.
- **`capsem-telemetry`**: the one owner of every `metrics`-facade metric name,
  with its kind, unit and description. Depends only on `metrics`; emitting
  crates import names from it and never spell a metric name themselves.
- **`capsem-assets`**: asset manifests, compatibility, download, resolution,
  and verification.
- **`capsem-config`**: config types, parsing, validation, resolution, and
  provider/MCP identity.
- **`capsem-credentials`**: credential provider contracts and durable store.
- **`capsem-api`**: gateway request/response types and OpenAPI schema, shared by clients without service runtime access.
- **`capsem-sdk`** (`sdk/rust`): async HTTP gateway clients that reuse `capsem-api` DTOs; explicit URL/token, no filesystem discovery or service/core dependencies.
- **`capsem-proto`**: shared host/guest and service/process wire contracts.
- **`capsem-core`**: VM, hypervisor, security-engine, host-network, MCP runtime,
  and session/image domain logic.
- **`capsem-logger`**: session DB schema, queries, storage, and async writer.
- **`capsem-guard`**: parent-watch and singleton-flock lifecycle primitives.
- **`capsem-service`**: daemon HTTP/UDS API and VM-process orchestration.
- **`capsem-process`**: per-VM boot, vsock, IPC, and job runtime; still holds
  only the paths and descriptors granted for its session and generation.
- **`capsem-proxy`**: confined HTTP/DNS worker for VM and standalone proxy
  traffic; upstream, ledger, credential, and MCP authority arrives by grant.
- **`capsem`**: CLI client; HTTP/UDS to the service, including shell streams.
- **`capsem-tui`**: terminal control UI over the gateway API.
- **`capsem-admin`**: runtime image build plus asset, release, and config validation.
- **`@capsem/mcp`** (`mcp/typescript`): separately installed host MCP server;
  typed SDK client over authenticated gateway HTTP with no native runtime privilege.
- **`capsem-router`**: Seatbelt/seccomp-confined companion with two jobs from one binary. Per VM owner, a TCP relay for published host ports only (connected descriptor pairs over a private, bounded grant channel). Per named network, `--network`: that network's layer-2 switch. The service plugs each attached VM's cable (a duplicate of that cable's VSOCK 5009 stream) into it; it forwards ethernet frames on MAC only, floods broadcast under a cap, and carries every protocol. No service control socket, ambient file access, listener acceptance, or virtualization entitlement in either job.
- **`capsem-network`**: the cable's frame codec (`u16` length + ethernet frame) and the switch's MAC forwarding table as pure code. The kernel inside each guest does ARP, IP and everything above; no host process parses past a frame's MAC addresses.
- **`capsem-mcp-aggregator`**: low-privilege external-MCP subprocess manager.
- **`capsem-mcp-builtin`**: built-in HTTP MCP tools.
- **`capsem-gateway`**: authenticated TCP-to-UDS HTTP/WebSocket gateway.
- **`capsem-app`**: thin Tauri webview shell pointing at the gateway.
- **`capsem-tray`**: system tray status and quick actions through the gateway.
- **`capsem-agent`**: musl guest PTY, network, DNS, MCP, and sysutil binaries.
- **`capsem-bench`**: host/guest benchmark harness and collectors.
- **`capsem-mock-server`**: hermetic HTTP/TLS/WebSocket test upstream.

## Process privilege model

The service launches each VM owner, proxy, and ledger worker with a cleared
environment, private control descriptors, a generation-bound protocol grant,
and an OS sandbox. Linux uses Landlock plus seccomp; macOS uses a parameterized
Seatbelt profile. Each worker runs a startup attestation that proves ambient
file, socket, process-execution, and signal authority was denied before it
accepts workload data.

The VM owner retains only its session paths, verified boot assets, inherited
VM resources, and explicit service grants. It cannot open the ledger or dial
upstream sockets. `capsem-proxy` receives connected DNS/TCP descriptors plus
credential, MCP, and ledger channels. `capsem-ledger` receives the ledger files
and typed producer channels; durable service-owned commitment checkpoints
anchor the sequence outside that worker.

Socket permissions remain 0600 and session directories 0700 as defense in
depth. The guest sees only `session_dir/guest/` through VirtioFS; the writable
system overlay, serial log, ledger, and worker state remain outside the share.
Runtime rootfs assets are read-only and injected guest binaries are mode 0555.

The service and kernel remain trusted. A compromised producer can lie about an
observation before committing it, and a crash can lose an unanchored tail.
Anchored alteration, substitution, omission, reordering, and stale-generation
reuse are detected when the service opens the ledger.

### MITM CA key transparency
The MITM proxy CA private key (`crates/capsem-core/resources/ca/capsem-ca.key`) is committed to the repo and embedded at compile time. This is intentional -- capsem's network interception exists for user visibility into what AI agents do, not for secrecy. The CA is only trusted inside capsem's own air-gapped VMs and has zero trust outside them. A public key lets anyone verify there is no hidden interception. Per-installation key generation would reduce transparency.
