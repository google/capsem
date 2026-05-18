# S18 - Full Verification And Release Gate

## Goal

Prove the redesign is releaseable.

## Tasks

- Run backend tests for settings, profiles, assembly, APIs, CLI, enforcement.
- Run frontend tests for settings, profiles, rules, security capabilities.
- Run E2E profile create/fork/delete/select/launch.
- Run manifest/profile-catalog install/update/remove/revoke tests.
- Run profile-backed VM create with missing assets to prove first-use download,
  signature/hash verification, VM pinning, and successful boot.
- Run resume-after-profile-update tests to prove existing VMs keep their pinned
  profile revision and asset hashes.
- Prove MCP, skills, AI providers, credential brokerage, PII, and canonical
  rules enforce through VM-effective settings.
- Prove fresh install still works after v1 removal.
- Prove asset cleanup preserves files referenced by installed active/deprecated
  profile revisions and existing VM pins, and removes unreferenced
  removed/revoked profile assets.
- Prove rollback and revocation behavior:
  stale signed manifest cannot downgrade an installed active profile; revoked
  profile revisions cannot create new VMs; existing revoked VM behavior matches
  the S07a contract and is visible in status/debug.
- Prove first-use download safety under concurrency:
  two simultaneous VM creates for the same profile revision do not corrupt
  partial files, duplicate network work unnecessarily, or race cleanup.
- Prove package/tool contract at runtime:
  a capsem-doctor or equivalent in-guest probe reads declared versions from the
  selected profile revision and verifies the booted VM actually contains them.
- Prove pre-S07a compatibility:
  old persistent VM registry entries resume or fail with an explicit legacy
  compatibility status; they never silently bind to the current catalog default.
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
  bypass models with untyped nested dict manipulation except at parse/output
  boundaries.

## Coverage Ledger

- Unit/contract: complete for profile catalog schema, `capsem.profile.v2`
  JSON Schema Draft 2020-12 closure, shared Rust/Python schema fixture parity,
  Pydantic v2 model coverage for every admin data shape, signatures/hashes,
  lifecycle status, package/tool contracts, per-arch assets, rollback
  protection, resolver inheritance, VM pin metadata, and API/CLI/UI shapes.
- Functional: complete for manifest update, profile install/update/remove/
  revoke, first-use asset download, VM create/resume/fork/delete, cleanup
  retention, explicit profile selection through UDS/HTTP/CLI/UI, and
  `capsem-admin` profile/image/manifest workflows.
- Adversarial: complete for malformed manifests/profiles, bad signatures,
  truncated hashes, unauthorized profile signing key, unsupported arch,
  incompatible binary, revoked/deprecated/removed revisions, partial downloads,
  cleanup races, path traversal, bad URL schemes, and stale catalogs.
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
