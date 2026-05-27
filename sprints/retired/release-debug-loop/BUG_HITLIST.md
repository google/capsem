# Release Startup Reliability Symptom Hitlist

Last updated: 2026-05-13

## How To Use This File After Context Reset

Start with `MASTER.md` and `startup-info.md`, then use this file as evidence.
The old B1/B2/B3 bugs are symptoms feeding the startup reliability meta-sprint,
not standalone patch tickets.

The active release gate is:

```bash
capsem uninstall
just install
capsem status
```

S1 expands `capsem status` into the release health gate. Until then, use
explicit installed runtime, service, gateway, setup, assets, UI/tray/app, and
saved-VM checks.

Use these skills/instructions:

- `dev-sprint`
- `dev-bug-review`
- `dev-debugging`
- `dev-installation`
- `dev-testing`
- `superpowers:brainstorming`
- `superpowers:systematic-debugging`
- `superpowers:test-driven-development`

## Ground Rules

- Work one sprint slice at a time.
- Do not treat screenshots as the root cause.
- Do not let existing code define the desired install/startup chain.
- Package install and expanded `capsem status` have their own sprint.
- Saved VM asset dependencies are first-class requirements.
- Asset supervision belongs to the service; the installer should not call a
  one-off asset reconcile RPC.
- For each bug, fill in:
  - reproduction
  - expected behavior
  - actual behavior
  - traced setup/runtime chain
  - root cause
  - failing test/proof
  - fix
  - verification
- No production code patch before failing proof unless the reason is written here.
- If a previous patch proves a failure but conflicts with the desired ownership
  model, mark it as superseded instead of calling it done.

## Current Local Machine State

- A live `just install` was attempted multiple times.
- First live run proved the hard-clean gate caught `~/.capsem/update-check.json` being recreated by `capsem uninstall --yes`.
- That uninstall/update-cache side effect was fixed in `crates/capsem/src/main.rs` with targeted unit coverage.
- A later live run passed the hard-clean phase, then blocked at macOS `sudo` password before package installation.
- Because the run stopped after the clean phase, local `~/.capsem` is currently absent.
- `~/Library/LaunchAgents/com.capsem.service.plist` was also absent when last checked.
- The GUI screenshots therefore may reflect a half-installed state, but the same symptoms were also reported on the other Mac after a release install, so we must treat them as real release-flow bugs until disproven.
- Later local state check found `~/.capsem` present again with all three `arm64`
  VM assets present and `capsem status` reporting assets OK, but service
  running=false and gateway port 19222 refusing connections. Treat that as a
  possible B3/service-health follow-up, not proof that B1 was false.

## Active Symptom List

### B1 - VM assets missing / install setup chain broken

Status: superseded by startup reliability meta-sprint.

New sprint homes:

- S1 - Package Install And `capsem status` Health Gate
- S3 - Service Asset Supervisor And Consumer Audit
- S5 - `capsem-setup` Hardening
- S6 - UI Wizard/Dashboard Startup States

User evidence:

- Onboarding Welcome step reports:
  - `vmlinuz Missing`
  - `initrd.img Missing`
  - `rootfs.squashfs Missing`
- New tab reports:
  - `VM asset status is unknown`
  - `Waiting for the service to report rootfs and manifest readiness`
- This matches the original other-Mac report: installed/running service, but key VM/network functionality broken.

Suspected area:

- macOS `.pkg` postinstall and/or local `just install` chain:
  - package payload seeds only `manifest.json` and `manifest.json.minisig`
  - postinstall calls `capsem install`
  - postinstall calls `capsem setup --non-interactive --accept-detected`
  - setup should download or verify VM assets
  - service/gateway status should report asset readiness
- Local dev path additionally runs `scripts/sync-dev-assets.sh` after install, but this happens after the package postinstall and after the app may have opened.

Need trace, in order:

1. `scripts/build-pkg.sh`
2. `scripts/pkg-scripts/postinstall`
3. `crates/capsem/src/setup.rs`
4. `crates/capsem-core/src/asset_manager.rs`
5. service startup asset loading in `crates/capsem-service/src/main.rs`
6. gateway `/status` mapping in `crates/capsem-gateway/src/status.rs`
7. frontend asset display in:
   - `frontend/src/lib/stores/onboarding.svelte.ts`
   - `frontend/src/lib/components/onboarding/WelcomeStep.svelte`
   - `frontend/src/lib/components/shell/NewTabPage.svelte`

Required proof now:

- `capsem uninstall -> just install -> capsem status` once S1 expands the
  diagnostic.
- Until then, a failing test or script proving the install/setup chain can
  complete while assets remain missing, unknown, or unobservable.
- Live proof from a terminal where the sudo password can be entered, then capture:
  - `capsem status`
  - `capsem run 'ls -l /etc/resolv.conf; getent hosts elie.net; curl -fsS --connect-timeout 10 https://elie.net >/dev/null'`
  - `ls -la ~/.capsem ~/.capsem/assets ~/.capsem/assets/$(uname -m)`
  - gateway `/status`
  - app/tray launch status
  - saved VM fixture status once S4 exists

Open questions:

- Does release `capsem setup` download heavy assets, or does it skip because only the signed manifest exists? Answer: it attempts to download from the signed manifest during `step_welcome`.
- Does `setup-state.json` mark install complete before assets are actually present? Answer: before the B1 fix, yes. A failed download was logged as a warning, `welcome` had already been marked done, and setup still printed `Setup complete`.
- Does the app open before setup/assets are complete?
- Does service status fail to report asset health when service is not fully initialized?

Progress and correction:

- Reproduction/proof: `tests/capsem-install/test_setup_wizard.py::test_setup_fails_when_required_assets_cannot_download` simulates a package-style manifest-only asset directory and an unreachable release URL. Before the fix, setup exited 0 and reported success despite missing VM assets.
- Root cause: `crates/capsem/src/setup.rs` marked the `welcome` setup step done before the background asset download completed, then converted download failure into a warning instead of an error. Later setup runs would skip the only download step.
- Superseded experiment: setup was briefly patched to mark `welcome` only after the asset download task succeeds, and asset download errors aborted setup.
- Why superseded: the final architecture should allow setup/config work to continue while service-owned asset supervision reports `checking`, `updating`, `ready`, or `error`.
- Follow-up: the narrow patch was reverted. S1/S3/S5 must replace it with service-owned asset truth plus honest setup/UI status.

### B2 - AI provider onboarding/settings parsing broken

Status: mapped into S5 and S6.

User evidence:

- Providers step screenshot shows title and description but no provider rows or controls.
- User says "settings are not properly parsed and the AI stuff is fubar".

Likely immediate symptom:

- `frontend/src/lib/components/onboarding/ProvidersStep.svelte` derives provider rows from `api.getSettings().tree`.
- If settings are unavailable, empty, malformed, or the tree IDs changed, the component silently catches and renders an empty provider list.

Need trace:

1. Service `/settings` response.
2. Settings model/tree IDs for:
   - `ai.anthropic.api_key`
   - `ai.openai.api_key`
   - `ai.google.api_key`
   - `repository.providers.github.token`
   - `ai.anthropic.claude.credentials_json`
3. `ProvidersStep.svelte` error handling and fallback rows.
4. Detection API:
   - `frontend/src/lib/api.ts` `runDetection`
   - service setup/detect handler
   - host credential detection writer.

Required proof:

- A frontend test where `/settings` returns an empty or partial tree and Providers step still renders actionable provider rows or a clear error.
- A backend/settings test if the IDs or parser output are wrong.

### B3 - VM list/session UI broken after launch

Status: mapped into S2, S3, S4, and S6.

User evidence:

- User reports VM list is also broken once launched.
- Likely related to missing/unknown assets, because VM creation/listing depends on service being healthy and asset resolution succeeding.

Need trace after B1:

1. Gateway `/status`
2. Service `/list`
3. New tab VM list state.
4. Any service logs around asset resolution or session provisioning.

Required proof:

- Reproduce after assets fixed or confirmed missing.
- If still broken with assets ready, write a separate failing UI/API test for VM list state.

### B4 - Built-in local MCP tools cannot be disabled

Status: fixed in the current checkout; keep follow-up UI verification open.

User evidence:

- User reports the MCP local tools cannot be disabled or modified and are "on
  all the time".

Root cause found:

- `build_server_list_with_builtin` always inserted the built-in `local` server
  with `enabled: true`, bypassing `mcp.servers.local.enabled`.
- `batch_update_settings_json` rejected `mcp.servers.local.enabled` as an
  unknown setting, so the UI toggle could not persist the user choice.
- The settings tree filtered disabled MCP server nodes out, which would remove
  the toggle after a successful disable.
- Agent config injection wrote disabled stdio MCP servers into Claude/Gemini/Codex
  configs anyway.

Fix:

- The built-in runtime server now honors corp-over-user enabled overrides.
- Settings save accepts `mcp.servers.<name>.enabled` and writes
  `[mcp.server_enabled]`.
- Disabled MCP servers remain visible in the settings tree so they can be
  re-enabled.
- Agent config injection removes disabled generated stdio MCP servers while
  preserving unrelated user servers.

Verification:

- `cargo test -p capsem-core --lib build_server_list_builtin_local -- --nocapture`
- `cargo test -p capsem-core --lib batch_update_mcp_local_enabled_writes_override_and_keeps_node_visible -- --nocapture`
- `cargo test -p capsem-core --lib disabled_mcp_servers_are_not_injected_into_agent_configs -- --nocapture`
- `cargo test -p capsem-core --lib mcp_servers_in_tree -- --nocapture`
- `npx vitest run src/lib/__tests__/mcp-section.test.ts` from `frontend/`

## Existing Related Work In This Checkout

Install hardening changes already made:

- `justfile`
  - `just install` now preserves durable state, runs forced runtime uninstall, asserts old runtime state is gone, clears stale Tauri app bundle, uses native package install commands, verifies installed layout/version, service, gateway, and guest DNS/HTTPS.
- `tests/test_release_workflow_policy.py`
  - static policy coverage for runtime-clean uninstall, native install parity, stale bundle cleanup, and guest network gate.
- `crates/capsem/src/main.rs`
  - prevents `capsem uninstall --yes` from spawning background update refresh.
- `crates/capsem/src/status.rs`
  - `capsem status --json` now reports stale executable `capsem-service` and
    `capsem-process` helper binaries as `host_binary_version_mismatch` instead
    of accepting them as healthy.
- `crates/capsem/src/service_install.rs`
  - service/status reporting now honors install-test isolation and stops
    reading the developer's real LaunchAgent/systemd unit when `CAPSEM_HOME`,
    `CAPSEM_RUN_DIR`, or `CAPSEM_ASSETS_DIR` is set.
- `crates/capsem/src/uninstall.rs`
  - `capsem uninstall` removes service/runtime wiring, binaries, stale run files, and temporary VM state while preserving config, setup state, assets, logs, session/audit data, and persistent VM state.
  - Under install-test isolation (`CAPSEM_HOME` / `CAPSEM_RUN_DIR` /
    `CAPSEM_ASSETS_DIR`), uninstall skips real LaunchAgent/systemd mutation so
    black-box tests can prove isolated runtime cleanup safely.

Verification already passed:

- `rustfmt --edition 2021 --check crates/capsem/src/uninstall.rs`
- `uv run python -m pytest tests/test_release_workflow_policy.py -q`
- `cargo test -p capsem uninstall_does_not_refresh_update_cache -- --nocapture`
- `cargo test -p capsem uninstall -- --nocapture`
- `cargo test -p capsem status::tests -- --nocapture`
- `cargo test -p capsem service_status_ignores_platform_unit_in_isolation_env -- --nocapture`
- `uv run pytest tests/capsem-install/test_error_paths.py::TestErrorPaths::test_status_json_reports_stale_process_helper_binary -q`
- `uv run pytest tests/capsem-install/test_uninstall.py::TestUninstall::test_runtime_uninstall_preserves_durable_state -q`
- `just --list | rg "install|test-install"`

Live verification blocked:

- `just install` reached the macOS sudo password prompt in this Codex session.
- It needs to be rerun in a terminal where the password can be entered.

## Notes For Next Agent

- Do not assume release install, local install, and direct setup are equivalent
  until S1/S2 prove the contract.
- The package postinstall opening the GUI is part of the product chain and must
  be tested.
- If the GUI appears before assets are ready, the UI should show service-owned
  progress/error state, not pretend setup failed or succeeded silently.
- Do not hide missing assets with nicer UI. Make the service truth observable
  and make UI recovery honest.
- Do not delete rootfs/kernel/initrd blobs referenced by saved VMs.
