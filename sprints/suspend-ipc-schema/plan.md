# Suspend IPC Schema Guard Plan

## Goal

Fix the suspend failure where `capsem-service` talks to a `capsem-process` binary with the same package version but a different IPC schema hash. That mixed install currently passes version health checks and only fails when suspend opens the per-VM IPC socket.

## Approach

- Add a shared build-info surface that reports package version, protocol version, and schema hash.
- Teach installed host binary health checks to compare protocol/schema compatibility, not just the visible package version.
- Make `capsem-service` verify its configured `capsem-process` binary at startup before it can spawn incompatible VM workers.
- Keep `/version` and gateway health responses backward-compatible while exposing the richer build info.

## Files

- `crates/capsem-core/src/build_info.rs`
- `crates/capsem-core/src/lib.rs`
- `crates/capsem/src/status.rs`
- `crates/capsem/src/status/tests.rs`
- `crates/capsem-service/src/main.rs`
- `crates/capsem-service/src/startup.rs`
- `crates/capsem-process/src/main.rs`
- `crates/capsem-gateway/src/main.rs`
- `crates/capsem-tray/src/main.rs`
- `CHANGELOG.md`

## Done

- A same-version/different-schema helper binary is reported unhealthy.
- The service refuses to start with an incompatible `capsem-process`.
- Focused Rust tests and checks pass.
