# Sprint: credential-pipeline

## Why now

The host-side credential detection in `crates/capsem-core/src/host_config.rs`
is five hand-rolled `detect_*` functions, one per provider. Each bakes env
var names, file paths, JSON field names, and OAuth markers into Rust. The
symptoms we just hit during `just install`:

1. The wizard failed to detect an Anthropic API key that is actually present
   on the host. There is no way to see *what* was tried, from the UI or the
   CLI.
2. Gemini credentials render as a single "Configured" badge even though
   Google has three shapes: `GEMINI_API_KEY`, `~/.gemini/settings.json`
   apiKey, and gcloud Application Default Credentials (`refresh_token`).
   The UI collapses them, so the user cannot tell whether the VM will see
   an API key or an OAuth-style ADC blob.
3. Adding a provider (Mistral, xAI, DeepSeek, Google Workspace) today means
   writing Rust, recompiling, shipping a release. The guest-side already
   ships declarative TOML per provider (`guest/config/ai/<provider>.toml`)
   with `[<provider>.api_key]`, `[<provider>.files.*]`, `[<provider>.network]`.
   The host-side is the only part still hardcoded.

At the same time we want to add **Google Workspace** as a first-class
provider via the `gws` CLI (<https://github.com/googleworkspace/cli>). GWS
has its own auth model (multiple workflows, encrypted-at-rest credentials
keyed off the OS keyring, optional `GOOGLE_WORKSPACE_CLI_TOKEN` /
`GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE` envs). It does not fit the single
"api_key or bust" shape the existing code assumes.

The fix is structural: make host detection declarative, inspectable, and
extensible, extend the guest image builder to understand multi-credential
providers, and add GWS as the first user of the new shape.

## Scope

In:

1. **Declarative host detection.** Extend the existing per-provider TOML
   (`guest/config/ai/<provider>.toml`) with a `[<provider>.detect]` section
   that spells out every source to try, in order. Shape per-source:
   `kind = "env" | "plain_file" | "json_field" | "json_contains"`, plus the
   relevant parameters (`name`, `path`, `field`, `markers`). Multiple
   credential *kinds* per provider (api_key, oauth, adc, workspace_token).
2. **Detection walker.** Replace the `detect_*` Rust functions with a single
   walker that reads the spec, tries each source, and returns a
   `DetectionReport { provider, kind, sources: [SourceOutcome] }` so callers
   can see exactly which sources were checked and why each did or did not
   match. The walker produces a `HostConfig`-equivalent for backwards compat
   so nothing downstream breaks.
3. **Inspection surface.** `capsem setup --explain` prints the full report
   to stdout; the wizard's Providers step renders an expandable "details"
   per provider that shows the same thing. "Why wasn't my key detected?"
   answered by reading the UI, not the source.
4. **UI disambiguation.** The Providers step shows the credential *kind*
   detected (API key / Claude OAuth / Google ADC / Workspace OAuth) instead
   of a single "Configured" badge. `ProvidersStep.svelte` loses the hardcoded
   `providerDefs` list and iterates the spec.
5. **Google Workspace provider (gws).** New `guest/config/ai/google_workspace.toml`
   spec covering the gws CLI install + its auth shapes:
   - env: `GOOGLE_WORKSPACE_CLI_TOKEN`, `GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE`
   - files: `~/.config/gws/.encryption_key`, the keyring-backed credential
     blob path (verify empirically)
   - install: brew on macOS, npm `@googleworkspace/cli` fallback
   Injected into the VM alongside ADC so gcloud-based auth and
   workspace-only OAuth co-exist.
6. **Image builder updates.** `capsem-builder` already installs per-provider
   CLIs and writes `[<provider>.files.*]` into the rootfs. Extend it to:
   - Understand multiple credential-kind entries per provider.
   - Handle the GWS encryption-key file correctly (can't be re-encrypted on
     the host; either inject the unlocked token or mint a guest-local
     encryption key and decrypt at boot).
   - Continue working end-to-end under `just build-assets`. A broken image
     builder is the stopgate.
7. **Delete the old.** `detect_anthropic_key`, `detect_google_key`,
   `detect_openai_key`, `detect_claude_oauth`, `detect_google_adc` all go.
   The two public entry points (`detect_host_config`,
   `detect_and_write_to_settings`) keep their signatures for backwards
   compat.
8. **Tests.** For each provider, round-trip: spec file -> walker ->
   `DetectionReport` matches a golden JSON. Existing unit tests in
   `host_config.rs` get ported to drive the walker with fixture home dirs.
   One new integration test per credential kind (api_key, oauth, adc,
   workspace) spins up a fake home dir and asserts the walker picks the
   right source first.

Out of scope, explicitly deferred:

- Encrypted-at-rest credential storage inside Capsem itself. The spec
  should describe sources, but we keep plain-text provider settings on disk
  for now (that's a separate sprint).
- Auto-refreshing OAuth tokens. We detect and inject; refresh happens in
  the guest against its own network policy or fails loudly.
- A "credential vault" UI redesign. The Providers step gets richer, but
  the overall shape (Settings > AI Providers) stays the same.

## Key decisions

1. **Spec lives in the existing guest TOML files, not a new registry.** We
   already have `guest/config/ai/<provider>.toml` as the source of truth
   for each provider (installed CLI, network policy, files injected into
   the guest). Adding `[<provider>.detect]` there keeps one file per
   provider -- no parallel directory that drifts.
2. **Cross-language consumer.** The Python image builder
   (`src/capsem/builder/`) already reads these TOMLs. The new `[detect]`
   section stays builder-irrelevant (the builder only cares about what
   lands in the guest, not what the host found) but both Rust and Python
   must tolerate the extra keys. Python side: field should round-trip
   through the generator without breaking goldens.
3. **`DetectionReport` is user-visible, not internal.** It gets serialized
   to JSON on the `/setup/detect` endpoint and printed by
   `capsem setup --explain`. Schema stable from day one:
   `{ provider, kind, selected_source?, sources: [{ kind, name, status,
   reason }] }`. `status` is one of `matched` / `skipped` / `not_found` /
   `invalid`.
4. **Multi-credential-per-provider.** Anthropic has api_key + Claude OAuth.
   Google has api_key + ADC + (new) Workspace OAuth. The walker returns a
   `Vec<DetectionReport>` per provider. The UI shows each kind as its own
   row with its own badge.
5. **Google Workspace is an AI-adjacent provider, not an AI one.** It ships
   in `guest/config/ai/` for now because that's where the Workspace CLI
   (`gws`) lives in the install manager. If this grows a second non-AI
   provider we move the dir.
6. **Builder-side precedence is explicit.** If the spec has multiple
   sources and more than one matches, the walker picks the first in
   declaration order and records the rest as `matched but superseded` in
   the report -- not silent.
7. **No tauri runtime changes.** The existing `runDetection()` flow, the
   `/setup/detect` endpoint, and the settings leaves (`ai.anthropic.api_key`,
   `ai.google.gemini.google_adc_json`, etc.) stay. Only their producer
   changes.

## Files to create / modify

Host detection:
- `guest/config/ai/anthropic.toml` -- add `[anthropic.detect]` with
  env + file sources for both api_key and claude_oauth.
- `guest/config/ai/google.toml` -- add `[google.detect]` with sources for
  api_key and ADC.
- `guest/config/ai/openai.toml` -- add `[openai.detect]` for api_key.
- `guest/config/ai/google_workspace.toml` -- **new**, full spec for the
  gws CLI including install manager, detect entries for token/credentials
  file, files injected into the guest.
- `crates/capsem-core/src/host_config.rs` -- keep `HostConfig`,
  `DetectedConfigSummary`, the two entry points; replace the
  `detect_*` functions with a walker over the spec files.
- `crates/capsem-core/src/host_config/detect_spec.rs` -- **new**, spec
  loader + walker + `DetectionReport`.
- `crates/capsem-service/src/main.rs` -- `/setup/detect` returns
  `DetectionReport[]` alongside `DetectedConfigSummary`.
- `crates/capsem/src/setup.rs` -- add `capsem setup --explain` to dump
  the report to stdout without side effects.
- `crates/capsem/src/main.rs` -- wire the `--explain` flag.

UI:
- `frontend/src/lib/types/onboarding.ts` -- add `DetectionReport`,
  `SourceOutcome` shapes.
- `frontend/src/lib/components/onboarding/ProvidersStep.svelte` -- drop
  the hardcoded `providerDefs`, iterate the report. Show per-kind badge.
  Add "Why?" disclosure that renders the source list.
- `frontend/src/lib/components/shell/SettingsPage.svelte` (AI Providers
  section) -- same disclosure pattern so users outside the wizard can
  inspect detection.

Image builder:
- `src/capsem/builder/providers.py` (or equivalent) -- allow multiple
  credential-kind file entries, support GWS.
- `guest/config/ai/google_workspace.toml` consumed end-to-end through
  `just build-assets`. Verify `just shell` boots a VM with `gws --help`
  working.

Tests:
- `crates/capsem-core/src/host_config/tests/` -- new fixture-driven
  tests: one per provider + credential kind. Delete the old `detect_*`
  tests (keep the fixtures, port them).
- Python builder: golden fixtures for the new TOML shape so the spec
  doesn't break the generator.
- Python install tests: existing `test_setup_wizard.py` keeps asserting
  the settings leaves populate correctly.

Docs:
- `skills/dev-capsem/SKILL.md` -- add a line pointing at the new detect
  spec as the place to add a provider.
- `docs/` -- user-facing "Adding a provider" page (optional in this
  sprint, list in tracker).

## Order of work

The order is about keeping `just build-assets` green the whole time, so
we can dogfood each change:

1. **Landing gear.** Spec loader + walker over the existing TOMLs, but
   *behind* the current `detect_*` calls. New unit tests pass, old tests
   still pass. No behavior change yet.
2. **Switch over.** Replace `detect_*` callers with the walker. Delete
   the old functions. `cargo test`, `pnpm run check`, `just smoke`.
3. **Inspection.** `DetectionReport` surfaces on `/setup/detect` and
   `capsem setup --explain`. UI disclosure. No new provider yet.
4. **Google Workspace.** Land the new TOML, update the builder,
   `just build-assets`, `just shell` verifies `gws` is present, detection
   report includes it.
5. **Polish.** Settings page disclosure. Doc page. Final test pass.

Each numbered step is at least one commit; step 2 and step 4 are likely
two each.

## What "done" looks like

- `capsem setup --explain` on my machine correctly identifies the
  Anthropic API key that the old code missed, and prints exactly what it
  found and what it skipped.
- The wizard's Providers step shows Anthropic with two rows (API key and
  Claude OAuth) and Google with three (API key, ADC, Workspace), each
  with its own badge. Clicking "Why?" on any row shows the source list.
- `just build-assets && just shell` boots a VM where
  `gws drive files list` works against the user's Workspace domain using
  whatever credential was detected on the host.
- Removing every `detect_*` function from `host_config.rs` leaves
  `cargo test -p capsem-core` green.
- Adding a new provider is: drop a TOML in `guest/config/ai/`, run
  `just build-assets`, done. No Rust edit.
