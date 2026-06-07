# Audit: Transition to Next-Gen Platform

## Objective
Fastest path to `capsem-service` (daemon) and `capsem` (CLI) providing:
- `capsem start` (provision one VM)
- `capsem list` (see running VMs)
- `capsem shell` (interactive terminal in VM)

## Current State & Reuse Candidates

### `capsem-core` (Keep)
- [x] `Hypervisor` and `VmHandle` traits: solid abstraction for Apple VZ and KVM.
- [x] `VmConfig` and `VirtioFsShare`: well-defined VM parameters.
- [x] `net/mitm_proxy`: fully functional, can be embedded in `capsem-process`.
- [x] `mcp/gateway`: fully functional, can be embedded in `capsem-process`.
- [x] `session/index`: per-VM session DB logic is already isolated.
- [x] `vsock`: protocol framing for control/terminal is already there.

### `capsem-app` (Extract to `capsem-core` or new crates)
- [x] `boot_vm`: Moved to `capsem-core::vm::boot`.
- [x] `VmInstance`: Moved to `capsem-core::vm::registry` as `SandboxInstance`.
- [x] `TerminalOutputQueue`: Moved to `capsem-core::vm::terminal`.
- [x] `vsock_wiring`: Handled in `capsem-process`.

### `capsem-agent` (Keep)
- [x] PTY agent: Works well, no major changes needed.

## New Crates (All created)

### `capsem-process`
- [x] Self-contained process managing ONE VM + support services.
- [x] Communicates with `capsem-service` via UDS IPC.
- [x] Job store for async exec/file operations.
- [x] Auto-snapshot timer.

### `capsem-service` (Daemon)
- [x] Orchestrator for `capsem-process` instances.
- [x] Re-adopts orphaned processes on restart (stale PID cleanup).
- [x] UDS/HTTP API (10 endpoints).
- [x] max_concurrent_vms admission control.

### `capsem` (CLI)
- [x] 9 commands: start, stop, shell, list, status, exec, delete, info, doctor.
- [x] Shell with standalone (auto-delete) and attach modes.

### `capsem-mcp` (Host MCP Server)
- [x] 9 tools wired to service API.
- [x] Param validation and security tests.

## Progress

### Phase 1: Shared Core (Done -- 2026-04-02)
- [x] Moved `TerminalOutputQueue` to `capsem-core::vm::terminal`.
- [x] Moved `VmInstance` and `VmNetworkState` to `capsem-core::vm::registry`.
- [x] Moved `boot_vm` logic to `capsem-core::vm::boot`.

### Phase 2: The New Crates (Done -- 2026-04-02)
- [x] `capsem-service`: Daemon with UDS API and state tracking.
- [x] `capsem`: CLI with all commands.
- [x] `capsem-process`: Per-VM process that boots the VM.

### Phase 3: The Integration (Done -- 2026-04-02, commit 0c6cd8d)
- [x] `capsem-service` spawns `capsem-process` on provision.
- [x] UDS terminal bridging in `capsem-process` (IPC-based).
- [x] `capsem shell` in the CLI (standalone + attach modes).
- [x] Exec path: CLI -> service -> process -> guest with job correlation.
- [x] File read/write via IPC.

### Phase 4: MCP + Snapshots (Done -- post Sprint 1)
- [x] `capsem-mcp` host server: 9 tools, all backed by real endpoints.
- [x] Snapshot infrastructure: dual-pool, APFS clonefile/reflink, auto-timer, 7 gateway tools.
- [x] Telemetry: snapshot_events table in session.db.
- [x] VM state machine: 8 states (Created -> Booting -> VsockConnected -> Handshaking -> Running -> ShuttingDown -> Stopped + Error).
- [x] Host state validation with zero-trust message checking.

## What's next

See restructured tracker: `sprints/next-gen/tracker.md` (23 sprints, S1-S4 done).

## Risks & Blockers
- **MacOS Main Thread**: Apple VZ *must* run on the main thread. Handled by capsem-process being the VM owner.
- **Terminal resizing**: Propagated from CLI through service IPC to agent. Working.
