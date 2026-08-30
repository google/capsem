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
`just build-ui`; the Tauri binary embeds `web/app/dist` at cargo build time.

## Testing

| Recipe | What it does | Boots VM? |
|--------|-------------|-----------|
| `just fast-test` | Incomplete source feedback from the canonical `_test-fast` module | No |
| `just focus-test <group>` | Run one existing owner: `assets`, `binaries`, `benchmark`, `install`, `release-system`, or `functional`; `release-system` is source-only contract proof | Depends on group |
| `just install` | Build the complete local macOS package and install that exact package for hands-on testing | No |
| `just test-full [commit]` | Exceptional cold whole-system diagnostic: rebuild and exercise every configured artifact and VM path | Yes |

Use `fast-test` once for cheap source feedback and the smallest `focus-test`
group for a specific regression. Use `test-full` only when stale reuse is
suspected or a release candidate is ready for one cold Mac diagnostic. The
hosted `release-profile` and `release-binaries` lanes are the publication
qualification authority; they do not consume the local diagnostic journal.

## Policy Verification

Policy work spans parser contracts, runtime boundaries, settings UI, docs,
and telemetry. Use this sequence for focused iteration:

| Step | Command |
|---|---|
| Rust policy contracts | `cargo test -p capsem-core policy_config --lib` |
| Framed MCP policy | `cargo test -p capsem-core net::mitm_proxy::mcp_frame --lib` |
| Frontend policy UI/model | `pnpm -C web/app test -- settings-model settings-export api settings-store` |
| Frontend type/check gate | `pnpm -C web/app run check` |
| Docs gate | `cd web/docs && pnpm run build` |
| Focused VM feedback | `just focus-test functional` |
| Session integrity | `just inspect-session [id]` |
| Session SQL proof | `just query-session "SQL" [id]` |
| Final gate | `just test-full` |

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
| `just build-kernel <arch> code` | Kernel only through the profile-derived build rail | ~5 min |
| `just build-rootfs <arch> code` | Rootfs only through the profile-derived build rail | ~8 min |
| `just cross-compile [arch]` | Full Linux build in container: agent binaries + `.deb` package | ~15 min |

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
`config/` source files and `target/assets/manifest.json`.

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
| `just update-deps` | `cargo update` + `pnpm update` to latest compatible versions |
| `just update-prices` | Refresh model pricing JSON from upstream |
| `just doctor` | Check tools, colored output, structured recap (exits 1 if failures) |
| `just doctor fix` | Doctor + auto-fix all fixable issues in dependency order |

Rust and JavaScript vulnerability audits are mandatory parts of `just fast-test`
and `just test-full`; there is no separate public audit recipe that can drift from
the tested composition.

## Release

| Recipe | What it does |
|--------|-------------|
| `just release-binaries <channel> <source-commit>` | Qualify one committed source, build packages only, and publish binary-owned manifest fields |
| `just release-profile <channel> <profile> <source-commit>` | Qualify one committed source, build one channel/profile, and publish only that profile |

Both commands share one `capsem-release-<channel>` lock from source-manifest
read through deployment. If a profile needs newer code, release the profile
first as staged immutable assets, then release the binary; the second lane
reuses the same profile bytes and activates the completed pairing after the
full functional and glow-up proof.

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
test-install     -> Docker package install + generated local stable/nightly glow-up
build_system/packaging/macos/macos_release_glowup.py -> production .pkg + clean Tart install + physical-host exact-payload VZ boot
release-profile  -> capsem-admin release + locked one-profile workflow
release-binaries -> adversarial binary script + locked package workflow
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
| `_test-fast` | YAML/source syntax, source contracts, Clippy, Python/JavaScript checks, web surfaces, and all dependency audits |
| `_test-static` | Rust/Python coverage, install-harness preflight, and cross-compilation |
| `_test-artifacts` | Packages, inventories, SBOM/OBOM, images, evidence, digests, architecture coverage, and boot |
| `_test-functional` | VM suites, Winterfell, MCP lifecycle, IronBank, injection, integration, benchmarks, and full doctor |
| `_test-glowup` | Native install and manifest-driven binary/profile/channel update transitions |
| `_test-release-contracts` | Lane boundaries, shared serialization, deploy containment, and corporate authoring |

## Where the logic lives

The justfile dispatches; it does not decide. Every recipe is a call into
`capsem-gate` or a single command, and none carries a shell body -- a contract
test holds that. The build, test and release logic lives in
`src/capsem/gate/`, where it is unit tested.

That means a recipe is rarely the thing to read. To see what one does:

```bash
uv run --project build_system --frozen capsem-gate <command> --dry-run
```

which prints every step in execution order with the exact argv it would invoke,
and runs none of it. `--graph` prints the same thing as a diagram, and
`--timing` reports where a finished run's time went.
