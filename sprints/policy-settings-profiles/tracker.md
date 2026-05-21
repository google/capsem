# Sprint: Policy, Settings, Profiles

## Where this sprint lives

**One branch, one worktree, one agent.** This sprint is executed
end-to-end on a single development branch in a single working tree.
Do not assume any item is being worked elsewhere unless this section
is updated with the concrete branch and worktree path, verified by
`git worktree list` and the listed branch's `git log`.

- **Branch:** `profile-v2`.
- **Worktree:** `/Users/elie/.codex/worktrees/824d/capsem`.
- **Verifying the state:** `git worktree list` shows every worktree
  on disk. `git log <branch> --oneline | head` shows what each
  branch has actually landed. **Read those two commands before
  trusting any prose in this file** -- prose drifts; git history
  does not.
- **Current git posture:** as of 2026-05-21, this branch is
  expected to be `138 ahead / 0 behind` `origin/main` in this worktree after the
  S07b admin closeout commit. The rescue
  reconciliation and S07 foundation are closed for the active profile sprint; do not
  resurrect the old "main is way ahead" warning unless `git
  rev-list --left-right --count HEAD...origin/main` says it is true
  again.

## Latest Green Gate

- **2026-05-20:** `just smoke` passed in 272s after the long-term smoke
  hygiene pass. The VM-heavy service/CLI and MCP suites now run in sequence
  inside smoke so Apple VZ cleanup pressure does not turn healthy service
  requests into client timeouts. The final pass also split MCP fixtures so
  VM lifecycle/destructive tests use the signed catalog-backed profile, while
  profile mutation tests opt into an editable unsigned fork explicitly.
- **2026-05-21:** S07b closeout passed the focused admin/profile/image/
  manifest/security/docs/doctor suite with `174 passed, 1 skipped`, plus
  `uv run python -m compileall src/capsem` and the docs build. This is the
  latest narrow gate proving the admin/profile/image trust-chain tooling stayed
  green after the S08a rule/detection planning commits.
- **2026-05-21:** S07/Post-S06 debt audit re-ran `cargo test -p capsem-core
  profile_manifest --lib`, `cargo test -p capsem-core --test profile_schema`,
  repeated `cargo test -p capsem-service profile_asset`, `cargo test -p
  capsem-service handle_fork`, `cargo test -p capsem-service vm_profile_pin`,
  `uv run python -m pytest tests/test_admin_cli.py tests/test_admin_hygiene.py
  tests/test_profiles.py tests/test_image_verify.py -q`, `git diff --check`,
  and `pnpm run build` in `docs`. The audit fixed the asset supervisor so every
  Profile V2 asset check path emits `profile_asset_check_finish`; the broad
  `profile_asset` gate is stable under parallel test execution.
- Focused proof before the full gate: `uv run python -m pytest
  tests/capsem-mcp/test_state_transitions.py::test_purge_all
  tests/capsem-mcp/test_state_transitions.py::test_isolated_mcp_session_does_not_affect_shared_service
  tests/capsem-mcp/test_mcp_connectors.py -v --tb=short -m mcp` passed with
  4 tests.

## Operating Mode

**Rescue is closed; S07 foundation is closed.** The S00-S06 audit plus
S07/S07a/S07c/S07d/S07b brought the branch back to a coherent profile-v2
contract:

- V1 settings/defaults authority is removed from the active runtime path.
- Profile V2 settings, resolver trace, Policy runtime wiring, UDS profile
  and rule routes, package/tool contracts, profile schema artifacts, Pydantic
  admin contracts, and profile-driven VM asset readiness have landed.
- Old asset-only manifests are no longer runtime authority. `assets.manifest.*`
  service settings and setup-time signed asset manifest checks are removed.

Legacy sprint directories are retired as Profile V2 planning authority. See
[RETIRED-LEGACY-SPRINTS.md](RETIRED-LEGACY-SPRINTS.md). They may provide
historical context, but active scope must be promoted into this board before it
can affect sequencing, product requirements, public surfaces, or release
claims.

The tracker is now an implementation board for the post-S07 engine work. Work
proceeds in this order:

1. S08b cuts the Network Engine, File Engine, Process Engine, Security Engine,
   Conversation Engine, and Resolved Event Emitter boundaries before public
   surfaces consume the event model.
   The model/MCP portion of that boundary consumes the side-sprint
   [S08 Side Sprint - Canonical AI Interaction Evidence](S08-side-canonical-ai-interaction-evidence.md):
   OpenAI, Anthropic, and Google/Gemini provider parsing must project into a
   canonical evidence layer before CEL, Sigma, telemetry, quotas, timeline, or
   plugin contracts rely on model/tool/MCP fields.
   The same boundary must distinguish accounting ownership from correlation:
   host/service AI calls can link to a VM/session/profile for explanation, but
   they require host attribution and must not increment VM health/model/MCP/
   token/cost counters.
2. S08d records VM-originated security-engine performance before speed claims;
   S09/S11/S12/S13/S14/S15/S16/S16a/S17 then lift CLI, status/debug,
   telemetry, plugins, rule UI, Confirm UX, profile UI, timeline/workbench, and
   security controls onto those contracts.
3. S19/S19a document and market only the behavior proven by those contracts.
4. S18 performs final release replay, doctor/VM/install verification, and any
   remaining cross-process/per-asset lock or upgrade hardening.

Winter readiness rules:

- Engineering quality vocabulary is defined in
  [The Ledger of the Realm](ENGINEERING-REALM-LEDGER.md). When this board says
  Lannister-grade, Winterfell-grade, Baratheon-grade, Tyrell-grade,
  Greyjoy-grade, or Iron-Bank clean, that reference is binding sprint language,
  not decoration.
- The old stack is dead and stays dead.
- Profiles are the banner under which VM assets, package assumptions, and
  runtime policy march.
- Service settings must be as typed, schematized, and admin-validatable as
  profiles before `capsem-admin` exposes them.
- Policy rules and detection rules must be deliberately separated or
  deliberately unified before telemetry/plugins/UI/Confirm make that choice
  expensive. S08a must pick real CEL for enforcement and a real
  Sigma-compatible detection path; the current CEL-like shortcut is not the
  final rule language.
- Detection is profile-owned. Signed profile revisions must resolve policy and
  detection packs for a VM; detections are not loose telemetry queries.
- Network transport, file/snapshot mechanics, process/audit mechanics, security
  decisions, and resolved-event emission must be separate contracts before
  public surfaces or enterprise plugin contracts harden around today's mixed
  paths.
- `session.db` must stop growing as independent authority tables. S08b must
  introduce a canonical resolved-event journal, then keep existing domain tables
  only as projections/read models unless a table is explicitly retired.
- Everyday agent work needs a first-class structured timeline. Codex/Claude SDK
  sessions and terminal fallback sessions must be reviewable/searchable through
  the single `/timeline/{id}` API with cursor pagination over typed timeline
  blocks, not raw PTY logs, a parallel conversation API, or direct SQL over
  legacy tables. Filtering and formatting belong in the client workbench over
  the loaded block window.
- A VM without explicit profile/revision/package/asset identity is invalid and
  must fail closed; there is no pre-S07a compatibility lane.
- The release gate is the wall: every claim needs tests, status/debug
  explanation, and tracker evidence before it crosses.

## Linear path

Strictly ordered. Finish item N before starting item N+1. No
parallel forks, no "if X then Y" branches, no parking-lot
proposals. If a new concern surfaces, it gets inserted into this
list at a specific position with a written reason -- never as a
side-branch.

Status: `[x]` done, `[~]` in flight, `[ ]` not started. "In flight"
without a verified branch + worktree pinning in
[Where this sprint lives](#where-this-sprint-lives) is **not**
a valid claim -- mark it `[ ]` instead.

1. [x] [S00 - Meta sprint setup](S00-meta-sprint-setup.md)
2. [x] [S01 - Remove v1 settings/policy](S01-remove-v1-settings-policy.md)
3. [x] [S02 - Service settings design](S02-service-settings-design.md)
4. [x] [S03 - Service settings implementation](S03-service-settings-implementation.md)
5. [x] [S04 - Profile design](S04-profile-design.md)
6. [x] [S05 - Profile implementation](S05-profile-implementation.md)
7. [x] [S06-pre - Network contract + confirm wiring](S06-pre-network-contract-and-confirm.md) -- closed. Callback wiring (slices 6a-6e), backoff refactor, adversarial backfill, and slice 6f exit tests all landed; details in [Completed sub-sprints](#completed-sub-sprints). Slice 6f's E2E capsem-doctor ask probe is **deferred** (see [Deferred items](#deferred-items-visible-debt)); slice 7 (`policy_confirm_events` table + remaining deferrals) is tracked separately as future S06-pre+ work.
8. [x] [S06 - Assembly and VM-effective settings](S06-assembly-vm-effective-settings.md) -- six sub-slices closed (parent-chain validation 6.1, layered merge 6.2, resolver trace 6.3, corp directives add/remove/replace 6.4, lock/forbid 6.5, runtime cutover + status/debug exposure 6.6). The in-VM E2E probe is **deferred** (see [Deferred items](#deferred-items-visible-debt)).
9. [x] [S06a - Model request rewrite support](S06a-model-request-rewrite-support.md) -- closed. `evaluate_model_request_policy` now applies the rewrite via `rewrite_model_request_body` against the `request.data` field (unified with the canonical condition vocabulary), forwards the redacted body upstream, and attributes telemetry to the matched rewrite rule. Fail-closed paths: unsupported target, non-UTF-8 body, pattern non-match. The `LastModelPolicyDecision::unsupported_rewrite` shim is removed.
10. [x] [S06b - Legacy allowlist migration + rule ownership locks](S06b-legacy-allowlist-migration-and-rule-ownership.md) -- closed. Inventory found that S01's runtime cutover left the legacy v1 settings registry + allowlist builders as test-only dead code, so "migration" boiled down to deletion plus enriching the v2 model. Nine slices landed: 6b.0 deleted v1 (~12k LOC), 6b.1 added ownership metadata fields, 6b.2 enforced priority tiers (corp `[-1000, -1]`, toggle-derived `0`, user `[1, 999]`, catch-all reserved `1000`), 6b.3 added nestable rules under setting hosts, 6b.4 added `http.read` / `http.write` callbacks, 6b.5 added per-type catch-all rules at priority `1000`, 6b.6 added provider-toggle derived rules, 6b.7 added MCP `allowed_tools` derived rules, 6b.8 added the `ensure_rule_editable` mutation gate. 6b.9 documentation scope captured in [S19 spec](S19-documentation-and-site.md).
11. [x] [S06c - Ablate legacy NetworkPolicy runtime](S06c-ablate-legacy-networkpolicy.md) -- closed. Deleted legacy `NetworkPolicy` + V1 MITM policy hook runtime, collapsed DNS/MITM/process policy authority to the shared Profile V2 `PolicyConfig` handle, and migrated DNS/HTTP tests to equivalent Policy rules.
12. [x] [S06d - Core structure and test boundaries](S06d-core-structure-and-test-boundaries.md) -- closed. DNS behavior tests now live in focused modules; MITM connection, HTTP policy, and model policy buckets are split; production MITM upstream, pipeline construction, and gzip response helpers are internal modules; V1 `NetworkPolicy`/hook source guard added. New engine crate boundaries remain deferred to S08b.
13. [x] [Post-S06 cleanup milestone](#post-s06-cleanup-milestone) -- closed. Branch check is `138 ahead / 0 behind` `origin/main`; singular `policy` runtime rename, S06c V1 runtime ablation, S06d structure/test boundaries, `just smoke`, S07 route proof, S07c live asset boot proof, and S07b admin closeout are green. Remaining release probes are owned by S08b/S15/S18, not Post-S06 cleanup.
14. [x] [S07 - UDS service API](S07-uds-service-api.md) -- closed; first
  foundation slice landed `capsem_proto::metrics` plus
  `ServiceToProcess::GetMetricsSnapshot` /
  `ProcessToService::MetricsSnapshot`; read-only profile list/get/resolve
  routes, profile create/fork/update/delete mutation routes, Profile V2 MCP
  server list/create/delete routes, and rules list/get/create/delete/evaluate
  routes landed. The old `/mcp/{servers,tools,policy}` and `/mcp/tools/*`
  service/CLI/capsem-mcp surface is removed rather than shimmed, along with
  the dead MCP management IPC variants. Rules read/evaluate is
  now hardened with a chained service workflow, generated `http.read`/`http.write`
  dry-run support, boolean catch-all CEL support, and a bounded large-profile
  evaluation test. Rules create/delete now materialize user-profile overrides
  for default built-ins, reject duplicate user rules, and fail closed on locked
  built-in deletes with `rule_is_builtin`.
  Profile/settings composition has additional service coverage for create-id
  collisions across locked roots, selected-profile settings saves, and
  Profile V2 MCP server mutation/lock semantics. The profile file shape is now
  the standard `mcpServers` map, with Capsem-only governance nested under each
  server's `capsem` key; legacy `[mcp.connectors]` is rejected. Closeout added
  typed `GET /confirm/pending`, Profile V2 skills list/create/delete, duplicate
  and inherited-lock coverage, and a chained service proof across profiles,
  skills, MCP servers, rules, evaluate, and confirm listing. HTTP, CLI,
  production confirm resolution, and UI lift remain in S08/S09/S15/S16.
15. [x] [S07a - Profile manifest, packages, and assets](S07a-profile-manifest-assets.md)
    -- closed. Canonical profile catalog/status parser landed in
    `capsem-core::profile_manifest`; typed profile package/tool contracts and
    per-arch VM asset declarations now parse, validate, serialize through
    VM-effective settings, and merge through profile inheritance. The formal
    `schemas/capsem.profile.v2.schema.json` artifact and Rust golden fixture
    validation gate have landed. Python Pydantic v2 profile/manifest models now
    validate JSON through Pydantic, dump JSON through Pydantic, and bridge TOML
    through immediate Pydantic JSON validation. Rust now validates profile JSON
    and TOML payloads against the production schema artifact. Service startup
    now resolves/downloads VM assets from profile declarations, forwards
    expected profile hashes to `capsem-process`, rejects old asset manifests as
    runtime authority, and no longer exposes `assets.manifest.*` service
    settings. `session.db` now records VM/profile/user telemetry identity, and
    VM metadata now carries a profile pin with resolved profile id, signed
    profile revision, profile payload hash, package-contract hash, and pinned
    asset hashes. `capsem profile reconcile-catalog` now accepts either a local
    catalog file or a bounded HTTPS catalog URL, with cleartext HTTP restricted
    to loopback development/test hosts. Typed `[profile_catalog]` service
    settings now persist the catalog URL, profile payload public key, and check
    interval; service startup schedules the same reconcile path and logs
    summary counts. `GET /profiles/catalog`, `GET /profiles/{id}/revisions`,
    `capsem profile catalog [--json]`, and `capsem profile revisions <id>
    [--json]` now expose configured catalog source, persisted manifest
    presence, current/installed revisions, and lifecycle status.
    `POST /profiles/{id}/revisions/{install,update,remove}` and `capsem
    profile install|update|remove <id> [--revision <rev>] [--json]` now add
    selected revision lifecycle actions. Fresh VM create now accepts explicit
    profile/revision selection, reconciles that profile's assets before spawn,
    attaches the selected VM-effective profile, and refuses incomplete
    installed revision payloads. Later S07c/S07b/S08 work closed the remaining
    production gaps: duplicate reconcile sharing, cleanup/update race proof,
    structured check/download logs, status/debug/gateway provenance, real
    profile-asset boot proof, image inventory + doctor-bundle in-guest
    verification, profile-backed release-image boot gate, and HTTP catalog/
    revision/profile-state mirroring. UI-rich clients and deeper post-engine
    provenance are assigned to S16/S11/S18.
16. [x] [S07c - Profile asset update orchestration](S07c-profile-asset-update-orchestration.md)
  -- manual service reconcile endpoint, `capsem update --assets` service
  trigger, checked-at/profile provenance status propagation, structured
  lifecycle logs, service debug Profile V2 asset-health reporting, old Rust
  asset-manifest parser/loader/downloader cleanup, and duplicate-download /
  active-cleanup race proof have landed. First-use VM create/run now drives the
  Profile V2 reconciler before spawn, and source/fork/persist derive boot
  asset identity from the profile pin. Asset health now includes installed
  profile payload hash plus redacted per-asset source/hash metadata. A chained
  service-level operator test now proves reconcile, `/setup/assets`, `/list`,
  debug report, and `/service-logs` agree after a real local profile asset
  download. The closing E2E probe now reconciles real profile-declared VM
  assets into an empty cache through `capsem update --assets`, boots a real VM,
  execs inside it, and verifies `capsem info --json` reports the installed
  profile revision pin.
17. [x] [S07d - Service settings schema and admin contract](S07d-service-settings-schema-admin-contract.md)
    -- closed on 2026-05-20. First contract slice landed:
    `ServiceSettingsV2` Pydantic models, Pydantic-only JSON/TOML helpers,
    committed `schemas/capsem.service-settings.v2.schema.json`, minimal/complete
    valid fixtures, invalid fixture coverage for unknown fields/catalog/roots/
    telemetry/remote-policy/credentials/assets, and Rust/Python fixture parity.
    Verification: `uv run python -m pytest tests/test_service_settings.py -q`
    passed with 11 tests; `cargo test -p capsem-core service_settings_json
    --lib` passed with 2 tests. Second slice landed `capsem-admin settings
    schema|validate|doctor`, typed Pydantic JSON reports, TOML/JSON validation,
    and installed console-script smoke coverage. Verification: `uv run python
    -m pytest tests/test_admin_cli.py tests/test_service_settings.py -q` passed
    with 17 tests; `uv run capsem-admin settings validate
    schemas/fixtures/service-settings-v2-complete.json` and `uv run
    capsem-admin settings doctor
    schemas/fixtures/service-settings-v2-complete.json --json` passed.
    Third slice aligned Python's default profile user roots with Rust's
    `CAPSEM_HOME` / `$HOME/.capsem` contract and added a shared
    `service-settings-v2-defaults.json` fixture consumed by Python and Rust.
    Verification: `uv run python -m pytest tests/test_service_settings.py
    tests/test_admin_cli.py -q` passed with 18 tests; `cargo test -p
    capsem-core service_settings --lib` passed with 21 tests. Closeout docs now
    explain service settings versus profiles, the `capsem.service-settings.v2`
    schema, `capsem-admin settings` usage, and the split from the guest/UI
    descriptor schema. Admin ergonomics follow-up added `capsem-admin settings
    init` with Pydantic-generated JSON/TOML drafts, profile-root options,
    `--default-profile`, `--assets-dir`, overwrite protection, and TOML
    round-trip validation through `tomli-w`, plus parity tests proving init
    JSON matches init TOML after reparsing. Verification: `uv run python -m
    pytest tests/test_service_settings.py tests/test_admin_cli.py -q` passed
    with 34 tests.
18. [x] [S07b - Capsem admin tooling and profile-derived images](S07b-capsem-admin-tooling.md)
    -- closed on 2026-05-21 after S07d closeout. First slice landed
    `capsem-admin profile schema` and `capsem-admin profile validate
    <profile.json|profile.toml> [--json]`, using the existing Profile V2
    Pydantic model and typed JSON report output. Verification: `uv run python
    -m pytest tests/test_admin_cli.py tests/test_profiles.py -q` passed with 24
    tests; installed console-script smoke for schema, validate, and JSON report
    passed against `schemas/fixtures/profile-v2-valid.json`. Later slices
    closed profile init, image plan/build/verify, manifest generate/check/sign,
    bootstrap/release install proof, policy/detection admin models after S08a,
    and `capsem-admin doctor`. Second slice added the Profile V2 `editable`
    block with section-level
    gates for `general`, `appearance`, `ai`, `mcpServers`, `skills`,
    `packages`, `tools`, `vm`, `security_capabilities`, and `security_rules`.
    Service routes now enforce those locks for skills, MCP servers, rules,
    settings-save rule updates, and whole-profile `PUT`; forks preserve the
    lock map, and profile `PUT` cannot mutate the `editable` map itself.
    Verification: `cargo test -p capsem-service profile --bin capsem-service`
    passed with 64 tests; `cargo test -p capsem-core profile_parse --lib`
    passed with 4 tests; `cargo test -p capsem-core profile_payload --lib`
    passed with 11 tests; `uv run python -m pytest tests/test_profiles.py
    tests/test_admin_cli.py -q` passed with 26 tests; `uv run capsem-admin
    profile schema | rg -n 'editable|mcpServers|security_rules'` confirmed the
    admin schema exposes the editability contract. Third slice added
    `capsem-admin profile init <profile-id>` with Pydantic-generated Profile V2
    JSON/TOML drafts, all-architecture VM asset placeholders, package/tool
    contract defaults, section editability defaults, optional file output,
    overwrite protection, and parity tests proving init JSON matches init TOML
    after reparsing. Verification: `uv run python -m pytest
    tests/test_profiles.py tests/test_admin_cli.py -q` passed with 36 tests;
    installed console-script smoke proved `profile init` output validates with
    `profile validate`. Fourth slice added `capsem-admin image plan <profile>`
    with typed `capsem.image-plan.v1` output derived from Profile V2 package/
    tool contracts, VM resources, selected arches, declared per-arch assets, and
    a package-contract BLAKE3 hash. `--arch all` is the default, single-arch
    narrowing works for CI shards, and missing selected-arch assets fail closed.
    Verification: `uv run python -m pytest tests/test_image_plan.py
    tests/test_admin_cli.py -q` passed with 26 tests; installed console-script
    smoke proved JSON and TOML profile inputs produce valid image plans. Fifth
    slice added `capsem-admin image verify <profile> --assets-dir <dir>` with a
    typed `capsem.image-verification.v1` report derived from the image plan.
    The verifier checks local per-arch kernel/initrd/rootfs assets for
    existence, declared size, and BLAKE3 hash, supports default `--arch all`
    plus single-arch narrowing, and exits non-zero on missing or mismatched
    assets. Verification: `uv run python -m pytest tests/test_image_verify.py
    tests/test_image_plan.py tests/test_admin_cli.py -q` passed with 32 tests.
    Sixth slice added `capsem-admin manifest check <manifest> --fast` with a
    typed `capsem.manifest-check.v1` report. The checker validates the Profile
    V2 catalog manifest through Pydantic, checks remote profile payload and
    signature URLs with HTTP(S) `HEAD`, verifies local `file://` profile payload
    BLAKE3 hash plus id/revision parity, and exits non-zero on missing local
    signatures, hash drift, invalid payloads, unsupported schemes, unexpected
    profile content types, or remote HTTP errors. Verification: `uv run python
    -m pytest tests/test_manifest_check.py -q` passed with 4 tests.
    Seventh slice added `capsem-admin manifest check <manifest> --download`
    with optional `--download-dir`, GET-based download of every referenced
    profile payload/signature and every profile-declared VM asset/signature,
    profile payload BLAKE3 hash and id/revision checks, non-empty signature
    byte checks, and VM asset size/BLAKE3 verification. Fast mode remains
    HEAD-only; download mode records downloaded paths in the typed report.
    Remaining manifest scope includes cryptographic signature verification,
    manifest generation, and manifest signing. Verification: `uv run python -m
    pytest tests/test_manifest_check.py tests/test_profiles.py
    tests/test_manifest.py tests/test_service_settings.py tests/test_image_plan.py
    tests/test_image_verify.py tests/test_admin_cli.py -q` passed with 117
    tests. Eighth slice added `capsem-admin manifest generate --profiles <dir>`
    to create typed Profile V2 catalog manifests from local JSON/TOML profile
    payloads. Generation validates through Pydantic, hashes exact payload bytes,
    derives profile payload and `.minisig` URLs, rejects duplicate
    profile/revision pairs, supports hosted `--base-url`, lifecycle
    `--status profile@revision=...`, and `--current profile=revision`
    overrides, and produces manifests immediately checkable by `manifest check
    --fast`. Verification: `uv run python -m pytest
    tests/test_manifest_generate.py -q` passed with 4 tests. Ninth slice added
    minisign-backed `capsem-admin manifest sign`, `manifest verify-signature`,
    and `manifest check --download --pubkey` cryptographic verification for
    downloaded profile payload signatures and VM asset signatures. Missing
    `minisign` fails closed; Linux/corp admin docs now call out installing the
    distro `minisign` package. Verification: `uv run python -m pytest
    tests/test_manifest_crypto.py tests/test_manifest_generate.py
    tests/test_manifest_check.py tests/test_profiles.py tests/test_manifest.py
    tests/test_service_settings.py tests/test_image_plan.py
    tests/test_image_verify.py tests/test_admin_cli.py -q` passed with 124
    tests, including real throwaway minisign key generation, valid
    profile/asset/manifest signature verification, and bad-signature rejection.
    Tenth slice added a developer bootstrap proof for `capsem-admin`: after
    `uv sync`, `bootstrap.sh` runs `uv run capsem-admin --version`; bootstrap
    tests pin the pyproject script entry point, smoke ordering, and real uv
    entrypoint execution. Verification: `uv run python -m pytest
    tests/capsem-bootstrap/test_dev_setup.py -q` passed with 8 tests and 1
    existing setup-sentinel skip.
    Eleventh slice added release package layout proof for `capsem-admin`:
    `scripts/prepare-admin-cli.sh` produces a relocatable wrapper plus
    `capsem-admin-python/`, macOS `.pkg` and Linux `.deb` packaging require
    both pieces, postinstall exposes the wrapper, `.deb` payload verification
    checks the admin payload, and release workflow policy tests prove the
    payload is prepared before OS packages are assembled. `uv run python -m
    pytest tests/test_package_scripts.py tests/test_verify_deb_payload.py
    tests/test_release_workflow_policy.py -q` passed with 37 tests; `uv run
    python -m pytest tests/test_repack_deb.py -q` skipped 7 Linux-only
    `dpkg-deb` tests on this macOS host; real temp-payload
    `scripts/prepare-admin-cli.sh` smoke plus wrapper `--version` passed with
    the uv interpreter.
    Twelfth slice added `capsem-admin image build-workspace <profile> --out
    <dir>` and the typed `capsem.image-workspace.v1` report. It materializes
    source profile TOML, image-plan JSON, build/manifest/vm resources TOML, and
    apt/Python/npm package TOML directly from the Profile V2 package/tool
    contract without reading repo `guest/config`; tests parse the generated
    workspace with `load_guest_config`. Verification: `uv run python -m pytest
    tests/test_image_workspace.py tests/test_image_plan.py
    tests/test_admin_cli.py -q` passed with 30 tests.
    Thirteenth slice fixed release SBOM attestation so the SPDX 2.3
    `capsem-sbom.spdx.json` predicate is attached to both `release-artifacts/*.pkg`
    and `release-artifacts/*.deb`. Release policy tests now pin the `Attest
    SBOM` step, and build-verification docs clarify that the current cargo-sbom
    artifact is the Rust host SBOM while profile-derived guest package/tool
    SBOM remains S07b image-verification work. Verification: `uv run python -m
    pytest tests/test_release_workflow_policy.py -q` passed with 26 tests;
    docs build passed with `pnpm run build`.
    Fourteenth slice added `capsem-admin image build <profile>` as the public
    profile-derived image build entrypoint. It materializes the generated
    workspace, parses it through `load_guest_config`, routes selected
    arches/templates into the existing Docker builder, supports `--dry-run`,
    and emits typed `capsem.image-build.v1` JSON embedding the workspace report.
    Verification: `uv run python -m pytest tests/test_image_workspace.py
    tests/test_image_plan.py tests/test_admin_cli.py -q` passed with 33 tests;
    installed CLI smoke proved `capsem-admin image build --dry-run --json`.
    Fifteenth slice added the required Profile V2 `ui` contract with
    `everyday` and `coding` enum values across Python/Pydantic, JSON Schema,
    Rust profile/effective-settings parsing, fixtures, and signed fixture
    verification. It added `capsem-admin profile init-builtins` and committed
    generated `everyday-work` and `coding` base profile TOML drafts under
    `config/profiles/base/`. It also made `scripts/build-assets.sh`,
    `just build-assets`, `just build-kernel`, and `just build-rootfs`
    profile-aware so selected asset builds can route through
    `capsem-admin image build`; the unprofiled guest-config fallback remains
    only until the release profile preserves the full existing guest package
    set. Verification: `uv run python -m pytest tests/test_profiles.py
    tests/test_admin_cli.py tests/test_image_workspace.py
    tests/test_build_assets_script.py -q` passed with 53 tests;
    `cargo test -p capsem-core --test profile_schema` passed with 6 tests;
    `cargo test -p capsem-core settings_profiles:: --lib` passed with 143
    tests; `cargo test -p capsem-service --no-run` compiled the service test
    targets; `uv run capsem-admin profile validate config/profiles/base/*.toml`
    and `uv run capsem-admin image build config/profiles/base/coding.profile.toml
    --arch arm64 --template rootfs --dry-run --json` proved the generated
    profiles validate and feed the image build entrypoint.
    Sixteenth slice changed built-in profile generation from placeholder
    drafts to a typed `GuestImageConfig` bridge. `capsem-admin profile
    init-builtins --guest-dir guest` now derives `everyday-work` and `coding`
    from the current rich `guest/config` package/tool/resource inputs, keeping
    the two built-ins identical except for identity and `ui` while preserving
    unpinned package intent as `*` in Profile V2 and rendering it back to
    unpinned package specs for the existing image builder. Verification:
    `uv run python -m pytest tests/test_profiles.py tests/test_admin_cli.py
    tests/test_image_workspace.py tests/test_build_assets_script.py -q` passed
    with 54 tests; `cargo test -p capsem-core --test profile_schema` passed
    with 6 tests; `cargo test -p capsem-core settings_profiles:: --lib`
    passed with 143 tests; `cargo test -p capsem-service --no-run`,
    `cargo fmt --check`, `uv run python -m compileall src/capsem`,
    profile validation for both generated base profiles, and a coding profile
    `capsem-admin image build --dry-run --json` smoke passed.
    Seventeenth slice removed the unprofiled VM asset build fallback from live
    build lanes. `scripts/build-assets.sh` now requires `--profile`, Justfile
    `build-assets`/`build-kernel`/`build-rootfs` default to
    `config/profiles/base/coding.profile.toml`, and PR install CI passes that
    profile explicitly before `just test-install`. Verification: `uv run
    python -m pytest tests/test_build_assets_script.py
    tests/test_ci_codesign_runner.py tests/test_release_workflow_policy.py -q`
    passed with 42 tests; the expanded focused gate with admin/image workspace
    tests passed with 75 tests; `just --dry-run build-assets arm64` showed the
    default generated profile path; docs build passed with `pnpm --dir docs run
    build`; and a static `rg` check found no remaining `capsem-builder build
    guest/` live caller in Justfile/scripts/CI/tests.
    Eighteenth slice added typed package/tool inventory checking to
    `capsem-admin image verify`: optional `--inventory` reads a
    `capsem.image-inventory.v1` Pydantic JSON artifact, compares profile
    apt/Python/node package contracts and required tools against the
    image-derived inventory, accepts `*` as present-any-version, fails closed on
    missing or exact-version mismatches, and reports per-contract rows in the
    existing `capsem.image-verification.v1` output. Verification: `uv run
    python -m pytest tests/test_image_verify.py -q` passed with 10 tests.
    Nineteenth slice made rootfs builds generate the verifier input instead of
    expecting hand-produced JSON: after Docker build, `extract_image_inventory`
    runs inside the built container, collects apt/Python/node package versions
    and tool versions, validates the bytes with
    `ImageInventory.model_validate_json()`, and writes canonical
    `image-inventory.json` beside `tool-versions.txt`. `capsem-doctor --version`
    now exits before pytest so the required guest tool can be inventoried.
    Verification: `uv run python -m pytest tests/test_docker.py
    tests/test_image_verify.py tests/test_image_workspace.py
    tests/test_admin_cli.py -q` passed with 171 tests; `uv run python -m
    compileall src/capsem` passed.
    Twentieth slice made inventory verification architecture-scoped:
    `capsem-admin image verify` now auto-discovers
    `<assets-dir>/<arch>/image-inventory.json`, emits `inventories[]` contract
    rows by arch, supports `--inventory FILE` only with a single `--arch`, and
    supports inventory directories for all-arch alternate layouts. Missing
    inventory for any selected arch fails closed instead of downgrading to
    asset-only proof. Verification: `uv run python -m pytest
    tests/test_image_verify.py -q` passed with 13 tests, and docs now list
    `image-inventory.json` in every arch asset directory.
    Twenty-first slice added typed in-VM probe ingestion:
    `capsem-admin image verify --doctor-bundle <tar>` reads the
    `capsem-doctor --bundle` JUnit result without extracting archive contents,
    emits typed `capsem_doctor_bundle` probe rows, and fails verification on
    in-VM diagnostic failures or missing JUnit evidence. Verification:
    `uv run python -m pytest tests/test_image_verify.py -q` passed with 17
    tests; docs now describe doctor bundles as image probe evidence.
    Twenty-second slice added `capsem-admin image sbom`, generating SPDX 2.3
    guest-image SBOM JSON from typed per-arch image inventories. Single-arch
    output streams one SPDX document; all-arch output writes
    `<out-dir>/<arch>/guest-sbom.spdx.json`. SPDX names/namespaces include
    profile id, revision, arch, and package-contract hash, and apt/Python/node
    rows carry package-manager purl references. Verification:
    `uv run python -m pytest tests/test_image_sbom.py -q` passed with 5 tests.
    Twenty-third slice added the release-image boot gate: the profile-backed
    E2E test reconciles selected assets, runs `capsem doctor --fast --bundle`,
    requires `doctor-latest.tar`, and feeds that doctor bundle plus the
    host-arch `image-inventory.json` into `capsem-admin image verify`.
    `CAPSEM_REQUIRE_ARTIFACTS=1` now also requires the host-arch
    `image-inventory.json`, preventing artifact-gated runs from silently
    skipping the package/tool proof; `_check-assets` now rebuilds when that
    inventory is missing. Verification: `uv run python -m pytest
    tests/capsem-e2e/test_profile_asset_boot.py -q` passed locally with one
    boot test and one asset-dependent skip; `uv run python -m pytest
    tests/test_image_verify.py tests/test_image_sbom.py tests/test_leak_detection.py
    -q` passed.
    Twenty-fourth slice added typed `capsem-admin policy validate|schema` and
    `capsem-admin detection validate|schema` commands backed by strict
    Pydantic `capsem.policy-pack.v1` and `capsem.detection-pack.v1` models plus
    committed schema artifacts; detection envelopes support YAML. Verification: `uv run python -m pytest
    tests/test_security_packs.py tests/test_admin_cli.py tests/test_profiles.py
    tests/test_service_settings.py -q` passed with 65 tests. Remaining
    policy/detection admin work: `detection compile|check`, pySigma/Rust parity
    fixtures, and docs/release proof.
    Twenty-fifth slice added pySigma-backed `capsem-admin detection compile`
    into typed `capsem.detection.ir.v1` plus `capsem-admin detection check`
    over normalized SecurityEvent JSONL fixtures; unsupported Sigma conditions
    and unsupported subset features fail closed. Verification:
    `uv run python -m pytest tests/test_security_packs.py -q` passed with 15
    tests; `uv run python -m pytest tests/test_security_packs.py
    tests/test_admin_cli.py tests/test_profiles.py tests/test_service_settings.py
    -q` passed with 71 tests. Remaining policy/detection admin work: Rust
    parity fixtures and docs/release proof.
    Twenty-sixth slice added `capsem-core::security_packs` with strict Rust
    Detection IR V1 schema validation, serde parsing, normalized SecurityEvent
    parsing, exact-match evaluator support, and golden fixture parity with the
    Python `capsem-admin detection compile` output. Verification:
    `cargo test -p capsem-core --test security_packs` passed with 5 tests;
    `uv run python -m pytest tests/test_security_packs.py -q` passed with 16
    tests; `uv run python -m pytest tests/test_security_packs.py
    tests/test_admin_cli.py tests/test_profiles.py tests/test_service_settings.py
    -q` passed with 72 tests; `cargo test -p capsem-core --test profile_schema`
    passed with 6 tests; `cargo clippy -p capsem-core --test security_packs --
    -D warnings` passed. Remaining policy/detection admin work: docs/release
    proof.
    Twenty-seventh slice added corp-facing Admin CLI, Enforcement, and
    Detection Format docs; the detection docs explicitly require pySigma
    validation and Detection IR instead of an ad hoc Sigma validator. The docs
    also distinguish PyPI operator install from editable `uv` development
    usage. Verification: `uv run python -m pytest tests/test_admin_docs.py -q`
    passed with 4 tests; `uv run python -m pytest tests/test_admin_docs.py
    tests/test_security_packs.py -q` passed with 20 tests; docs build passed
    with `pnpm run build`.
    Twenty-eighth slice added only the shared agent-client plumbing:
    `bootstrap.sh` now creates non-destructive shared `skills/` symlinks for
    Claude Code, Gemini CLI, Codex, and Cursor, and the skills documentation
    references those bootstrap-managed links. No new admin skill content ships
    in this slice because the workflow is already captured in the code, docs,
    and sprint contracts. Verification: `uv run python -m pytest
    tests/capsem-bootstrap/test_dev_setup.py -q` passed with 10 tests and 1
    existing setup-sentinel skip; docs build passed with `pnpm run build`.
    Twenty-ninth slice closed S07b with typed `capsem-admin doctor` output:
    the admin doctor checks local toolchain/source readiness and optional
    Profile V2 image-plan derivation without reading `guest/config` as
    operator input. Shared doctor output and fix hints now point to
    `capsem-admin doctor` / `capsem-admin profile init-builtins`, and
    `tests/test_admin_hygiene.py` guards S07b admin contract modules against
    raw `json.loads` / `json.dumps` command-boundary regressions. Verification:
    `uv run python -m pytest tests/test_admin_cli.py tests/test_admin_hygiene.py
    tests/test_doctor.py tests/test_cli.py::TestDoctorCommand
    tests/test_admin_docs.py -q` passed with 70 tests. Final focused S07b gate:
    `uv run python -m pytest tests/test_admin_cli.py tests/test_admin_hygiene.py
    tests/test_profiles.py tests/test_service_settings.py tests/test_image_plan.py
    tests/test_image_workspace.py tests/test_image_verify.py
    tests/test_image_sbom.py tests/test_manifest_generate.py
    tests/test_manifest_check.py tests/test_manifest_crypto.py
    tests/test_security_packs.py tests/test_admin_docs.py tests/test_doctor.py
    tests/test_cli.py::TestDoctorCommand tests/capsem-bootstrap/test_dev_setup.py
    -q` passed with 174 tests and 1 existing setup-sentinel skip; `uv run
    python -m compileall src/capsem` and docs build `pnpm run build` passed.
    S07b is closed.
19. [~] [S08 - HTTP gateway API](S08-http-gateway-api.md)
    -- started by explicit user direction after S07 closeout. First gateway
    contract slice landed for Profile V2 catalog/revision routes, profile
    CRUD/resolve, skills, standard `mcpServers` server management,
    rules/evaluate, confirm-pending read, profile-selected VM create response
    payloads, `/status` profile/asset provenance, `/setup/assets`
    profile-scoped download progress, `/debug/report` Profile V2 asset
    provenance, exact typed-error passthrough, and service debug-report gateway
    runtime mismatch diagnostics. The live HTTP proof now starts real
    capsem-service plus real capsem-gateway against a Profile V2 asset fixture,
    proves selected-profile `/provision` downloads verified assets before boot,
    execs through the gateway, and reports the same pinned profile through
    `/info/{vm_id}`. Adversarial typed-error coverage now proves exact
    status/body passthrough for malformed profile create, locked skill/MCP/rule
    mutations, invalid rule evaluation, asset cleanup while updating, and
    revoked revision install. Remaining: S15 confirm resolution/stream once S15
    makes that production route real.
20. [x] [S08a - Rule abstraction and detection architecture](S08a-rule-abstraction-detection-architecture.md)
    -- inserted during the 2026-05-19 regroup. Decide real CEL enforcement,
    real Sigma-compatible detection, profile-owned enforcement/detection packs,
    and whether Capsem enforcement rules and Sigma-style detection rules are
    separate families. Update logging, telemetry, plugins, rule UI, Confirm UX, and docs
    before those surfaces freeze around the wrong abstraction. This sprint also
    defines `capsem-admin` validation/schema requirements and VM health/OTel
    attribution for detection findings plus model provider/model/cost usage.
    First decision slice landed on 2026-05-21: enforcement and detection are
    separate profile-owned rule families; enforcement uses real CEL through the Rust
    `cel` crate family; Sigma is a detection authoring/import format, not a
    blocking language; detection compiles into Capsem normalized detection IR
    and attaches typed findings to resolved security events before
    telemetry/audit/logging/timeline sinks.
    Second slice drafted the concrete contract names and shapes:
    `capsem.policy-pack.v1`, `capsem.detection-pack.v1`,
    `capsem.detection.ir.v1`, `DetectionFinding`, `SecurityEvent`,
    `ResolvedSecurityEvent`, Sigma logsource mapping, and
    `capsem-admin enforcement|detection validate/schema/compile/check` command
    requirements. S07b, S12, S13, S14, S15, S16a, and S19 now reference those
    contracts instead of generic "real CEL/Sigma later" placeholders.
    Third slice closed the ADR with rejected alternatives, implementation
    ordering, and a testing matrix. S08a is now done; next implementation work
    is S07b admin schemas/commands followed by S08b Rust event/security-engine
    contracts.
    Fourth regroup slice split the public runtime surfaces into
    `/enforcement/*` and `/detection/*`, made backtest first-class for both
    families, kept detection hunt forensic/detection-only, and clarified that
    `capsem-admin` must work offline while S08b service routes own runtime
    validation/compile/live registry/stats/backtest/hunt.
21. [~] [S08b - Security event engine, Network Engine, File Engine, and Process Engine](S08b-security-event-engine-and-file-engine.md)
    -- inserted during the 2026-05-19 engine-boundary regroup. Split runtime
    activity handling into Network Engine, File Engine, Process Engine, Security
    Engine, and Resolved Event Emitter contracts/crates. File writes, deletes,
    snapshots, restores, observe-only file behavior, exec chains, and
    process/audit attribution must feed the same normalized security event
    pipeline as network/DNS/MCP/model activity without collapsing file and
    process mechanics into one engine. Security Engine consumes S08a's real
    CEL/Sigma/profile-owned rule-pack decisions. Session DB moves toward a
    canonical resolved-event journal with existing domain tables treated as
    emitter-written projections. Conversation Engine capture and the structured
    `/timeline/{id}` read API become part of the canonical session DB story.
    Model/MCP events must use the sidecar
    [Canonical AI Interaction Evidence](S08-side-canonical-ai-interaction-evidence.md)
    substrate so provider-specific parsers feed stable evidence for model
    requests, responses, tool calls, tool results, MCP executions, usage, parse
    status, and linkage. OpenAI, Anthropic, and Google/Gemini are first-slice
    providers; Bedrock is explicitly later adapter coverage.
    S08b must also add the missing host/service AI attribution contract:
    `SourceEngine::HostAi` or equivalent, an explicit attribution scope on
    events/quota dimensions, logger/resolved-event fields for accounting owner,
    and fixture tests proving a host-originated VM naming/session summary call
    with `vm_id` correlation does not increment VM-owned model, MCP, token,
    cost, quota, or health counters.
    S08b must add service-owned runtime `/enforcement/*` and `/detection/*`
    routes for validate, compile, backtest, live add/update/delete/list, stats,
    plus detection hunt. Supported Sigma detection constructs should lower into
    the CEL-backed predicate plan wherever exact; unsupported constructs fail
    closed with typed diagnostics. Backtest returns up to 100 matched event
    rows by default, deduped by simple evidence signature, with full local
    evidence and event refs.
    First implementation slice started S08b: added the
    `capsem-security-engine` crate as the shared contract home for
    `SecurityEvent`, `ResolvedSecurityEvent`, `DetectionFinding`,
    `SecurityAction`, reserved `Throttle`, resolved-event steps, family
    subjects, and quota dimensions. Verification:
    `cargo test -p capsem-security-engine` passed with 3 tests covering
    identity/quota extraction, throttle/rate-limit-step roundtrip, and
    unknown-field rejection. Missing/deferred for this slice: engine wiring,
    runtime registries, session.db emitter, service routes, E2E/VM, telemetry,
    and performance remain explicit S08b/S08d work.
    Second implementation slice wired the existing Rust Detection IR evaluator
    to consume `capsem-security-engine::SecurityEvent` directly, preserving the
    existing normalized JSON fixture path while proving a real HTTP event from
    the new contract matches the Sigma-derived metadata-access rule.
    Verification: `cargo test -p capsem-core --test security_packs` passed with
    6 tests.
    Third contract-hardening slice added parent event, stream, activity,
    sequence, source-engine, and enforceability fields to the shared
    `SecurityEventCommon` contract and pinned those values in quota/correlation
    extraction tests. Verification: `cargo test -p capsem-security-engine` and
    `cargo test -p capsem-core --test security_packs` passed.
    Fourth contract slice added `schema_version` fields, enforcement/detection
    `SecurityPackIdentity` pins, and committed JSON fixtures for DNS, HTTP, MCP,
    model, file, process, credential, VM lifecycle, profile, conversation, and
    snapshot events plus a resolved-event finding fixture. Verification:
    `cargo test -p capsem-security-engine` passed with 5 tests and
    `cargo test -p capsem-core --test security_packs` passed with 6 tests.
    Fifth contract slice added the first resolved-event emitter abstraction:
    required/best-effort sink requirements, delivery bookkeeping, sink failure
    recording on `ResolvedSecurityEvent`, and required-sink failure reporting.
    Verification: `cargo test -p capsem-security-engine` passed with 7 tests
    and `cargo test -p capsem-core --test security_packs` passed with 6 tests.
    Sixth contract slice added shared backtest result types with full event
    refs, matched fields, mismatch/error outcomes, default 100-row limit, and
    evidence-signature deduplication for diverse local evidence. Verification:
    `cargo test -p capsem-security-engine` passed with 9 tests and
    `cargo test -p capsem-core --test security_packs` passed with 6 tests.
    Seventh contract slice added a compile-first runtime rule registry with
    source metadata, compiled generation, delete, match stats, and previous-plan
    preservation when an update fails compilation. Verification:
    `cargo test -p capsem-security-engine` passed with 11 tests and
    `cargo test -p capsem-core --test security_packs` passed with 6 tests.
    Eighth contract-correction slice amended `SecurityEvent` as future
    deterministic plugin ABI groundwork: event-in/event-out callbacks carry
    labels, bounded context/trace history, findings, first-class decisions
    (`allow`, `ask`, `block`, `rewrite`, `throttle`), and declarative mutations.
    Rust validates mutation targets and projects the final event to internal
    transport behavior; plugins do not return `HookOutcome`. Verification:
    `cargo test -p capsem-security-engine` passed with 15 tests and
    `cargo test -p capsem-core --test security_packs` passed with 6 tests.
    Ninth plugin-determinism slice added canonical BLAKE3 event hashes,
    `PluginIdentity`, plugin transform records, immutable core field validation
    (`schema_version`, `common`, `subject`, `context`, `trace`), and guards that
    plugin output cannot drop prior labels/findings/mutations. Verification:
    `cargo test -p capsem-security-engine` passed with 18 tests and
    `cargo test -p capsem-core --test security_packs` passed with 6 tests.
    Tenth fixture-sync slice updated committed security-event/resolved-event
    JSON fixtures to include plugin-facing context, trace labels, decisions,
    findings, and declarative mutations. Verification:
    `cargo test -p capsem-security-engine` passed with 18 tests and
    `cargo test -p capsem-core --test security_packs` passed with 6 tests.
    Eleventh audit-link slice added `plugin_transforms` to
    `ResolvedSecurityEvent` plus a `PluginCallback` resolved-event step kind so
    session DB/telemetry can tie plugin identity to input/output event hashes.
    Verification: `cargo test -p capsem-security-engine` passed with 18 tests
    and `cargo test -p capsem-core --test security_packs` passed with 6 tests.
    Twelfth side-sprint contract slice executed
    [Canonical AI Interaction Evidence](S08-side-canonical-ai-interaction-evidence.md):
    added strict canonical AI evidence structs/enums, `SourceEngine::HostAi`,
    attribution scope/origin/accounting-owner fields on security events and
    quota dimensions, optional model/MCP evidence on security subjects,
    OpenAI/Anthropic/Gemini/host AI evidence fixtures, and tests proving
    host-attributed AI can correlate to a VM/session without charging VM-owned
    accounting. Verification: `cargo test -p capsem-security-engine` passed
    with 21 tests and `cargo test -p capsem-core --test security_packs` passed
    with 6 tests.
    Thirteenth parser-adapter slice added
    `capsem-core::net::ai_traffic::evidence`, projecting existing request
    metadata plus OpenAI/Anthropic/Gemini stream summaries into canonical
    `ModelInteractionEvidence`. The adapter preserves provider/API family,
    path-derived Gemini model names, usage/cost micros, tool-call origin,
    argument status, returned tool results, raw-shape version, and host-vs-VM
    attribution. Verification: `cargo test -p capsem-core
    ai_traffic::evidence` passed with 5 tests.
    Fourteenth telemetry-storage slice rejected the opaque
    `model_calls.ai_evidence` JSON-column approach and added normalized,
    indexed session DB tables for canonical AI interaction evidence:
    interactions, usage details, content blocks, model tool calls, model tool
    results, and MCP execution evidence. MITM model-call telemetry now attaches
    canonical evidence before write, and tests prove both the production hook
    evidence and queryable relational storage. Verification:
    `cargo test -p capsem-logger` passed with 227 tests, `cargo test -p
    capsem-core ai_traffic::evidence` passed with 5 tests, and `cargo test -p
    capsem-core telemetry_hook` passed with 8 tests.
    Fifteenth Lannister-grade hardening slice replaced generic enum serde
    persistence with an explicit SQL enum text trait for canonical AI evidence
    enums and added SQLite `CHECK` constraints to the evidence ledger tables.
    Verification: `cargo test -p capsem-logger` passed with 229 tests,
    including enum spelling parity and invalid enum DB constraint tests;
    `cargo test -p capsem-core telemetry_hook` passed with 8 tests.
    Sixteenth linkage slice connected framed MCP `tools/call` telemetry to
    canonical model tool-call evidence when trace id and normalized tool name
    agree, records explicit link status for unmatched/ambiguous executions,
    and backfills the legacy `tool_calls.mcp_call_id` projection. The model
    AI trace state now prefers ambient Capsem trace ids when starting a new
    tool chain so model/MCP rows can share a join key. Verification:
    `cargo test -p capsem-logger` passed with 230 tests, `cargo test -p
    capsem-core telemetry_hook` passed with 8 tests, and `cargo test -p
    capsem-core ai_traffic::evidence` passed with 5 tests.
    Seventeenth projection slice added Security Engine quota/status dimensions
    for linked canonical AI evidence: model API family, parse/evidence status,
    tool-call/result/MCP-execution counts, linked MCP tool-call counts, MCP
    link status, and linked model ids. Verification: `cargo test -p
    capsem-security-engine` passed with 22 tests, `cargo test -p capsem-core
    --test security_packs` passed with 6 tests, `cargo test -p capsem-logger
    ai_evidence_is_stored_in_queryable_tables` passed, and `cargo test -p
    capsem-core ai_traffic::evidence` passed with 5 tests.
    Eighteenth side-sprint closeout slice audited the canonical AI evidence
    acceptance criteria and filled the remaining fixture/proof gaps: OpenAI
    Responses has a parser-adapter test, the committed evidence fixture now
    covers orphan model tool calls, orphan MCP executions, and provider
    unknown-field drift, and the side sprint is closed at the contract,
    adapter, session-ledger, and policy-projection layer. Verification:
    `cargo test -p capsem-security-engine
    canonical_ai_evidence_fixture_covers_first_slice_providers_and_host_accounting`
    passed, `cargo test -p capsem-core
    openai_responses_path_projects_responses_api_family` passed, and the
    closeout gate re-ran the focused side-sprint suites before commit. Full
    VM-originated E2E, live resolved-event journal, timeline, runtime
    enforcement/detection routes, and performance remain S08b/S08d work.
    Nineteenth Security Engine core slice added the first ordered runtime
    pipeline shell inside `capsem-security-engine`: preprocessors,
    enforcement, Security Engine-owned confirm, detection, postprocessors, and
    resolved-event construction now execute in that order, detection findings
    attach to both the event and resolved event before emission, and
    enforcement errors fail closed as `SecurityAction::Error` with an error
    step. Verification: `cargo test -p capsem-security-engine` passed with
    24 tests, including the new ordered-pipeline and fail-closed enforcement
    tests. Still missing for S08b.2: real CEL adapter replacement,
    Sigma-to-runtime detection lowering, runtime service routes, historical
    hunt over the session journal, and VM/runtime integration.
    Twentieth S08b.2 slice replaced the next enforcement shortcut with a real
    CEL adapter in `capsem-security-engine`. `CelEnforcementRule` now compiles
    through the `cel` crate before install, `CelEnforcementEvaluator` evaluates
    compiled programs against normalized `SecurityEvent` data, matched
    decisions preserve rule and pack identity in the resolved-event step, and
    malformed CEL fails closed before entering the running engine. Verification:
    `cargo test -p capsem-security-engine` passed with 26 tests and
    `cargo test -p capsem-core --test security_packs` passed with 6 tests.
    Still missing for S08b.2: Sigma-to-runtime detection lowering, service
    `/enforcement/*` and `/detection/*` route wiring, historical hunt over the
    session journal, and VM/runtime integration.
    Twenty-first S08b.2 slice put runtime detection on the same real CEL
    substrate. `CelDetectionRule` now compiles through the `cel` crate,
    `CelDetectionEvaluator` emits typed `DetectionFinding` records with Sigma
    metadata when a normalized `SecurityEvent` matches, and findings attach to
    both the event and resolved event before emitter delivery. Verification:
    `cargo test -p capsem-security-engine` passed with 28 tests and
    `cargo test -p capsem-core --test security_packs` passed with 6 tests.
    Still missing for S08b.2: lowering the existing Sigma-derived Detection IR
    into CEL runtime rules, service route wiring, historical hunt over the
    session journal, and VM/runtime integration.
    Twenty-second S08b.2 slice bridged the existing Sigma-derived
    `capsem.detection.ir.v1` artifact into the real CEL runtime detection
    evaluator. `compile_detection_ir_to_cel_detection_rules` lowers supported
    `equals_any` matchers into `CelDetectionRule`s, preserves pack id, Sigma id,
    severity/confidence/tags, adds an event-family guard, and rejects
    unsupported runtime field paths with typed errors before install.
    Verification: `cargo test -p capsem-core --test security_packs` passed with
    8 tests and `cargo test -p capsem-security-engine` passed with 28 tests.
    Still missing for S08b.2: service route wiring, historical hunt over the
    session journal, match-stat integration with the runtime registry, and
    VM/runtime integration.
    Twenty-third S08b.2 slice connected Security Engine rule matches to runtime
    registry stats. The engine now accepts a `RuleMatchRecorder`, records
    enforcement decisions and all detection findings with event id/timestamp,
    and `RuntimeRuleRegistry` implements the recorder so future
    `/enforcement/stats` and `/detection/stats` routes expose counters updated
    by the same runtime path that produced the decision/finding. Verification:
    `cargo test -p capsem-security-engine` passed with 29 tests and
    `cargo test -p capsem-core --test security_packs` passed with 8 tests.
    Still missing for S08b.2: service route wiring, historical hunt over the
    session journal, and VM/runtime integration.
    Twenty-fourth S08b.2 slice added the first service-owned runtime
    enforcement/detection API spine. `capsem-service` now has in-memory
    runtime registries for `/enforcement/*` and `/detection/*`, handlers for
    validate/compile, live add/update/delete/list, and stats, and compile-first
    installs through the real CEL enforcement/detection evaluators so malformed
    candidate rules fail before touching the registry. Verification:
    `cargo test -p capsem-service
    handle_enforcement_runtime_routes_compile_install_and_report_stats` passed
    and `cargo test -p capsem-service
    handle_detection_runtime_routes` passed with both detection route tests.
    Still missing for S08b.2: historical backtest/hunt over the
    resolved-event/session journal, gateway/CLI/UI route exposure, persistence
    or profile-pack seeding for installed runtime plans, and VM/runtime
    integration.
22. [ ] [S08c - Rule corpus, backtest, and admin parity](S08c-rule-corpus-admin-parity.md)
    -- inserted during the 2026-05-21 rule-runtime regroup. Build the shared
    enforcement/detection/event corpus, offline `capsem-admin` backtest parity,
    Rust runtime parity, and real-session fixture generation after S08b's
    resolved-event journal stabilizes.
23. [ ] [S08d - Security engine performance benchmarks](S08d-engine-performance-benchmarks.md)
    -- inserted during the 2026-05-21 performance/marketing regroup. Extend
    `capsem-bench`, host serial benchmark capture, and Rust microbenchmarks to
    prove VM-originated allow/block/ask/detect latency, rule-count scaling,
    Sigma/CEL matching speed, backtest/hunt scan rates, and resolved-event
    evidence correctness before public surfaces or marketing make speed claims.
24. [ ] [S09 - CLI integration](S09-cli-integration.md)
25. [ ] [S10 - Credential brokerage](S10-credential-brokerage.md)
26. [ ] [S11 - Status, debug, provenance](S11-status-debug-provenance.md)
    -- includes live VM health rendering from S12 snapshots: model call count,
    providers, models, token totals, estimated cost, detection findings, latest
    detection/latest block, and stale/partial metrics state.
27. [ ] [S12 - OpenTelemetry metrics architecture](S12-observability-plugin.md)
    -- typed live accumulator and OTel/status metrics for model/provider/token/
    cost, MCP, enforcement, detection, and host/service AI accounting. VM
    snapshots remain authoritative for VM-originated activity only; host AI
    prompts need separate service-owned counters and OTel dimensions even when
    correlated with a VM/session/profile. S12 also owns enforcement/detection
    match stats, detection finding health, latest detection/latest block
    summaries, and future S22 quota/budget inputs.
    Running status reads memory only; persistent VMs seed/recompute from
    `session.db` exactly once at load.
28. [ ] [S13 - Remote enforcement plugin](S13-remote-policy-plugin.md)
    -- decision mode participates only in `/enforcement/*`; observer mode can
    receive resolved events and detection findings but cannot convert detection
    into blocking decisions. S13 is not the rate-limit/budget or centralized
    quota sprint; it preserves event identity needed by S22 without expanding
    this release scope.
29. [ ] [S14 - Rules UI components](S14-rules-ui-components.md)
    -- enforcement-rule editor component is consumed by S15; detection
    rule/finding/backtest UX consumes S08b/S08c.
30. [ ] [S15 - Confirm UX (Ask)](S15-confirm-ux.md)
31. [ ] [S16 - Profile UI](S16-profile-ui.md)
32. [ ] [S16a - Unified timeline and agent workbench](S16a-unified-timeline-and-agent-workbench.md)
    -- inserted during the 2026-05-19 timeline/UI regroup. Build a friendly
    everyday-work UI for Codex/Claude SDK-backed sessions and terminal fallback
    sessions, backed by S08b's structured `/timeline/{id}` API. Users must be
    able to review/search prompts, assistant responses, tools, files, network,
    processes, findings, asks/confirms, snapshots, artifacts, and profile/rule
    provenance from one coherent timeline. The API provides stable pagination
    over typed blocks; the UI provides conversation/turn/process/activity/trace/
    finding/artifact filtering and formatting with one renderer per block type.
33. [ ] [S17 - Security capabilities UI](S17-security-capabilities-ui.md)
34. [ ] [S19 - Documentation and site](S19-documentation-and-site.md)
    -- adds first-class enforcement and detection-format pages, corporate admin
    security links, `capsem-admin` enforcement/detection validation/backtest
    docs, add-detection/add-enforcement admin guides, telemetry extension guide,
    and VM health/OTel docs for model/provider/token/cost, enforcement counters,
    detection metrics, future quota inputs, and unified event evidence.
35. [ ] [S19a - Marketing site refresh](S19a-marketing-site-refresh.md)
    -- refresh the landing page around four pillars: Ship Fast With AI, Ship
    Safely, Scale Your Productivity Without Drag, and Enterprise Ready. Include
    realtime CEL enforcement, Sigma-compatible detection with backtest and
    forensic timeline/session analysis, fast matching over unified events,
    and S08d artifact-backed engine performance claims without overclaiming
    beyond the sprint tracker. Current-site
    baseline screenshots were captured in
    `artifacts/S19a-marketing-site-refresh/current-ui-baseline/`; refreshed
    pillar screenshots remain part of S19a's final gate.
36. [ ] [S18 - Full verification and release gate](S18-full-verification-release-gate.md)
    -- core Profile V2 release replay and verification gate.
37. [ ] [S20 - OpenAPI to MCP](S20-openapi-to-mcp.md)
    -- proposed standalone product sprint. Convert reviewed OpenAPI-described
    HTTP services into profile-owned MCP tools with provenance, diagnostics,
    UI visibility, and normal security/audit/timeline treatment.
38. [ ] [S21 - Local LLM](S21-local-llm.md)
    -- proposed standalone product sprint. Make local model services
    first-class profile/VM AI providers instead of generic HTTP traffic.
39. [ ] [S19b - Reporting setup](S19b-reporting-setup.md)
    -- proposed standalone, non-blocking operations sprint. Provide reporting
    setup docs, collector examples, privacy guidance, and dashboard packaging
    after S12/runtime fields are stable.
40. [ ] [S22 - Rate limits, budgets, and quotas](S22-rate-limits-budgets-and-quotas.md)
    -- proposed later full sprint, not S08/S13 scope. Decide local engine vs
    plugin-backed provider vs hybrid centralized quota design, then implement
    HTTP/MCP/model/token/cost/request limits using S08b normalized event
    dimensions and the reserved `Throttle` action.

## S06c - Ablate legacy NetworkPolicy runtime

Status: done. Goal: remove the second policy runtime so V1 is gone end-to-end.

S01 removed the V1 settings registry but kept the V1 runtime
plumbing (`crates/capsem-core/src/net/policy.rs`,
`crates/capsem-core/src/net/mitm_proxy/policy_hook.rs`,
`SharedPolicy` type alias). After S01 + S06 + S06b, V1's
domain+method allow/deny is expressible as `dns.request` /
`http.request` rules in V2 with `decision = "block"` and the V1
hook is structurally redundant.

Scope:

- Delete `crates/capsem-core/src/net/policy.rs` (`NetworkPolicy` struct).
- Delete `crates/capsem-core/src/net/mitm_proxy/policy_hook.rs`
  (the V1 hook) and its tests.
- Remove the V1 hook from `make_production_pipeline*` registration.
- Remove the `policy: SharedPolicy` field from `MitmProxyConfig`,
  `DnsHandler`, etc. The V2 `policy` field becomes the only
  policy field.
- Collapse `SharedPolicy` -> `SharedPolicy` (single alias).
- Reroute the DNS `is_fully_blocked(qname)` check to V2 rule lookup;
  the `dns.request` callsite already handles this path.
- Regression test: confirm the migrated V1 denial behavior is
  preserved by the equivalent V2 rule (uses the migration tables
  produced by S06b).

Proof:

- `cargo check -p capsem-core -p capsem-process`
- `cargo test -p capsem-core --all-targets --no-run`
- `cargo test -p capsem-core net::dns:: --lib`
- `cargo test -p capsem-core policy_hot_reload --lib`
- `cargo test -p capsem-core policy_http_ --lib`
- `cargo test -p capsem-core --test mitm_integration mitm_proxy_plain_http_denies_disallowed_host`
- `cargo test -p capsem-core --test mitm_integration mitm_proxy_plain_http_denies_port_not_in_allowlist`

Details live in
`sprints/policy-settings-profiles/S06c-ablate-legacy-networkpolicy.md`.

## S06d - Core structure and test boundaries

Status: done. Goal: split oversized MITM/DNS modules and behavior test
buckets before the post-S06 rename and S08b engine extraction.

Scope:

- Split `crates/capsem-core/src/net/mitm_proxy/mod.rs` into smaller internal
  modules for config/shared deps, connection/TLS handshake, request handling,
  upstream dispatch, and direct telemetry helpers.
- Split `crates/capsem-core/src/net/mitm_proxy/tests.rs` into focused test
  files for connection behavior, HTTP Policy, hot reload, model policy,
  upstream failures, telemetry, and body preview behavior.
- Split `crates/capsem-core/src/net/dns/tests.rs` into focused test files for
  Policy decisions, cache semantics, resolver failover/errors, metrics/
  telemetry, and rewrite response behavior.
- Split `crates/capsem-core/tests/mitm_integration.rs` if the resulting
  integration test filters remain straightforward.
- Keep all moves behavior-preserving. New engine crates are explicitly
  deferred to S08b, after S08a/S08b define the contracts.

Details live in
`sprints/policy-settings-profiles/S06d-core-structure-and-test-boundaries.md`.

Progress:

- DNS unit tests are split into `policy_decisions`, `resolver_behavior`,
  `rewrite_behavior`, `metrics_behavior`, and `cache_behavior`.
- MITM policy regression tests are split into `tests/model_policy.rs` and
  `tests/http_policy.rs`; connection/metadata/FD/TLS behavior is split into
  `tests/connection_behavior.rs`; the remaining `tests.rs` harness carries
  shared fixtures plus smaller utility/body/upstream behavior.
- Production MITM helpers are split into `upstream.rs`, `pipeline_factory.rs`,
  and `response.rs`.
- Added `runtime_call_sites_do_not_import_legacy_network_policy_runtime` so the
  removed V1 `NetworkPolicy`/MITM hook runtime cannot creep back.

Proof:

- `cargo fmt --package capsem-core`
- `cargo check -p capsem-core -p capsem-process`
- `cargo test -p capsem-core runtime_call_sites_do_not_import_legacy_network_policy_runtime --lib`
- `cargo test -p capsem-core net::mitm_proxy::tests::connection_behavior --lib`
- `cargo test -p capsem-core net::mitm_proxy::tests::response_uses_gzip_content_encoding_accepts_token_lists_case_insensitively --lib`
- `cargo test -p capsem-core net::mitm_proxy::tests::upstream_connect_target_honors_debug_test_override --lib`
- `cargo test -p capsem-core policy_model_ --lib`
- `cargo test -p capsem-core policy_http_ --lib`
- `cargo test -p capsem-core policy_hot_reload --lib`
- `cargo test -p capsem-core net::dns:: --lib`
- `cargo test -p capsem-core --all-targets --no-run`
- `git diff --check`

## Post-S06 cleanup milestone

Originally planned to run before S07. It is closed as of 2026-05-21. The
rescue merge/reconciliation portion is closed for the active branch:
`HEAD...origin/main` is currently `138 ahead / 0 behind`. S06d structural
hygiene is closed, the final V2 naming collapse is complete, and later S07/S07a
route plus asset/admin gates proved the rename against the public contracts.

1. **Confirm branch remains caught up.** Done: `git rev-list --left-right
   --count HEAD...origin/main` returned `138 0`.
2. **S06d structural hygiene is closed.** Keep crate extraction deferred to
   S08b.
3. **V2 rename across the crate.** Done.
   - Files moved to `net/policy`, `policy_http_hook.rs`,
     `policy_model.rs`, and `benches/policy.rs`.
   - Types now use `PolicyHttpHook`, `LastHttpPolicyDecision`, and
     `LastModelPolicyDecision`.
   - Runtime fields/helpers/test filters now use singular `policy` names.
   - MCP keeps `rules_policy` only where the unified rules config must coexist
     with the existing local `McpPolicy`.
4. **Focused verification passed.**
   - `cargo check -p capsem-core -p capsem-process`
   - `cargo test -p capsem-core policy_model_ --lib`
   - `cargo test -p capsem-core policy_http_ --lib`
   - `cargo test -p capsem-core policy_hot_reload --lib`
   - `cargo test -p capsem-core policy_mcp --lib`
   - `cargo test -p capsem-core net::dns:: --lib`
   - `cargo test -p capsem-core --all-targets --no-run`
   - `cargo test -p capsem-process mcp_runtime`
   - `cargo test -p capsem-process --no-run`
   - `git diff --check`
5. **Full verification gate.** Closed by the 2026-05-20 `just smoke` pass and
   the 2026-05-21 S07b focused admin/profile/image/manifest/security/docs/
   doctor suite (`174 passed, 1 skipped`) plus Python compileall and docs
   build. The heavier final replay remains S18 release-gate work, not
   Post-S06 cleanup debt.

Public API work reconciled the rename through S07/S07a/S07c/S07b and S08
gateway mirroring. Any future fallout belongs to the owning sprint that touches
the surface.

### Merge conflict guidance (applies in step 1)

Conflicts most likely in:
`crates/capsem-service/src/main.rs` around
`enrich_telemetry_from_session_db` / `handle_list` / `handle_info`,
the new `/list` regression test, and policy code touched by the
parallel hardening work.

Resolve in favor of main where the conflict overlaps with
[S12's](S12-observability-plugin.md) intent (the `/list`
SQL-on-hot-path hotfix and the `attach_list_live_metrics_placeholder`
/ regression test pair). Preserve the S06-pre confirmer plumbing
landed across slices 6a-6e: `crates/capsem-core/src/net/policy_confirm.rs`
(including `confirm_with_backoff` + `default_confirm_backoff`),
the DNS / HTTP / MCP / model ask wiring callsites, and the
per-subsystem `confirm_opts` builders.

## Notes for upcoming work

(Only items that inform a sprint not yet started. Anything tied to
a closed slice/sprint moved to [completed sub-sprints](#completed-sub-sprints).)

- **S07 inherits a proto-types task.** Foundational metrics types
  (`capsem_proto::metrics`) land in S07 so [S12](S12-observability-plugin.md)
  can start with proto already in place. See S12 spec.
- **S07a is closed.** Before HTTP, CLI, and UI harden profile
  create/VM create semantics, the signed manifest must become the profile
  catalog and profiles must carry a closed `capsem.profile.v2` contract backed
  by JSON Schema Draft 2020-12, with package/tool contracts plus per-arch VM
  asset declarations. S07a also defines first-use asset download, profile
  revision status, cleanup retention, and persistent VM profile/revision/asset
  pins. That bridge is now implemented and verified; UI polish lives in S16,
  deeper provenance in S11, and release replay in S18.
- **S07c is closed.** The background downloader exists, but
  `capsem update --assets`, status/debug provenance, structured lifecycle logs,
  and cleanup/create concurrency must be unified around the Profile V2 service
  reconciler before profile asset operations are production-grade. That
  operator path now exists and has live boot proof.
- **S07d is closed.** Profiles now have a stronger
  contract than service settings. Before `capsem-admin` exposes service
  settings, add `capsem.service-settings.v2` JSON Schema Draft 2020-12,
  Pydantic v2 models, valid/invalid fixtures, Rust/Python drift tests, and
  admin `settings validate/schema/doctor` hooks. The schema/Pydantic/fixture
  contract, first `capsem-admin settings` commands, cross-runtime defaults
  drift proof, and closeout docs have landed.
- **S07b is closed.** The current Python image builder and
  manifest scripts must be unified under a released `capsem-admin` package.
  Profiles become the source of truth for image build plans and manifest
  entries; service settings become a first-class admin object through S07d;
  `capsem-admin profile/settings validate/schema` consumes shared JSON Schema
  artifacts and valid/invalid fixtures; Python admin internals use Pydantic v2
  models with Pydantic-only JSON input/output instead of raw nested dicts;
  hand-edited `guest/config` image settings are not carried forward as
  compatibility input. That tooling now exists, including doctor closeout.
- **S08a is the enforcement/detection architecture gate.** Before S11/S12/S13/S14/S15
  harden logging, telemetry, plugins, rule UI, and Confirm UX, decide whether
  Capsem runtime enforcement rules and Sigma-style detection rules are separate rule
  families, and define the normalized event/finding schemas they consume.
- **S12 architecture: single source of truth.** The in-memory
  per-VM accumulator in `capsem-process` is the only runtime
  source; `session.db` is read on the data path exactly twice in
  a VM's life (seed at launch + cold one-shot in stopped-VM
  `/info`). No `/list` / scrape endpoints / running-VM `/info` /
  gateway status path opens `session.db`. Two open questions
  remain (hypervisor-vs-guest-agent for guest counters; new-counter
  schema migration); decide before [S12](S12-observability-plugin.md)
  starts.
- **S15 release hold.** Do not ship a release that advertises
  `decision = "ask"` while only `PlaceholderConfirmer` is
  registered. Either [S15](S15-confirm-ux.md) lands the UI + CLI
  prompter, or release docs say ask = allow-by-default. The
  same hold is captured in [MASTER Release Holds](MASTER.md#release-holds).

## Completed sub-sprints

One-line each. Detail lives in the corresponding spec file and in
the commit history.

- **S00** (2026-05-14) - Meta sprint setup: board, requirements,
  plan, tracker, all sub-sprint files.
- **S01** (2026-05-14) - V1 settings/policy removal: provision/run
  VM defaults, `/mcp/*`, `capsem-process` runtime, `/settings*`
  cut over to typed `settings_profiles`. Strict payload contract
  (no legacy `tree` / `issues` / `presets` / `policy` keys).
- **S02** (2026-05-14) - Service settings design closed.
- **S03** (2026-05-14) - Service settings implementation: typed
  service settings, profile TOML, built-in Everyday Work profile,
  TOML credentials, profile discovery, descriptors. Asset/manifest
  startup wiring + `/setup/assets` provenance.
- **S04** (2026-05-14) - Profile design closed; canonical v1 rule
  format locked at `security.rules.<type>.<rule_name>` with
  default priority `1`.
- **S05** (2026-05-14) - Profile implementation: parser, validation,
  CRUD primitives, fork, security capabilities, narrowed profile
  types.
- **S06-pre slices 6a-6e** - Confirmer trait + placeholder; DNS,
  HTTP request+response, MCP request+response, model
  request/response/tool-call/tool-response ask wiring.
- **S06-pre adversarial backfill** - Per-subsystem redaction +
  oversized-snapshot + concurrency + panic-isolation tests. TDD
  surfaced two real bugs (HTTP path unbounded; MCP tool_name
  unbounded), fixed via per-field truncation.
- **S06-pre backoff refactor** - Replaced the bespoke
  `Confirmer::timeout()` + `DEFAULT_CONFIRMER_TIMEOUT` constant
  with the shared `capsem_proto::poll::RetryOpts` /
  `crate::poll::poll_until` primitives. New
  `confirm_with_backoff(confirmer, args, &RetryOpts)` wraps each
  attempt in a per-attempt timeout and retries with exponential
  backoff up to the overall deadline. All five callsites (DNS,
  HTTP req/resp, MCP req/resp, model) route through it. Each
  subsystem state has a `confirm_opts: RetryOpts` field with a
  `with_confirm_opts` builder.
- **S06-pre slice 6f - Exit tests** (closed) -
  `confirm_with_backoff` contract tests (accept/deny passthrough,
  hang -> Deny on timeout, panic propagation across the await
  boundary, documented defaults); 200-way concurrent-load smoke
  for HTTP ask resolution; resolved-outcome attribution fix in
  HTTP / DNS / model so `policy_action` reflects `"allow"` /
  `"block"` after the confirmer returns (MCP already correct).
  The capsem-doctor E2E ask probe is deferred (needs doctor
  policy-injection + session-DB read-back fixtures). See
  [Deferred items](#deferred-items-visible-debt) for the
  carry-over.
- **S06 - Assembly and VM-effective settings** (closed,
  2026-05-15 / 2026-05-16) - Six slices: parent-chain
  validation + ancestor-chain helper (6.1), layered profile
  merge with `inherited_from` provenance (6.2), resolver trace
  artifact `vm-effective-trace.json` + service-side attach
  (6.3), corp directives add/remove/replace (6.4), lock /
  forbid + typed `ResolverViolation` (6.5), trace summary in
  status / debug + `Reject` event before violation early
  return (6.6). In-VM E2E probe deferred with same unblock as
  S06-pre slice 6f.
- **S06a - Model request rewrite** (closed, 2026-05-15) -
  `evaluate_model_request_policy` applies rewrite via
  `rewrite_model_request_body` on `request.data` (unified with
  the condition vocabulary). Fail-closed paths: unsupported
  target, non-UTF-8 body, pattern non-match. Removed the
  `unsupported_rewrite` shim. 4 new tests plus 1 repurposed
  integration test.
- **S06b - Legacy allowlist migration + rule ownership locks**
  (closed, 2026-05-16) - Nine slices. Inventory found S01's
  cutover left v1 settings registry + allowlist builders as
  test-only dead code, so the sprint became: 6b.0 deleted
  ~12k LOC of v1 surface; 6b.1 added ownership metadata fields
  (`owner_setting_path`, `owner_setting_label`, `editable`)
  on `EffectiveRule`; 6b.2 enforced priority tiers (corp
  `[-1000, -1]`, toggle-derived `0`, user `[1, 999]`,
  catch-all reserved `1000`) with origin-aware corp-exclusive
  validation; 6b.3 added nestable rule blocks under setting
  hosts (`ai.providers.<name>.rules.*`,
  `mcpServers.<name>.capsem.rules.*`); 6b.4 split HTTP catch-all
  into `http.read` / `http.write` callbacks dispatched by
  method group; 6b.5 retargeted capability-derived rules from
  priority 100 -> 1000 as proper per-runtime-callback
  catch-alls (`dns.default`, `http.default_read`,
  `http.default_write`, `model.default`, `mcp.default`); 6b.6
  added provider-toggle derived rules at priority 0 from
  `ai.providers.<name>.enabled` (static host map + base_url
  fallback for unknown providers); 6b.7 added MCP
  `allowed_tools` derived rules at priority 0; 6b.8 added
  `ensure_rule_editable` mutation gate returning
  `RuleManagedBySetting { rule_id, owner_setting_path }`. 6b.9
  documentation scope captured in
  [S19 spec](S19-documentation-and-site.md) as a
  decisions-to-document appendix + per-slice docs task list.

## Coverage ledger (sprint-wide rollup)

Current as of 2026-05-19 after the S08 live profile-selected gateway boot
proof slice.

- **Unit/contract**: `settings_profiles` carries **118** focused
  tests (resolver, ownership, priority validation, nestable
  rules, catch-alls, provider toggles, MCP allowed_tools,
  mutation gate). `corp/tests.rs` carries **18** corp-directive
  tests. `resolver_trace/tests.rs` carries **9** trace tests.
  HTTP/DNS/MCP/model confirm wiring covered;
  `confirm_with_backoff` covered by 5 dedicated tests.
  `http.read` / `http.write` callback split covered by **5**
  hook-boundary tests in `policy_http_hook/tests.rs`.
  S07 metrics proto foundation adds **36** focused `capsem-proto`
  IPC tests and **18** focused `capsem-process` IPC tests. S07a
  telemetry identity now has focused logger schema/writer/reader,
  core env-resolution, and service serialization/enrichment tests. Profile
  manifest lifecycle gates now have explicit `active` / `deprecated` /
  `revoked` install/new-VM/existing-VM contract tests, plus current/specific
  revision resolution tests in both Rust and Pydantic admin models. Core
  install guards cover active-status, BLAKE3 payload hash, schema validation,
  and manifest/payload id+revision parity in both Rust and Pydantic admin
  models. Runtime conversion/materialization tests prove verified Profile V2
  payloads become resolver-compatible corp TOML while preserving the exact
  signed payload bytes in installed revision storage; `current.json` records
  the installed profile id, revision, and payload hash for later status/debug
  and VM pinning. Profile payload signature verification reuses the existing
  minisign verifier with tamper coverage; fetch tests prove catalog payload/
  signature locations are read and verified before hash/schema/id/revision
  checks. Core profile catalog reconciliation covers active install/update,
  incomplete active re-install, complete active no-op, deprecated installed
  revision keep, and revoked launchable profile plus current-state removal. VM
  profile pins add registry roundtrip, package-contract hash, installed sidecar
  revision/payload-hash capture, API serialization, and fork persistence
  coverage. Service profile catalog reconciliation covers active current
  revision install and revoked installed revision removal through
  `POST /profiles/catalog/reconcile`, including per-revision error summaries.
  The native CLI parser now covers `capsem profile reconcile-catalog
  --manifest --pubkey [--json]` and `--manifest-url --pubkey`, and URL-source
  contract tests cover local file reads, loopback URL fetches, non-loopback
  HTTP rejection, missing/conflicting sources, and oversized response
  rejection. Typed service-settings coverage now covers `[profile_catalog]`
  URL/public-key/check-interval validation, and service coverage proves a
  configured catalog URL installs a verified payload and persists the trusted
  manifest snapshot. Absent installed profile cleanup now has a
  core contract test for removing launchable current state while preserving the
  archived payload plus service-route coverage for the `absent_removed`
  summary/outcome. Retention-source coverage now proves installed current
  profile payloads emit hash-derived VM asset filenames, archived payloads
  without `current.json` do not retain assets, persistent VM profile pins feed
  saved-asset retention, and real cleanup preserves the combined profile+VM-pin
  set while deleting an unreferenced hash-named asset. Production cleanup now
  adds a manifest-free hash cleanup helper plus `POST /setup/assets/cleanup`,
  preserving installed-profile and saved-VM retention, deleting stale
  hash-named files and legacy `v1.0.*` directories, and returning
  `409 Conflict` while assets are checking or updating. VM list/status now
  reports pinned profile id/revision plus current/needs_update/deprecated/
  revoked/corrupted/unknown state, and `capsem list`/`capsem info` render the
  typed client enum; missing pins are corrupted. Profile pin construction now
  requires a signed catalog revision, profile payload hash, and pinned asset
  identity, and create-from-source/fork/persist reject missing, revisionless,
  or payload-hash-less pins before durable clone/move work. Fork cloning now
  preserves VM-effective profile attachments, rejects profile and payload-hash
  drift, and has fork-plus-exec IPC coverage for same-profile execution. S07
  closeout adds focused capsem-service tests for Profile V2 skills
  create/list/delete, duplicate direct and inherited same-kind skill rejection,
  enabled/disabled conflict cleanup, inherited skill delete rejection, typed
  empty confirm listing, and a chained profile -> skills -> MCP -> rules ->
  evaluate -> confirm-listing proof.
- **Functional**: profile CRUD, VM-effective resolve via
  ancestor chain, layered merge, resolver trace artifact
  round-trip, corp directives end-to-end through
  `resolve_effective_vm_settings_with_corp`, debug-report
  rendering with resolver-trace summary, service startup +
  asset settings, verified profile payload materialization into the corp
  profile root and installed revision payload storage, service API profile
  catalog reconcile install/revoke/absent-removal summaries, native
  CLI-to-service wiring for `profile reconcile-catalog`, `/setup/assets`
  provenance, profile-aware cleanup retention source composition, `POST
  /setup/assets/cleanup` cleanup execution with installed-profile/saved-VM
  retention, `/list`/`/info`/`capsem list`/`capsem info` profile-state
  rendering, create-from-source/fork/persist fail-closed profile pin gates,
  fork-plus-exec same-profile IPC coverage, profile payload-hash pin
  enforcement, Profile V2 skills and confirm listing through the live service
  UDS HTTP harness, chained S07 profile/skills/MCP/rules route proof,
  mitm_proxy integration test for model.request rewrite redaction. S08 live
  gateway coverage now starts real service/gateway processes from a Profile V2
  asset fixture, creates a VM with explicit profile id/revision over HTTP,
  waits for exec-ready, execs inside the VM, and verifies `/info` echoes the
  same pinned profile/status.
- **Adversarial**: profile load (unknown fields, malformed TOML,
  bad endpoint schemes, callback/type mismatches, duplicate
  profile ids, governance toggles). Inheritance graph: unknown
  parent, multi-hop cycles, depth overflow. Confirm wiring:
  redaction, bounds, concurrency, panic isolation, hang
  fail-closed. Corp directives: unknown path, type mismatch,
  add-on-existing, remove-on-missing, lock then re-mutate,
  forbid then add restores (all surface `ResolverViolation`
  with a `Reject` trace event before the early return). Asset
  pipeline: full malformed-input matrix. Priority validation:
  out-of-range high/low, reserved catch-all `1000`, corp
  priority in non-corp profile, corp directive at user-tier
  priority. S07 skills mutation: duplicate direct and inherited same-kind
  skills fail with `skill_exists`, inherited deletes fail with
  `skill_is_locked`, and enabled/disabled transitions remove contradictory
  state. Model.request rewrite: unsupported target, no match, non-UTF-8 body.
  S08 gateway typed-error coverage proves HTTP preserves exact status/body for
  malformed profile create, locked inherited skill deletion, locked inherited
  MCP server deletion, locked built-in rule deletion, invalid rules/evaluate
  callback, asset cleanup while updating, and revoked profile revision install.
- **E2E/VM**: covered for the S03 service-settings asset
  runtime slice (real service + real gateway + malformed TOML
  startup + VM boot/exec) and the S06a mitm_proxy integration
  test forwarding rewritten model bodies. Capsem-doctor ask
  probe remains deferred (see below). S07c now has focused service-path proof
  that first-use VM create downloads missing selected-profile assets through
  the Profile V2 reconciler before process spawn plus a live E2E proof that
  `capsem update --assets` reconciles real profile-declared VM assets into an
  empty cache, boots a real VM, execs inside it, and preserves the installed
  profile revision pin in `capsem info --json`. S08 now adds the equivalent
  HTTP gateway proof for selected-profile create/download/boot/exec and
  `/info` profile-state echo.
- **Telemetry**: debug report exposes
  profile/settings/rule provenance and now the resolver trace
  summary (event count, corp event count, locked paths,
  rejected paths, last N events). Hook-boundary attribution
  for ask resolves locks the resolved outcome (`allow` /
  `block`). S07a adds a durable `session_identity` row to
  `session.db` with `vm_id`, `profile_id`, and `user_id`, service
  propagation into `capsem-process`, `/info` exposure, and focused
  read-back coverage. VM metadata surfaces the corresponding profile pin for
  status/detail paths without reopening `session.db` on `/list`, and now
  requires the installed profile payload hash for forward VM pin construction
  and source/fork/persist validation. S07c adds Profile V2 asset reconcile
  timestamp propagation through `capsem status --json`, text status rendering,
  installed profile payload hash, redacted per-asset source/hash metadata, and
  structured service log events for profile asset check/download lifecycle,
  with URL redaction coverage.
  Persisted
  policy-decision read-back from a running `session.db` (capsem-doctor E2E ask
  probe) is **deferred**.
  `policy_confirm_events` table remains S06-pre slice 7+ work.
- **Performance**: no benchmarks added by S06/S06a/S06b; the
  resolver runs at provision / reload, not on the hot path,
  so benchmarks would not represent a meaningful budget.
  Performance work remains pending for later sprints (S12
  in-memory metrics accumulator is the next perf-shaped piece).
  The S07 metrics snapshot request is classified as read-only
  `HealthCheck` IPC so it does not enter job/lifecycle dispatch.
- **Test-gate snapshot** (cargo test, updated 2026-05-18 for S07a service
  profile catalog reconciliation and the first native CLI hook):
  `cargo test -p capsem-logger` **100** + **126** passed;
  `cargo test -p capsem-service` **107** + **140** passed;
  after VM profile pins, `cargo test -p capsem-service` **108** + **141**
  passed;
  after installed profile payload identity pins, `cargo test -p capsem-service`
  **108** + **142** passed;
  after the service profile catalog reconcile route, `cargo test -p
  capsem-service` **108** + **144** passed;
  after the native profile catalog reconcile CLI hook, `cargo test -p capsem`
  **240** passed;
  after absent installed profile cleanup, `cargo test -p capsem-core
  reconcile_ --lib` **6** passed and `cargo test -p capsem-service
  handle_reconcile_profile_catalog` **3** passed;
  package gates after absent cleanup: `cargo test -p capsem-service`
  **108** + **145** passed and `cargo test -p capsem` **241** passed;
  `cargo test -p capsem-core --lib` **1612** passed / 0 failed / 1 ignored
  after absent installed profile cleanup;
  after profile-aware asset retention sources, `cargo test -p capsem-core
  installed_profile_asset_filenames --lib` **2** passed, `cargo test -p
  capsem-core settings_profiles --lib` **133** passed, and `cargo test -p
  capsem-service saved_vm_assets` **2** passed;
  package gates after profile-aware asset retention sources: `cargo test -p
  capsem-core --lib` **1614** passed / 0 failed / 1 ignored and `cargo test -p
  capsem-service` **110** + **145** passed;
  after the profile-aware asset cleanup caller, `cargo test -p capsem-core
  cleanup_ --lib` **7** passed, `cargo test -p capsem-core --lib` **1615**
  passed / 0 failed / 1 ignored, `cargo test -p capsem-service
  handle_asset_cleanup` **2** passed, and `cargo test -p capsem-service`
  **110** + **147** passed;
  after forward-only resume pin enforcement, `cargo test -p capsem-service
  resume_saved_vm` **2** passed and `cargo test -p capsem-service` **109** +
  **148** passed;
  after VM list/status profile-state reporting, `cargo test -p capsem-service
  profile_status` **1** passed, `cargo test -p capsem-service
  handle_reconcile_profile_catalog_installs_current_active_revision` **1**
  passed, `cargo test -p capsem format_session_profile_for_list` **1** passed,
  and `cargo test -p capsem list_response_with_entries` **1** passed;
  full package proof for the same slice: `cargo test -p capsem-service`
  **109 + 149** passed and `cargo test -p capsem` **242** passed;
  after forward-only create/fork/persist profile pin enforcement, `cargo test
  -p capsem-service vm_profile_pin_requires_signed_catalog_revision` **1**
  passed, `cargo test -p capsem-service
  provision_from_source_requires_profile_revision_pin` **1** passed, `cargo
  test -p capsem-service handle_fork_rejects_source_without_profile_revision_pin`
  **1** passed, `cargo test -p capsem-service
  handle_persist_rejects_running_vm_without_profile_revision_pin` **1** passed,
  nearby fork/resume positive-path tests passed, and `cargo test -p
  capsem-service` **109 + 153** passed;
  after fork profile-integrity coverage, `cargo test -p capsem-core
  clone_sandbox_state_preserves_vm_effective_profile_attachments` **1** passed,
  `cargo test -p capsem-service handle_fork_preserves_profile_and_fork_exec_works`
  **1** passed, and `cargo test -p capsem-service
  handle_fork_rejects_profile_string_drift_after_clone` **1** passed;
  full package proof after fork profile-integrity coverage: `cargo test -p
  capsem-core --lib` **1616** passed / 0 failed / 1 ignored, `cargo test -p
  capsem-service` **109 + 155** passed, and `cargo test -p capsem` **242**
  passed;
  after mandatory VM profile payload hashes, `cargo test -p capsem-service
  profile_payload_hash` **3** passed, `cargo test -p capsem-service
  vm_profile_pin` **5** passed, `cargo test -p capsem-service handle_fork`
  **8** passed, full `cargo test -p capsem-service` **109 + 158** passed,
  and `cargo test -p capsem` **242** passed;
  after the first S07c asset update orchestration slice, `cargo test -p
  capsem-service handle_asset_reconcile` **2** passed, `cargo test -p
  capsem-service asset_supervisor --lib` **8** passed, `cargo test -p capsem
  profile_asset_reconcile_summary_line` **2** passed, `cargo test -p capsem
  parse_update_assets` **1** passed, and `cargo test -p capsem
  status_report_preserves_service_asset_updating_state` **1** passed; full
  package proof: `cargo test -p capsem-service` **110 + 160** passed and
  `cargo test -p capsem` **242** passed;
  after the old Rust asset-manifest removal pass, `cargo test -p capsem-core`
  **1575** passed / 0 failed / 1 ignored plus integration/doc tests passed,
  `cargo test -p capsem-core asset_manager::tests` **5** passed,
  `cargo test -p capsem status::tests` **29** passed, `cargo test -p
  capsem` **242** passed, `cargo test -p capsem-service debug_report` **7 +
  1** passed, and `cargo test -p capsem-service handle_asset_` **5** passed.
  `cargo fmt --all -- --check`, `git diff --check`, and the Rust legacy-symbol
  scan for `ManifestV2` / old manifest loaders / old downloader returned clean.
  A broad `cargo test -p capsem-service` sweep was stopped after the lib suite
  passed because the existing `reload_config_returns_structured_failed_session_state`
  binary test sat past the 60s runner warning with no output;
  after profile asset provenance and race-proof hardening, `cargo test -p
  capsem-service active_profile_download` **1** passed, `cargo test -p
  capsem-service concurrent_calls_share_one_download_run` **1** passed,
  `cargo test -p capsem-service handle_asset_reconcile_downloads_missing_profile_assets`
  **1** passed, `cargo test -p capsem-service asset_supervisor --lib` **8**
  passed, `cargo test -p capsem-service debug_report` **7 + 1** passed,
  `cargo test -p capsem status::tests` **29** passed, and `cargo test -p
  capsem setup_asset_health` **4** passed; final local gates for this slice:
  `cargo fmt --all -- --check`, `git diff --check`, `cargo test -p
  capsem-service --lib` **110** passed, `cargo test -p capsem-service
  active_profile_download` **1** passed, `cargo test -p capsem-service
  concurrent_calls_share_one_download_run` **1** passed, `cargo test -p
  capsem-service handle_asset_reconcile_downloads_missing_profile_assets` **1**
  passed, and `cargo test -p capsem` **242** passed;
  after first-use VM create/profile-pin asset authority hardening, `cargo test
  -p capsem-service provision_attempt_reconciles_profile_assets_on_first_use_create`
  **1** passed, `cargo test -p capsem-service source_vm_base_assets` **2**
  passed, `cargo test -p capsem-service handle_fork_uses_profile_pin_assets_when_registry_side_field_is_absent`
  **1** passed, `cargo test -p capsem-service handle_fork` **9** passed,
  `cargo test -p capsem-service handle_persist` **1** passed, `cargo test -p
  capsem-service provision_from_source_requires_profile_revision_pin` **1**
  passed, `cargo test -p capsem-service --lib` **110** passed, `cargo fmt
  --all -- --check` passed, and `git diff --check` passed;
  after profile asset payload/per-asset provenance, `cargo test -p
  capsem-service startup_asset_requirement_includes_installed_profile_payload_provenance`
  **1** passed, `cargo test -p capsem-service
  profile_asset_provenance_redacts_source_urls --lib` **1** passed, `cargo
  test -p capsem-service handle_asset_reconcile_downloads_missing_profile_assets`
  **1** passed, `cargo test -p capsem-service --lib` **111** passed, `cargo
  test -p capsem-service debug_report` **7 + 1** passed, `cargo test -p
  capsem status::tests` **29** passed, `cargo test -p capsem setup_asset_health`
  **4** passed, and `cargo fmt --all -- --check` passed;
  after chained service-level operator proof, `cargo test -p capsem-service
  profile_asset_operator_flow_chains_reconcile_status_debug_and_logs` **1**
  passed, `cargo test -p capsem-service handle_asset_reconcile` **3** passed,
  `cargo test -p capsem-service --lib` **111** passed, `cargo test -p capsem`
  **242** passed, `cargo fmt --all -- --check` passed, and `git diff --check`
  passed;
  after live profile-asset boot proof, `cargo test -p capsem-service
  ensure_assets_once_copies_file_profile_assets_and_reports_ready` **1**
  passed, `cargo test -p capsem
  update_assets_uses_explicit_uds_socket_when_provided` **1** passed, and `uv
  run python -m pytest tests/capsem-e2e/test_profile_asset_boot.py -q` **1**
  passed;
  after S07 closeout, focused `cargo test -p capsem-service skills_api`,
  `handle_create_skill`, `handle_delete_skill_rejects_inherited_skill`,
  `handle_list_pending_confirms`,
  `s07_route_surface_chains_profiles_skills_mcp_rules_and_confirm_listing`,
  and `mcp_connector` passed; `cargo build -p capsem-service` passed; and
  `uv run pytest tests/capsem-service/test_svc_s07_surface.py
  tests/capsem-service/test_svc_mcp_api.py -q` passed with **4** functional
  UDS service tests. The final sweep also passed `cargo test -p
  capsem-service` with **113** lib tests, **193** service-bin tests, and doc
  tests, plus `cargo test -p capsem-core profile_manifest --lib` **20** passed
  and `cargo test -p capsem-core reconcile_profile_revision_from_manifest
  --lib` **5** passed after repairing the stale Profile V2 minisign fixture;
  after real-VM fork-lineage coverage, `uv run python -m pytest
  tests/capsem-e2e/test_winterfell_fork_lineage.py -q -s` **1** passed and the
  existing profile-asset boot proof was re-run with `uv run python -m pytest
  tests/capsem-e2e/test_profile_asset_boot.py -q -s` **1** passed;
  after file/URL profile catalog reconcile sources, `cargo test -p capsem
  profile_catalog` **7** passed, `cargo test -p capsem
  parse_profile_reconcile_catalog` **3** passed, and `cargo test -p capsem`
  **251** passed;
  after scheduled `[profile_catalog]` service source wiring, `cargo test -p
  capsem-core service_settings_` **17** passed, `cargo test -p capsem-service
  reconcile_configured_profile_catalog` **1** passed, `cargo test -p
  capsem-service --lib` **112** passed, and `cargo test -p capsem-core
  profile_manifest --lib` **20** passed;
  after read-only catalog status CLI/API wiring, `cargo test -p capsem-service
  handle_profile_catalog` **2** passed, `cargo test -p capsem
  parse_profile_catalog` **1** passed, and `cargo test -p capsem
  profile_catalog_summary` **1** passed;
  after per-profile revision inspection CLI/API wiring, `cargo test -p
  capsem-service handle_profile_revisions` **3** passed, `cargo test -p
  capsem parse_profile_revisions` **1** passed, and `cargo test -p capsem
  profile_revisions_summary` **1** passed;
  widened gates after that slice: `cargo test -p capsem` **255** passed and
  `cargo test -p capsem-service` passed with **112** lib tests, **174**
  service-bin tests, and doc tests after fixing the profile asset operator-flow
  log capture to run inside one dispatcher-bound runtime;
  after selected revision lifecycle actions, `cargo test -p capsem-service
  handle_install_profile_revision` **2** passed, `cargo test -p capsem-service
  handle_update_profile_revision` **1** passed, `cargo test -p capsem-service
  handle_remove_profile_revision` **1** passed, `cargo test -p capsem
  parse_profile_install_update_remove` **1** passed, `cargo test -p capsem
  profile_revision_action_summary` **1** passed, and `cargo test -p
  capsem-core remove_installed_profile_revision --lib` **1** passed;
  widened gates after selected revision lifecycle actions: `cargo test -p
  capsem` **257** passed, `cargo test -p capsem-service` passed with **112**
  lib tests, **178** service-bin tests, and doc tests, and `cargo test -p
  capsem-core settings_profiles --lib` **137** passed;
  `cargo test -p capsem-core profile_manifest --lib` **20** passed;
  `cargo test -p capsem-core settings_profiles --lib` **130** passed after
  core profile catalog reconciliation;
  `cargo test -p capsem-core --lib` **1611** passed / 0 failed / 1 ignored
  after core profile catalog reconciliation;
  `uv run pytest tests/test_profiles.py -q` **12** passed;
  `cargo test -p capsem-core telemetry --lib` **31** passed;
  `cargo test -p capsem-process --no-run` passed; and
  `cargo test -p capsem-mcp-aggregator --no-run` passed.
  Prior full snapshot (2026-05-16):
  capsem-core lib **1590** passed / 0 failed / 1 ignored;
  capsem-service **95** + **119** passed; capsem-process **98**
  passed; capsem-logger **98** + **126** passed. No warnings on
  touched code; rustc `deny(warnings)` clean. The heavyweight smoke/doctor
  replay was closed later by the [Post-S06 cleanup
  milestone](#post-s06-cleanup-milestone), not re-run per-slice (no slice in
  S06/S06a/S06b touched guest binaries or VM boot path, so the doctor-gated
  checks were not a meaningful regression catcher for what landed).

### Deferred items (visible debt)

- **capsem-doctor E2E ask probe** -- owned by S15/S18. Fire one ask rule per
  subsystem from inside a running VM and read the matched
  rule label back out of `session.db`. Unblock requires the
  [S15 resolve routes](S15-confirm-ux.md) and S08b's resolved-event journal.
  Hook-boundary
  attribution is locked by the Rust-side functional tests so
  this is a coverage-gap item, not a correctness gap.
- **capsem-doctor E2E corp-directive probe** -- owned by S11/S18. Launch a VM
  with a multi-level inherited profile + a corp replace
  directive; assert `/debug/report` shows the resolved policy.
  S07 route support exists; the remaining work belongs with richer debug
  provenance and release replay.
- **Streaming sliding-window body inspector**, pattern
  max-match-length parse-time enforcement, structural rewrite
  parse rejection, instant propagation (`ReloadConfig` push +
  `Arc<PolicyState>` swap), per-chunk `Arc` revalidation,
  `policy_confirm_events` + `policy_body_inspection_events`
  tables. Owned by S08b/S15/S18 because the canonical event journal, confirm
  integration, and final release replay now decide the durable shape.
