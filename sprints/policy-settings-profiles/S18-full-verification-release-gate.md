# S18 - Full Verification And Release Gate

## Goal

Prove the Profile V2 bedrock release is releaseable.

This gate is the Iron Bank. It does not certify a prototype, a partially usable
backend, or a future promise. It certifies that the engine split, signed profile
contract, runtime enforcement/detection, CLI, UI, docs, install path, VM path,
logs/status/debug, and benchmark claims stand together.

## Tasks

- Run backend tests for settings, profiles, assembly, APIs, CLI, enforcement.
- Run frontend tests for settings, profiles, runtime enforcement/detection
  overlays, VM create, logs/status/debug links, and security capabilities.
- Prove S08b engine split: Network/File/Process engines feed the Security
  Engine and Resolved Event Emitter through typed contracts; no shipped event
  family bypasses the canonical resolved-event journal or reintroduces the old
  policy runtime.
- Prove UDS/HTTP/CLI/UI contract alignment: profile, enforcement, detection,
  status/log/debug, and VM-create surfaces use the same route names, typed
  payloads, enum values, error semantics, and evidence fields.
- Run E2E profile create/fork/delete/select/launch.
- Run manifest/profile-catalog install/update/remove/revoke tests.
- Run profile-backed VM create with missing assets to prove first-use download,
  signature/hash verification, VM pinning, and successful boot.
- Run resume-after-profile-update tests to prove existing VMs keep their pinned
  profile revision and asset hashes.
- Prove MCP, skills, AI providers, credential brokerage, PII, and canonical
  rules enforce through VM-effective settings.
- If credential brokerage is not shipped in the bedrock cut, prove no shipped
  profile or docs page advertises credential release as available; S10 owns the
  later implementation.
- If quotas/rate limits are not shipped in the bedrock cut, prove no shipped
  profile or docs page advertises budget enforcement as available; S22 owns the
  later implementation.
- Prove fresh install still works after v1 removal.
- Prove asset cleanup preserves files referenced by installed active/deprecated
  profile revisions and existing VM pins, and removes unreferenced revoked
  profile assets.
- Prove rollback and revocation behavior:
  stale signed manifest cannot downgrade an installed active profile; revoked
  profile revisions cannot create new VMs; existing revoked VM behavior matches
  the S07a contract and is visible in status/debug.
- Prove profile status enum consistency:
  `ProfileRevisionStatus` is the only representation for profile revision
  lifecycle state across manifest parsing, Rust models, Pydantic admin models,
  UDS/HTTP payloads, CLI output, UI models, status/debug reports, and docs. All
  three values (`active`, `deprecated`, `revoked`) have golden tests and
  user-facing semantics. `removed` is not accepted as a status; absent revisions
  are modeled as absent/unknown.
- Prove first-use download safety under concurrency:
  two simultaneous VM creates for the same profile revision do not corrupt
  partial files, duplicate network work unnecessarily, or race cleanup.
- Prove package/tool contract at runtime:
  a capsem-doctor or equivalent in-guest probe reads declared versions from the
  selected profile revision and verifies the booted VM actually contains them.
- Prove forward-only VM identity:
  persistent VM registry entries without a profile pin or pinned asset identity
  fail closed before process spawn; they never silently bind to the current
  catalog default.
- Prove `capsem-admin` packaging:
  bootstrap and release packages install the admin CLI; packaged
  `capsem-admin profile validate`, `manifest check --fast`, and `image verify`
  run successfully from the installed layout.
- Prove bootstrap path:
  developer bootstrap installs the local editable admin tooling with uv
  (`uv sync` / `uv pip install -e .` as finalized by S07b), not by consuming a
  release package.
- Prove profile-derived images:
  release image builds derive package/tool/image settings from selected
  profiles, not hand-edited `guest/config`; tests fail if builder inputs bypass
  the profile source of truth.
- Prove all-arch default:
  omitted `--arch` on `capsem-admin image build`, `image verify`, and manifest
  checks means `all` and covers every supported release arch. Single-arch mode is
  tested only as a narrowing override.
- Prove manifest admin checks:
  `capsem-admin manifest check --fast` validates remote profile/asset URLs with
  HTTP `HEAD`, while `--download` downloads and verifies all referenced bytes.
- Prove profile schema closure:
  `capsem-admin profile validate` rejects unknown fields/tables, wrong
  `capsem.profile.v2` schema id/version, manifest/payload id or revision
  mismatches, malformed package versions, unsupported arch declarations, and
  incomplete per-arch asset records. Rust and Python validators must pass the
  same JSON Schema Draft 2020-12 valid/invalid fixtures.
- Prove admin type safety:
  Python admin workflows use Pydantic v2 models for profile, manifest, asset,
  package/tool, build-plan, doctor, and report shapes. Tests fail if workflows
  bypass models with untyped nested dict manipulation, `json.loads`, or
  `json.dumps`. JSON input tests must go through `model_validate_json()` or
  `TypeAdapter.validate_json()`, and JSON output tests must go through
  `model_dump_json()`.
- Prove release docs truth:
  S19 pages document the bedrock contract and identify S10 credential brokerage,
  S22 quotas/rate limits, S13 remote plugins, S16a workbench polish, S19a
  marketing refresh, S20/S21 product expansions, and S19b reporting setup as
  later work unless they actually landed before this gate.
- Run the S19 documentation review checklist below and paste the exact command
  outputs, grep summaries, and any accepted historical/developer-only matches
  into this gate before release.

## S19 Documentation Review Checklist

Release docs are part of the product contract. The gate must prove they match
the shipped bedrock, not the historical implementation.

- Build the docs site: `pnpm --dir docs run build`.
- Search for stale runtime authority language:
  `rg -n 'guest/config|defaults\.json|config/defaults|\[mcp\]|NetworkPolicy|domain_policy|policy_config|security preset' docs/src/content/docs -S`.
  Every match must be one of:
  historical release notes, explicit developer-only built-in-profile caveat, or
  a statement saying the old path is not runtime/operator authority.
- Confirm the Profile Status enum docs use only `active`, `deprecated`, and
  `revoked`; `removed` must only appear in text explaining that it is not a
  valid status.
- Confirm docs describe signed manifests as profile catalogs with profile id,
  revision, status, payload identity, asset identity, and VM pins.
- Confirm docs describe Service Settings V2 separately from Profile V2 and do
  not claim generated UI descriptor/default artifacts are runtime authority.
- Confirm `capsem-admin` docs cover enterprise PyPI install, developer editable
  install, Pydantic-only JSON I/O, profile validate/schema, image plan/build/
  verify, manifest generate/check/sign, `--fast` HEAD checks, full download
  checks, omitted `--arch` defaulting to all supported arches, and JSON reports.
- Confirm detection and enforcement are documented as separate surfaces:
  detection can validate/backtest/hunt and emit findings; enforcement can
  allow, ask, block, or rewrite synchronously.
- Confirm authored enforcement examples use canonical DSL roots such as
  `http.request.host`, `http.request.url`, `http.request.path`,
  `http.request.header("authorization").exists()`, and
  `http.request.body.text`, not internal `event.*`.
- Confirm docs name S10 credential brokerage, S22 quotas/rate limits, S13
  remote plugins, S16a/S17 richer UI, S19a marketing, S20/S21 product
  expansions, and S19b reporting setup as future lanes unless they have fully
  passed this gate.
- Confirm release pages link operators to profile, catalog, corporate
  deployment, corporate security, VM health, telemetry extension,
  add-enforcement, and add-detection pages without requiring raw SQL or curl to
  understand the shipped path.

## Coverage Ledger

- Unit/contract: complete for profile catalog schema, `capsem.profile.v2`
  JSON Schema Draft 2020-12 closure, shared Rust/Python schema fixture parity,
  Pydantic v2 model coverage for every admin data shape, Pydantic-only JSON I/O
  coverage, signatures/hashes, `ProfileRevisionStatus` enum parity, package/
  tool contracts, per-arch assets, rollback protection, resolver inheritance,
  VM pin metadata, and API/CLI/UI shapes.
- Functional: complete for manifest update, profile install/update/remove/
  revoke, first-use asset download, VM create/resume/fork/delete, cleanup
  retention, explicit profile selection through UDS/HTTP/CLI/UI,
  enforcement/detection runtime registry and backtest/hunt surfaces, and
  `capsem-admin` profile/image/manifest workflows.
- Adversarial: complete for malformed manifests/profiles, bad signatures,
  truncated hashes, unauthorized profile signing key, unsupported arch,
  incompatible binary, revoked/deprecated revisions, absent/unknown revisions,
  partial downloads, cleanup races, path traversal, bad URL schemes, and stale
  catalogs.
- E2E/VM: complete for profile-backed VM boot, package/tool contract proof,
  enforcement through VM-effective settings, resume after catalog update, and
  cleanup safety with at least one persistent VM pin. At least one release-gate
  image is built or fixture-built from profile-derived inputs and verified in a
  booted VM.
- Telemetry: complete for debug/status/reporting of chain-of-trust state,
  profile revision, package contract, asset readiness, verification failures,
  VM pins, drift, revocation, and operator overrides.
- Performance: complete or explicitly waived with rationale; list/status do not
  hit network or perform expensive hash verification, and concurrent first-use
  downloads are bounded and deduplicated.
