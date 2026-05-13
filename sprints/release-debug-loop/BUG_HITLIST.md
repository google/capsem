# Release Debug Loop Bug Hitlist

Last updated: 2026-05-13

## How To Use This File After Context Reset

Start here. The active mode is debug loop mode: confirm one bug, trace the full chain, write the evidence down, add a failing proof, then fix. Do not jump to patches from screenshots or guesses.

Use these skills/instructions:

- `dev-bug-review`
- `dev-debugging`
- `dev-installation`
- `superpowers:systematic-debugging`
- `superpowers:test-driven-development`

## Ground Rules

- Work one bug at a time.
- Bug 1 is first because missing assets likely causes or masks VM list failures.
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
- Keep screenshots as evidence, but do not treat screenshots as root cause.

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

## Active Bug List

### B1 - VM assets missing / install setup chain broken

Status: fix implemented; live install verification still pending.

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

Required proof:

- A failing test or script proving the install/setup chain can complete while assets remain missing or unknown.
- If live proof is needed, run `just install` from a terminal where the sudo password can be entered, then capture:
  - `capsem status`
  - `capsem run 'ls -l /etc/resolv.conf; getent hosts elie.net; curl -fsS --connect-timeout 10 https://elie.net >/dev/null'`
  - `ls -la ~/.capsem ~/.capsem/assets ~/.capsem/assets/$(uname -m)`
  - gateway `/status`

Open questions:

- Does release `capsem setup` download heavy assets, or does it skip because only the signed manifest exists? Answer: it attempts to download from the signed manifest during `step_welcome`.
- Does `setup-state.json` mark install complete before assets are actually present? Answer: before the B1 fix, yes. A failed download was logged as a warning, `welcome` had already been marked done, and setup still printed `Setup complete`.
- Does the app open before setup/assets are complete?
- Does service status fail to report asset health when service is not fully initialized?

Progress:

- Reproduction/proof: `tests/capsem-install/test_setup_wizard.py::test_setup_fails_when_required_assets_cannot_download` simulates a package-style manifest-only asset directory and an unreachable release URL. Before the fix, setup exited 0 and reported success despite missing VM assets.
- Root cause: `crates/capsem/src/setup.rs` marked the `welcome` setup step done before the background asset download completed, then converted download failure into a warning instead of an error. Later setup runs would skip the only download step.
- Fix: setup now marks `welcome` only after the asset download task succeeds, and asset download errors abort setup so package postinstall fails loudly.
- Verification: `uv run pytest tests/capsem-install/test_setup_wizard.py::test_setup_fails_when_required_assets_cannot_download -q`; `cargo test -p capsem setup -- --nocapture`.
- Remaining gate: rerun `just install` from a sudo-capable terminal and capture the live status/guest DNS/HTTPS proof.

### B2 - AI provider onboarding/settings parsing broken

Status: queued.

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

Status: queued after B1.

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

## Existing Related Work In This Checkout

Install hardening changes already made:

- `justfile`
  - `just install` now preserves settings, runs forced uninstall, asserts clean state, clears stale Tauri app bundle, uses native package install commands, verifies installed layout/version, service, gateway, and guest DNS/HTTPS.
- `tests/test_release_workflow_policy.py`
  - static policy coverage for hard clean, native install parity, stale bundle cleanup, and guest network gate.
- `crates/capsem/src/main.rs`
  - prevents `capsem uninstall --yes` from spawning background update refresh.

Verification already passed:

- `cargo fmt --check`
- `uv run python -m pytest tests/test_release_workflow_policy.py -q`
- `cargo test -p capsem uninstall_does_not_refresh_update_cache -- --nocapture`
- `just --list | rg "install|test-install"`

Live verification blocked:

- `just install` reached the macOS sudo password prompt in this Codex session.
- It needs to be rerun in a terminal where the password can be entered.

## Notes For Next Agent

- Do not assume the release install and local install are equivalent until traced.
- The package postinstall opens the GUI after setup; this may expose a transient or failed setup state.
- If the GUI appears before assets are ready, either setup/install ordering is wrong or the UI needs a clear "install still running/downloading" state.
- Do not hide missing assets with nicer UI. The first bug is to make assets actually present or make setup fail loudly.
