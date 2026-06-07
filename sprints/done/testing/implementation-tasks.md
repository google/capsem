# Implementation Tasks

Things the coding team must build or fix for the test sprints to pass. Organized by crate.

## CLI (crates/capsem/)

- [ ] Add `exec <id> <command>` subcommand -- POST /exec/{id} with {command}
- [ ] Add `delete <id>` subcommand -- DELETE /delete/{id}
- [ ] Add `info <id>` subcommand -- GET /info/{id}, print JSON
- [ ] Implement `stop <id>` -- currently prints "Not yet implemented", needs DELETE /delete/{id} or a graceful stop endpoint
- [ ] Implement `status <id>` -- currently prints "Not yet implemented", needs GET /info/{id}
- [ ] Add `doctor` subcommand -- start VM + exec capsem-doctor + collect session.db + print results + delete VM
- [ ] Add `CAPSEM_RUN_DIR` env var support (match capsem-mcp pattern)

## Service (crates/capsem-service/)

- [ ] Add `max_concurrent_vms` enforcement -- check instances.len() on provision, reject with clear error when at limit
- [ ] Add config key `vm.resources.max_concurrent_vms` (range 1-20, default 10)
- [ ] Add `POST /reload-config` endpoint -- reload policies from disk, swap into all running VM instances
- [ ] Hot-reload must update: network policy, domain policy, MCP policy for every running VM
- [ ] Ensure per-VM session.db is created in `{session_dir}/session.db`
- [ ] Ensure stale instance cleanup handles dead PIDs (check if process alive before routing IPC)
- [ ] Reject duplicate VM names on provision with clear error message

## Process (crates/capsem-process/)

- [ ] Ensure each VM process creates its own MCP gateway with independent policy state (Arc<RwLock<Policy>>)
- [ ] Ensure each VM process creates its own DbWriter for session.db in session_dir
- [ ] Ensure snapshot scheduler is per-VM with configurable auto_interval from provision request
- [ ] Accept per-VM config via provision (cpu/ram/snapshot_interval/policy overrides)

## Config (config/defaults.toml)

- [ ] Add `vm.resources.max_concurrent_vms` setting (range 1-20, default 10)
- [ ] Support per-VM config overrides passed in ProvisionRequest
- [ ] Config reload triggers immediate policy swap in all running VMs via RwLock

## Proto (crates/capsem-proto/)

- [ ] Consider adding per-VM config message in ServiceToProcess for forwarding settings at boot

## Just Recipes (justfile)

- [ ] `just run-service` -- build + sign + start capsem-service in background, print PID
- [ ] `just run` -- run-service + capsem start + capsem shell (replaces old capsem-app flow)
- [ ] `just ui` -- run-service + cargo tauri dev
- [ ] `just test-service` -- build + pytest tests/capsem-service/ -m integration
- [ ] `just test-cli` -- build + pytest tests/capsem-cli/ -m integration
- [ ] `just test-session` -- build + pytest tests/capsem-session/ -m session
- [ ] `just test-snapshots` -- build + pytest tests/capsem-snapshots/ -m snapshot
- [ ] `just test-isolation` -- build + pytest tests/capsem-isolation/ -m isolation
- [ ] `just test-security` -- build + pytest tests/capsem-security/ -m security
- [ ] `just test-config` -- build + pytest tests/capsem-config/ -m config
- [ ] `just test-bootstrap` -- pytest tests/capsem-bootstrap/ -m bootstrap
- [ ] `just test-stress` -- build + pytest tests/capsem-stress/ -m stress
- [ ] `just test-all` -- all of the above
- [ ] `just coverage` -- cargo llvm-cov --html + open report
- [ ] Update `just smoke-test` to use capsem-cli + capsem-service
- [ ] Update `just full-test` to include test-mcp, test-service, test-security

## Codecov (codecov.yml)

- [ ] Add `service` component: `crates/capsem-service/src/**`, `crates/capsem-process/src/**`
- [ ] Add `cli` component: `crates/capsem/src/**`
- [ ] Add `mcp-server` component: `crates/capsem-mcp/src/**`
- [ ] Set coverage targets: 60% for new crates, maintain 80% patch overall
