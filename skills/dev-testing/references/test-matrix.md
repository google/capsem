# Test Matrix: What Runs Where

Reference for /dev-testing: per-crate Rust CI matrix and the Python integration suite tier map (PR CI vs smoke vs full gate).

## Test matrix: what runs where

### Rust crate CI matrix

| Crate | Tests | CI macOS | CI Linux | Smoke | Full |
|-------|------:|:--------:|:--------:|:-----:|:----:|
| capsem-core | ~1695 | Yes | Compile/no-run + non-live-KVM | No | Yes |
| capsem-agent | ~71 | Yes | Compile/no-run | No | Yes |
| capsem-logger | ~47 | Yes | Compile/no-run | No | Yes |
| capsem-proto | ~132 | Yes | Compile/no-run | No | Yes |
| capsem-gateway | ~38 | Yes | Compile/no-run | No | Yes |
| capsem-service | ~109 | Yes | Compile/no-run | No | Yes |
| capsem (CLI) | ~140 | Yes | Compile/no-run | No | Yes |
| capsem-mcp | ~67 | Yes | Compile/no-run | No | Yes |
| capsem-tray | ~47 | Yes | No | No | Yes |
| capsem-process | ~62 | Yes | Compile/no-run | No | Yes |
| capsem-app | ~35 | Check | No | No | Yes |

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
