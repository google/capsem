# Meta Sprint: VM Lifecycle (Shutdown, Suspend, Resume, Identity)

Unified sprint for guest-initiated lifecycle control. Adds system binaries inside the VM (`shutdown`, `halt`, `poweroff`, `suspend`) that route through the service's existing code paths, plus the quiescence protocol and Apple VZ suspend/resume that those binaries depend on.

Covers next-gen S5 (quiescence), S7 (Apple VZ suspend/resume), and new work (guest identity, system binaries).

## Status

| Sub-Sprint | Name | Status | Depends On |
|-----------|------|--------|-----------|
| T0 | Protocol + Identity | Done | -- |
| T1 | capsem-sysutil binary | Done | T0 |
| T2 | Shutdown flow (end-to-end) | Done | T0, T1 |
| T3 | VmHandle trait + Apple VZ pause/save | Done | -- |
| T4 | Quiescence protocol | Done | T0, T3 |
| T5 | Agent reconnect | Done | T0 |
| T6 | Suspend/Resume service flow | Done | T3, T4, T5 |
| T7 | Guest-initiated suspend | Done | T1, T6 |
| T8 | CLI + MCP tools | Done | T6 |
| T9 | Testing gate | Done | All |

## Phases

**Phase 1 (T0-T2): Shutdown + Identity** -- standalone value, no hypervisor changes.
**Phase 2 (T3-T5): Hypervisor + Quiescence + Reconnect** -- infrastructure for suspend/resume.
**Phase 3 (T6-T8): Suspend/Resume end-to-end** -- wired through service, CLI, MCP, guest binary.
**Phase 4 (T9): Testing gate** -- integration tests, capsem-doctor, smoke.

## Critical Path

```
T0 (protocol) -----> T1 (sysutil) -----> T2 (shutdown e2e)
     |                    |
     |                    +----------------------------> T7 (guest suspend)
     |                                                      ^
     +---> T4 (quiescence) ---> T6 (suspend/resume svc) ---+---> T8 (CLI+MCP)
     |         ^                    ^
     |         |                    |
T3 (VmHandle) +                    |
     |                              |
     +---> T5 (agent reconnect) ---+
```

T0+T3 can start in parallel (no dependencies between them).
T1 can start once T0 lands.
T4 needs both T0 (protocol messages) and T3 (pause/save on trait).
T5 (agent reconnect) can start once T0 lands (needs protocol, not hypervisor).
T6 needs T3+T4+T5.
Phase 1 (T0-T2) delivers value immediately -- guest shutdown works without suspend.

## Relevant Just Recipes

```bash
just build          # Build all host binaries
just run            # Build + repack initrd + boot VM (fast iteration for guest binaries)
just test           # ALL tests
just smoke          # Quick integration tests
just run "capsem-doctor"  # In-VM diagnostics
```

## Key Files (entry points for each sub-sprint)

| Area | File |
|------|------|
| Host-guest protocol | `crates/capsem-proto/src/lib.rs` |
| Service-process IPC | `crates/capsem-proto/src/ipc.rs` |
| Guest agent | `crates/capsem-agent/src/main.rs` |
| Per-VM process | `crates/capsem-process/src/main.rs` |
| Service daemon | `crates/capsem-service/src/main.rs` |
| VmHandle trait | `crates/capsem-core/src/hypervisor/mod.rs` |
| Apple VZ machine | `crates/capsem-core/src/hypervisor/apple_vz/machine.rs` |
| Apple VZ handle | `crates/capsem-core/src/hypervisor/apple_vz/mod.rs` |
| Boot config | `crates/capsem-core/src/vm/boot.rs` |
| Init script | `guest/artifacts/capsem-init` |
| CLI | `crates/capsem/src/main.rs` |
| MCP server | `crates/capsem-mcp/src/main.rs` |
| Initrd packing | `justfile` (`_pack-initrd` recipe) |
