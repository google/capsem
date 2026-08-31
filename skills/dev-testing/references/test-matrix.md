# Test Matrix: What Runs Where

Reference for /dev-testing: per-crate Rust CI matrix and the Python integration suite tier map (PR CI vs smoke vs full gate).

## Test matrix: what runs where

### Rust crate CI matrix

Test counts are deliberately not copied here: the workspace runner and LCOV
artifact are the live inventory. Every Cargo workspace member must appear in
this table, enforced by `tests/citadel/test_rust_workspace_documentation.py`.

| Crate | Role | CI macOS | CI Linux | Fast source | Full |
|-------|------|:--------:|:--------:|:-----------:|:----:|
| `capsem-foundation` | Host primitives | Yes | Compile/no-run | Clippy | Yes |
| `capsem-assets` | Asset lifecycle | Yes | Compile/no-run | Clippy | Yes |
| `capsem-config` | Config contracts | Yes | Compile/no-run | Clippy | Yes |
| `capsem-credentials` | Credential contracts/store | Yes | Compile/no-run | Clippy | Yes |
| `capsem-proto` | Wire contracts | Yes | Compile/no-run | Clippy | Yes |
| `capsem-core` | VM/security/network runtime | Yes | Compile/no-run + non-live-KVM | Clippy | Yes |
| `capsem-logger` | Session database | Yes | Compile/no-run | Clippy | Yes |
| `capsem-guard` | Companion lifecycle | Yes | Compile/no-run | Clippy | Yes |
| `capsem-service` | Daemon API/orchestration | Yes | Compile/no-run | Clippy | Yes |
| `capsem-process` | Per-VM runtime | Yes | Compile/no-run | Clippy | Yes |
| `capsem` | CLI | Yes | Compile/no-run | Clippy | Yes |
| `capsem-tui` | Terminal UI | Yes | Compile/no-run | Clippy | Yes |
| `capsem-admin` | Profile/asset/release administration | Yes | Compile/no-run | Clippy | Yes |
| `capsem-mcp` | Host MCP server | Yes | Compile/no-run | Clippy | Yes |
| `capsem-mcp-aggregator` | External MCP subprocess manager | Yes | Compile/no-run | Clippy | Yes |
| `capsem-mcp-builtin` | Built-in MCP tools | Yes | Compile/no-run | Clippy | Yes |
| `capsem-gateway` | Authenticated HTTP gateway | Yes | Compile/no-run | Clippy | Yes |
| `capsem-app` | Tauri shell | Check | No | Clippy | Yes |
| `capsem-tray` | System tray | Yes | No | Clippy | Yes |
| `capsem-agent` | Guest binaries | Yes | Compile/no-run | Clippy | Yes |
| `capsem-bench` | Benchmark harness | Yes | Compile/no-run | Clippy | Yes |
| `capsem-mock-server` | Hermetic test upstream | Yes | Compile/no-run | Clippy | Yes |

### Python integration suite tier map

| Suite | Marker | VM? | CI | Smoke | Full |
|-------|--------|:---:|:--:|:-----:|:----:|
| capsem-bootstrap | `bootstrap` | No | Collect; run in full gate after assets exist | No | Yes |
| capsem-codesign | `codesign` | No | Collect; run in full gate after signing | No | Yes |
| capsem-rootfs-artifacts | `rootfs` | No | Run | No | Yes |
| capsem-mcp | `mcp` | Yes | Collect | Yes | Yes |
| capsem-service | `integration` | Yes | Collect | Yes | Yes |
| capsem-cli | `integration` | Yes | Collect | Yes | Yes |
| capsem-gateway | `gateway` | Yes | Collect | Yes | Yes |
| capsem-e2e | `e2e` | Yes | Collect | No | Yes |
| capsem-session | `session` | Yes | Collect | No | Yes |
| capsem-session-lifecycle | `session_lifecycle` | Yes | Collect | No | Yes |
| capsem-session-exhaustive | `session_exhaustive` | Yes | Collect | No | Yes |
| capsem-security | `security` | Yes | Collect | No | Yes |
| capsem-isolation | `isolation` | Yes | Collect | No | Yes |
| capsem-snapshots | `snapshot` | Yes | Collect | No | Yes |
| capsem-config | `config` | Yes | Collect | No | Yes |
| capsem-config-runtime | `config_runtime` | Yes | Collect | No | Yes |
| capsem-guest | `guest` | Yes | Collect | No | Yes |
| capsem-cleanup | `cleanup` | Yes | Collect | No | Yes |
| capsem-stress | `stress` | Yes | Collect | No | Yes |
| capsem-recovery | `recovery` | Yes | Collect | No | Yes |
| capsem-serial | `serial` | Yes | Collect | No | Yes |
| capsem-lifecycle | `integration` | Yes | Collect | No | Yes |
| capsem-build-chain | `build_chain` | Yes | Collect | No | Yes |
| capsem-recipes | `recipe` | No | Run | No | Yes |
| capsem-install | `install` | No | Yes (Docker) | No | Yes |

"Run" = tests execute in PR CI. "Collect" = imports verified (`--collect-only`) but tests do not execute in that PR lane. Artifact-dependent no-VM suites still execute in the full `just test` gate after their build/sign prerequisites exist. "Yes (Docker)" = runs in dedicated Docker+systemd CI job.
