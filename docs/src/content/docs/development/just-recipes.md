---
title: Just Recipes
description: Complete reference for all just recipes -- the single entry point for building, testing, and shipping Capsem.
sidebar:
  order: 10
---

[just](https://just.systems) is the task runner. Every build, test, and release workflow goes through the justfile. Run `just --list` to see all public recipes.

## Daily development

| Recipe | What it does | Time |
|--------|-------------|------|
| `just shell` | Build/sign as needed, boot a VM, and attach a shell | ~10s after first build |
| `just exec "CMD"` | Run a command in a fresh disposable VM, then destroy it | ~10s after first build |
| `just run-service` | Start or reuse the daemon service | continuous |
| `just ui` | Tauri desktop app with hot reload and the service path | continuous |
| `just dev-frontend` | Frontend-only dev server with mock data on port 5173 | continuous |
| `just build-ui [release]` | Frontend build plus `cargo build -p capsem-app` | build dependent |

`just shell` is the daily VM driver. `just exec "CMD"` is the one-shot path for
quick checks. After frontend changes intended for the desktop app, use
`just build-ui`; the Tauri binary embeds `frontend/dist` at cargo build time.

## Testing

| Recipe | What it does | Boots VM? |
|--------|-------------|-----------|
| `just smoke` | Fast end-to-end gate: audit, doctor, injection, service/CLI/MCP/gateway tests | Yes |
| `just test` | Full gate: unit, coverage, cross-compile, frontend, Python, injection, integration, benchmarks, install E2E | Yes |
| `just test-gateway` | Gateway unit and mock-UDS tests | No |
| `just test-gateway-e2e` | Gateway E2E tests with real service and VMs | Yes |
| `just test-install` | Installer E2E in Docker/systemd | No host VM |
| `just bench` | In-VM and host lifecycle benchmarks | Yes |

`just test` is the source of truth. Targeted commands are for iteration, not
for declaring a sprint done.

## Policy Verification

Policy work spans parser contracts, runtime boundaries, settings UI, docs,
and telemetry. Use this sequence for focused iteration:

| Step | Command |
|---|---|
| Rust policy contracts | `cargo test -p capsem-core policy_config --lib` |
| Framed MCP policy | `cargo test -p capsem-core net::mitm_proxy::mcp_frame --lib` |
| Frontend policy UI/model | `pnpm -C frontend test -- settings-model settings-export api settings-store` |
| Frontend type/check gate | `pnpm -C frontend run check` |
| Docs gate | `cd docs && pnpm run build` |
| VM smoke | `just smoke` |
| Session integrity | `just inspect-session [id]` |
| Session SQL proof | `just query-session "SQL" [id]` |
| Final gate | `just test` |

Useful policy audit queries:

```bash
just query-session "
SELECT event_id, event_type, rule_id, rule_action, detection_level
FROM security_rule_events
ORDER BY timestamp_unix_ms DESC
LIMIT 20;"
```

```bash
just query-session "
SELECT m.event_id, m.server_name, m.method, m.tool_name, m.decision,
       s.rule_id, s.rule_action, s.detection_level
FROM tool_calls m
LEFT JOIN security_rule_events s ON s.event_id = m.event_id
WHERE m.origin = 'mcp'
ORDER BY m.id DESC
LIMIT 20;"
```

```bash
just query-session "
SELECT n.event_id, n.domain, n.method, n.path, n.decision,
       s.rule_id, s.rule_action, s.detection_level
FROM net_events n
JOIN security_rule_events s ON s.event_id = n.event_id
ORDER BY n.id DESC
LIMIT 20;"
```

## VM image builds

| Recipe | What it does | Time |
|--------|-------------|------|
| `just build-assets code [arch]` | Full profile-derived rebuild: kernel + rootfs via `capsem-admin` (needs Docker) | ~10 min |
| `just build-kernel <arch> code` | Kernel only through the profile-derived profile-derived build rail | ~5 min |
| `just build-rootfs <arch> code` | Rootfs only through the profile-derived profile-derived build rail | ~8 min |
| `just cross-compile [arch]` | Full Linux build in container: agent binaries + deb + AppImage | ~15 min |

You only need `just build-assets code` on first setup or when profile-owned
package/root/install inputs or backend image templates change rootfs contents.
Day-to-day, `just shell` and `just exec` repack the initrd without rebuilding
rootfs images.

Runtime recipes run the shared generated-config path:

```text
_check-assets -> _pack-initrd -> _materialize-config -> _ensure-service
```

`_materialize-config` invokes `capsem-admin profile materialize`, which writes
the current-build runtime profile under `target/config/` from checked-in
`config/` source files and `assets/manifest.json`.

## Session inspection

| Recipe | What it does |
|--------|-------------|
| `just inspect-session [id]` | Session DB integrity check + event summary (latest by default) |
| `just list-sessions` | Table of recent sessions with event counts per table |
| `just query-session "SQL" [id]` | Run raw SQL against a session DB |
| `just update-fixture <path>` | Copy + scrub a real session DB as test fixture |

## Dependency management

| Recipe | What it does |
|--------|-------------|
| `just audit` | Check for known vulnerabilities in Rust + npm deps |
| `just update-deps` | `cargo update` + `pnpm update` to latest compatible versions |
| `just update-prices` | Refresh model pricing JSON from upstream |
| `just doctor` | Check tools, colored output, structured recap (exits 1 if failures) |
| `just doctor fix` | Doctor + auto-fix all fixable issues in dependency order |

## Release

| Recipe | What it does |
|--------|-------------|
| `just cut-release` | Run tests, bump version, stamp changelog, tag, push, wait for CI |
| `just release [tag]` | Wait for CI to build + publish an existing tag |
| `just install` | Build release package and install locally |

## Cleanup

| Recipe | What it does |
|--------|-------------|
| `just clean` | Remove Rust + frontend build artifacts |
| `just clean all` | Deep clean: build artifacts + container images + docker cache |

## Dependency chains

Recipes automatically pull in their prerequisites. You never need to run setup steps manually.

```text
shell            -> _check-assets + _pack-initrd + _ensure-service
exec             -> run-service
run-service      -> _check-assets + _pack-initrd + _ensure-service
ui               -> _ensure-setup + _pnpm-install + run-service
build-ui         -> _pnpm-install + frontend build + cargo build -p capsem-app
smoke            -> _install-tools + _pnpm-install + _check-assets + _pack-initrd + _ensure-service
test             -> _install-tools + _clean-stale + _pnpm-install + _generate-settings + _check-assets + _pack-initrd
build-assets     -> _install-tools + _clean-stale + doctor + capsem-admin image build
test-install     -> _build-host
cut-release      -> test + _stamp-version
```

`_`-prefixed recipes are internal (hidden from `just --list`). Key internal recipes:

| Recipe | What it does |
|--------|-------------|
| `_ensure-setup` | Checks setup state and required tools |
| `_install-tools` | Auto-installs Rust targets, components, and cargo tools |
| `_pack-initrd` | Cross-compiles guest agent + repacks initrd with latest binaries |
| `_sign` | Codesigns the binary with virtualization entitlement |
| `_check-assets` | Verifies VM assets exist, tells you to run `build-assets` if not |
| `_generate-settings` | Generates settings schema, UI metadata, and frontend mock data |
| `_ensure-service` | Builds/signs host binaries and starts or reuses the service |
