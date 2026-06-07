# Install, Assets, and Credential Broker Rebuild

## Goal

Remove the old `capsem setup` architecture and replace it with a disciplined
install path, first-class asset lifecycle, UI-owned onboarding, and the first
brokered substitution plugin for saving credentials observed inside the VM to
the broker credential store by default while user settings and downstream
systems carry only references.

This is foundation work, but it is product-facing: install should feel boring,
the UI should tell the truth, and credentials should flow through one brokered
security path rather than host scraping or parallel setup writes.

## Non-Goals

- Do not redesign the security event engine for T0-T5. T6 is the scoped
  exception: it fixes the rule/action/materialization boundary without
  replacing CEL, Sigma, or the typed security-event contract.
- Do not change network/file/model parsing behavior.
- Do not build full OAuth for OpenAI, Google, or Anthropic.
- Do not import host credentials during install/setup. Credential brokering
  starts from VM/user-observed credentials and visible token exchanges.
- Do not add a second credential UX layer. The brokered substitution plugin is
  infrastructure with minimal status/explanation UI, not a new feature maze.
- Do not keep compatibility shims for `capsem setup` unless a task explicitly
  proves a temporary bridge is necessary.

## Key Decisions

- `capsem setup` is obsolete and will be removed.
- `just install` must not mutate the installed tree after the package installer
  exits. It should build the package, put the package in the right place, and
  run the normal install path.
- Local dev packages must be self-contained for the current host arch, or must
  point at a local release/asset source before the service starts. They cannot
  rely on public GitHub releases for freshly stamped dev manifests.
- Release packages may remain manifest-only if the asset manager makes
  download progress and failures first-class.
- Service readiness is separate from asset readiness. The UI can open when the
  service and gateway are reachable; session creation stays disabled until
  assets are ready.
- Credential brokering is on by default. If a user creates `.env` with an AI
  key, or Capsem observes an OAuth/token exchange in the VM path, the broker
  saves the real credential into the broker credential store, writes only the
  stable reference to user settings, and substitutes that reference before the
  security/logging/policy pipeline sees it. Raw values must never be logged.
- Substitution is a general security pre-plugin contract. Credentials are the
  first material class, but the plugin shape must support future sensitive
  material classes without adding protocol-specific substitute loggers.
- Security actions run after rule match. The contract is
  `plugin(rule, SecurityEvent) -> SecurityEvent`; actions do not receive
  duplicated YAML source/target/provider/replacement metadata.
- Parser/runtime paths directly instantiate canonical `SecurityEvent`s and
  submit them to one auditable emitter. That emitter owns batching, DB writer
  handoff, logging/detection/enforcement fanout, and future multiprocess
  transport.
- New side writers to security/audit tables are forbidden. Protocol code adds
  typed context to `SecurityEvent`; it does not privately persist a second
  truth.
- HTTP outbound requests must be materialized from the final post-action
  security event. Direct MITM helper side paths are forbidden.
- The broker contract is protocol agnostic. HTTP authorization headers, GitHub
  tokens, OAuth/token exchanges, `.env` files, model payloads, MCP arguments,
  file content, and process/environment observations all use the same
  credential observation -> BLAKE3 reference -> security event/session DB shape.
- Ask-before-save, autosave-off, and broker-disable controls are later product
  settings. They must not block the first broker contract.

## Architecture

### Install State Machine

```text
build package
  -> install package payload
  -> register service
  -> start service
  -> wait for service + gateway
  -> open UI if GUI available
```

The package payload is the source of truth. No `just install` step may copy
assets or binaries into `~/.capsem` after the installer returns.

### Asset Lifecycle

Assets move out of setup and into an idempotent lifecycle:

```text
manifest loaded
  -> resolve expected files
  -> verify present hashes
  -> download missing/corrupt files if source configured
  -> expose status/progress through daemon API
```

Status names:

- `ready`
- `downloading`
- `missing`
- `corrupted`
- `failed`

The daemon must expose enough detail for the UI and CLI to explain the exact
file and action. Asset reconciliation must run on service start and through an
explicit CLI/API retry path.

Live byte counters are served from daemon memory so the downloader does not
write to disk for every chunk. Durable asset status is still first-class:
reconcile start, per-asset completion, and final success/failure are written to
`asset-status.json`, and daemon startup clears stale active progress from any
prior crash.

### UI States

The UI has independent rails:

- Service unavailable: "start service" / retry.
- Service available, assets not ready: show asset progress/failure; disable
  session creation.
- Assets ready: sessions available.
- Profile unconfigured: let user create/open a VM and configure tools there.
- Brokered credential available: show broker status and saved setting
  reference without exposing the raw secret.

No UI copy should tell the user to run `capsem setup`.
The old AI setup wizard should be simplified away: no provider OAuth/API-key
collection during install or onboarding. Users configure tools in the VM; the
broker observes credentials through the security path and saves them to user
settings by default.

### Brokered Substitution Plugin

Initial broker scope:

- Run for every credential observation that reaches Capsem. This is a security
  pre-plugin: raw secret in, credential reference out.
- Detect `.env` style credential candidates inside a VM/workspace.
- Detect visible OAuth/token exchange credentials in the VM security path.
- Candidate types: `OPENAI_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`,
  `ANTHROPIC_API_KEY`.
- Emit only metadata: provider, source path, key fingerprint, confidence,
  discovered_at, VM/session/profile context.
- Redact values from logs, security events, and DB previews.
- Save real values into the broker credential store by default through a
  broker-owned API, not direct file writes from random callers. User settings
  store only broker references.
- Return a stable credential reference string to the security pipeline so CEL,
  logging, detection, and enforcement can reason over the fact that a
  credential exists without handling the raw value.
- Use BLAKE3 for the substitution value. The broker replaces each raw secret
  with a canonical, domain-separated reference such as
  `credential:blake3:<hex>`, and the raw value is stored only through the
  credential store path. The digest input must include a Capsem credential
  domain tag and provider so references are stable without being a generic hash
  of the bare secret.
- Log broker decisions and failures with provider, source, BLAKE3
  reference/fingerprint, and outcome, never raw credential material.
- Emit a substitution log record for each replacement:
  `material_class`, `source`, `event_type`, `credential_ref` or future
  `substitution_ref`, `algorithm`, `confidence`, `outcome`, and session/profile
  context. The record must never include the raw material.
- Persist the BLAKE3 reference/fingerprint in `session.db` security-event rows
  as a top-level shared security-event field. Typed protocol tables may keep
  protocol-specific context, but credential identity must be read from the
  shared security-event envelope instead of duplicated as a protocol-owned
  identity.
- Keep future user controls out of the first path unless needed for safety:
  ask-before-save, autosave-off, ignore, and never-ask are follow-up settings.

Keychain decision: production macOS writes store raw credentials in Keychain
behind the broker API; tests use an explicit file-backed broker store. User
settings store only the stable broker reference. A future Linux credential-vault
backend can live behind the same API.

## Work Slices

### T0: Contract and Trace

- Write the authoritative install state diagram.
- Inventory current `setup` responsibilities and map each one to delete,
  replace, or defer.
- Define package mode matrix:
  - release `.pkg` / `.deb`
  - local dev `.pkg`
  - Docker/systemd install test
- Decide whether local dev packages bundle current arch assets or serve them
  from a local release URL.

### T1: Installer Discipline

- Remove post-installer mutation from `just install`.
- Make package payload contain everything needed for its chosen mode.
- Make macOS and Linux postinstall scripts use the same install contract.
- Add reinstall test where only `initrd` hash changes.
- Add stale symlink test for installed asset directory.

### T2: Asset Lifecycle

- Extract asset reconciliation from `setup`.
- Add CLI/API:
  - `capsem assets status`
  - `capsem assets ensure`
  - daemon `GET /assets/status`
  - daemon `POST /assets/ensure`
- Reconcile on service start without blocking service availability.
- Record progress/failure in a durable, queryable place.

### T3: UI States

- Remove setup/onboarding dependency from shell startup.
- Replace `/setup/assets` usage with `/assets/status`.
- Replace "run capsem setup" copy.
- Add asset progress, retry, and failure states.
- Make "service up but assets missing" a normal recoverable UI state.
- Simplify/remove AI setup wizard pages; provider setup happens inside the VM
  and the broker handles credential persistence.

### T4: Brokered Substitution Plugin

- Add credential observation and brokered-reference types.
- Add the general substitution plugin interface and log record type.
- Add redaction rules that make raw credential logging test-failing.
- Add BLAKE3 substitution/reference generation and collision tests.
- Add protocol-agnostic top-level credential fields to the shared security
  event envelope and session DB writes so every parser can carry the same
  BLAKE3 reference.
- Detect `.env` candidates in VM/workspace with fake test keys.
- Detect fake OAuth/token exchange material through the security pre-plugin
  path.
- Add fake HTTP/GitHub credential fixtures to prove the broker path is not
  AI-specific.
- Add broker API that saves observed credentials to the broker credential store
  by default, writes reference-only user settings, and returns credential
  references.
- Persist through one broker-owned path.
- Add minimal UI/settings status explaining brokered credentials by provider
  and reference, not raw values.
- Add/update architecture page for the broker invariant and its place in the
  session/security pipeline.

### T5: Burn Setup

- Remove `capsem setup` CLI command.
- Remove auto-setup on first command.
- Remove `/setup/retry`, `/setup/detect`, `/setup/complete`, and stale setup
  state dependencies after replacements land.
- Delete setup wizard tests or replace them with install/assets/profile tests.
- Update docs and troubleshooting.

### T6: Security Action Materialization

- Add typed rule `actions` and validate action identifiers through one action
  registry shared by CEL and Sigma-derived rules.
- Define the security action plugin contract as
  `plugin(rule, SecurityEvent) -> SecurityEvent`.
- Make parser/runtime paths instantiate canonical `SecurityEvent`s directly and
  submit them through one auditable emitter.
- Make that emitter own batching, DB writer handoff, logging/detection/
  enforcement fanout, and future multiprocess transport.
- Execute multiple actions deterministically, each receiving the event returned
  by the previous action.
- Build HTTP request security events before actions and materialize upstream
  HTTP requests from the final post-action event.
- Convert credential broker capture/substitute into registered action plugins.
- Keep direct MITM credential substitution side channels out of the request
  builder path.
- Preserve reference-only security/logging/session DB views while resolving
  raw brokered credentials only in the HTTP materializer for upstream dispatch.
- Audit model, MCP, DNS, file, and process paths so post-action event logging is
  consistent and unsupported wire mutation is explicit.
- Add fast benchmarks for action overhead and broker substitution.

## Files Likely Touched

- `justfile`
- `scripts/build-pkg.sh`
- `scripts/pkg-scripts/postinstall`
- `scripts/deb-postinst.sh`
- `scripts/sync-dev-assets.sh` (likely deleted or made test-only)
- `crates/capsem/src/main.rs`
- `crates/capsem/src/setup.rs` (delete)
- `crates/capsem/src/service_install.rs`
- `crates/capsem/src/update.rs`
- `crates/capsem-core/src/asset_manager.rs`
- `crates/capsem-service/src/main.rs`
- `frontend/src/lib/api.ts`
- `frontend/src/lib/stores/onboarding.svelte.ts`
- `frontend/src/lib/components/shell/App.svelte`
- `frontend/src/lib/components/shell/NewTabPage.svelte`
- `frontend/src/lib/components/onboarding/*` (delete or replace)
- `tests/capsem-install/*`
- `tests/capsem-service/*`

## Proof Matrix

- Unit/contract:
  - Asset resolver picks hash-prefixed paths consistently.
  - Asset reconciler is idempotent and reports missing/corrupt/download states.
  - Installer package mode selects correct asset payload policy.
  - Credential candidate parser redacts values and produces stable fingerprints.
  - BLAKE3 substitution is stable for the same secret and different for
    different secrets/providers.
  - Security event/session DB schema accepts brokered credential references in
    the shared event envelope and rejects raw credential values in credential
    reference fields.
  - Substitution log records include algorithm/ref/source/outcome metadata and
    never raw material.

- Functional:
  - `capsem install`, `capsem start`, `capsem status`, `capsem assets status`,
    and `capsem assets ensure` work without `capsem setup`.
  - UI connects to service with assets missing and shows the asset state.
  - UI disables session creation until assets are ready.

- Adversarial:
  - Reinstall with changed `initrd` hash.
  - Stale `~/.capsem/assets` symlink to old worktree.
  - Corrupt rootfs hash.
  - Asset download 404.
  - Service fails to start after package install.
  - `.env` contains malformed or low-confidence secret-like strings.

- E2E/integration:
  - Docker/systemd install test.
  - macOS `just install` normal package path.
  - VM session creation after assets become ready.
  - VM `.env` credential -> broker store -> user settings reference.
  - Fake OAuth/token exchange -> broker store -> user settings reference.
  - Fake HTTP authorization/GitHub token observation -> broker reference ->
    session DB security event with the same BLAKE3 reference.

- Telemetry/observability:
  - Installer failures print actionable cause.
  - Asset failures expose file, expected hash prefix, source URL/local source,
    and next action.
  - Credential broker events never contain secret values.
  - Security events and broker logs contain the BLAKE3 credential
    reference/fingerprint, not raw credentials.
  - Substitution plugin logs exist for replacements and are queryable without
    exposing raw material.
  - `session.db` rows carry the BLAKE3 reference/fingerprint on the shared
    security-event row; protocol-specific rows do not own a separate
    credential identity.

- Performance:
  - Asset status check is fast and does not hash the whole rootfs on every UI
    refresh.
  - Hash verification happens on reconcile, not polling.

## Done Means

- `capsem setup` is gone.
- The package install path is the only install path.
- `just install` does not patch `~/.capsem` after Installer exits.
- Service readiness and asset readiness are separate and explicit.
- A missing asset can be fixed through `assets ensure`, not setup.
- The UI never tells users to run setup.
- Fake `.env` and OAuth/token credentials in a VM can be brokered into the
  credential store, with reference-only user settings, without leaking raw
  values into events, logs, DB previews, or UI.
- HTTP/GitHub and future credential-bearing protocols use the same brokered
  credential identity shape; there is no AI-only credential lane.
