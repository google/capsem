# S07a - Profile Manifest, Packages, And Assets

## Goal

Make the signed manifest the profile catalog and make profiles the unit that
drives package/tool assumptions, VM asset download, retention, and lifecycle
state.

This sprint bridges the already-landed Profile V2 resolver work and the public
API/UI layers. It exists so enterprise deployments can publish multiple profile
revisions, each with its own package/tool contract and VM asset locations,
without coupling those assets to a single global "current image" or to the
Capsem binary version.

## Product Contract

- The Capsem binary owns the trust root: the baked-in manifest signing public
  key and the minimum compatibility floor it can enforce.
- The signed manifest owns the profile catalog:
  `profile_id`, `revision`, status, compatibility, profile payload identity,
  profile payload location, and profile payload signature/hash.
- The signed profile payload owns VM/session configuration and declares the
  packages/tools it expects inside the guest plus the VM assets needed to make
  those expectations true.
- VM creation pins the resolved `profile_id`, `revision`, package contract, and
  exact asset hashes. Existing VMs do not move when a profile revision changes
  unless the user explicitly rebases/migrates them.
- Asset cleanup preserves files referenced by existing VM pins and by installed
  active/deprecated profile revisions. Removed/revoked profile revisions do not
  keep assets alive unless an existing VM still pins them.

## Manifest Contract

Add a manifest section that lists profile records. Shape can evolve during
implementation, but the required semantics are:

```json
{
  "format": 3,
  "profiles": {
    "everyday-work": {
      "current_revision": "2026.0520.1",
      "revisions": {
        "2026.0520.1": {
          "status": "active",
          "min_binary": "1.0.0",
          "max_binary": null,
          "profile_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.1/profile.toml",
          "profile_hash": "blake3:...",
          "profile_signature_url": "https://assets.capsem.dev/profiles/everyday-work/2026.0520.1/profile.toml.minisig"
        }
      }
    }
  }
}
```

Required rules:

- `profile_id` is globally stable and unique inside the manifest.
- `revision` is immutable. Updating a profile creates a new revision.
- `current_revision` selects the default revision for new installs/updates.
- Status is the typed `ProfileRevisionStatus` enum everywhere: manifest
  records, Rust models, Pydantic admin models, UDS/HTTP payloads, CLI output,
  UI models, status/debug reports, docs examples, and tests. The only allowed
  values are:
  - `active`: install/update and allow new VMs.
  - `deprecated`: keep installed, warn, allow existing VMs, avoid as default.
  - `removed`: stop offering/installing; local cleanup may remove when unpinned.
  - `revoked`: block new use and surface a high-severity warning for existing
    VMs pinned to it.
- Unknown status strings are rejected. Do not model status as a loose string or
  boolean flags such as `is_active` / `is_revoked`.
- Profile payload identity is verified before the profile is installed or used.

## Normative Profile Payload Schema

S07a must ship a concrete, standard schema artifact for profile payloads:
`schemas/capsem.profile.v2.schema.json`, written as JSON Schema Draft 2020-12.
TOML remains the admin-authored syntax, but validation is defined over the
parsed TOML data model. Do not invent a private schema language.

A planning draft lives at
`sprints/policy-settings-profiles/schemas/capsem.profile.v2.schema.json`; S07a
implementation should either promote it into the production schema location or
replace it with an equivalent Draft 2020-12 artifact before code lands.

Required tooling baseline:

- Rust: add standard JSON Schema validation tooling, such as the `jsonschema`
  crate. If implementation chooses Rust-derived schema generation, use
  `schemars` or an equivalent maintained generator and diff the generated
  output against the committed schema artifact.
- Python/admin CLI: use Pydantic v2 `BaseModel` types for every profile,
  manifest, package, tool, asset, verification report, and command output
  shape. Models must set `extra="forbid"` and use typed validators for semantic
  checks.
- Python/admin CLI JSON I/O may only enter through Pydantic
  `model_validate_json()` or `TypeAdapter.validate_json()` and may only leave
  through `model_dump_json()`. Do not use `json.loads`, ad hoc dict mutation, or
  the Python `jsonschema` package in admin workflows.
- TOML authoring remains supported by parsing TOML once, immediately converting
  that parsed value into the matching Pydantic model, and discarding the
  intermediate dict. If a workflow needs the stricter JSON path, encode the
  parsed TOML value to canonical JSON bytes and validate those bytes through
  Pydantic `validate_json()`.
- Docs/editors/CI: publish the same JSON Schema artifact for documentation,
  editor validation, and golden fixture checks.
- Semantic checks that JSON Schema cannot express cleanly remain explicit code:
  manifest/profile id parity, signature authorization, rollback protection,
  package-manager-specific version resolution, URL allowlists by operating
  mode, and parent revision availability.

The JSON Schema artifact must be closed by default:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://schemas.capsem.dev/capsem.profile.v2.schema.json",
  "title": "Capsem Profile Payload v2",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "schema",
    "version",
    "id",
    "revision",
    "name",
    "description",
    "best_for",
    "profile_type",
    "compatibility",
    "vm",
    "packages",
    "tools",
    "security"
  ],
  "properties": {
    "schema": { "const": "capsem.profile.v2" },
    "version": { "const": 2 },
    "id": { "type": "string", "pattern": "^[a-z0-9][a-z0-9-]{2,63}$" },
    "revision": {
      "type": "string",
      "pattern": "^[0-9]{4}\\.[0-9]{4}\\.[0-9]+$"
    },
    "profile_type": { "enum": ["everyday-work", "coding"] },
    "compatibility": { "$ref": "#/$defs/compatibility" },
    "packages": { "$ref": "#/$defs/packages" },
    "tools": { "$ref": "#/$defs/tools" },
    "vm": { "$ref": "#/$defs/vm" }
  },
  "$defs": {
    "hash": {
      "type": "string",
      "pattern": "^blake3:[0-9a-f]{64}$"
    },
    "asset": {
      "type": "object",
      "additionalProperties": false,
      "required": ["url", "hash", "signature_url", "size", "content_type"],
      "properties": {
        "url": { "type": "string", "format": "uri" },
        "hash": { "$ref": "#/$defs/hash" },
        "signature_url": { "type": "string", "format": "uri" },
        "size": { "type": "integer", "minimum": 1 },
        "content_type": { "type": "string" }
      }
    }
  }
}
```

The committed schema must fully enumerate `$defs` for identity, compatibility,
VM resources, packages, tools, per-arch assets, and the existing S04 security
rule sections. Open-ended package maps may use JSON Schema `patternProperties`
with `additionalProperties: false`; unrestricted `object` holes are not
allowed in the published schema.

Published profile payloads must use this top-level TOML shape:

```toml
schema = "capsem.profile.v2"
version = 2
id = "everyday-work"
revision = "2026.0520.1"
name = "Everyday Work"
description = "Balanced defaults for day-to-day work."
best_for = "Balanced defaults for day-to-day work."
profile_type = "everyday-work"
icon_svg = "<svg ...>...</svg>"
extends_profile_id = "base-everyday"
extends_profile_revision = "2026.0520.1"

[compatibility]
min_binary = "1.0.0"
max_binary = ""
guest_abi = "capsem-guest-v2"

[vm]
cpus = 4
memory_mib = 8192
disk_mib = 32768

[packages.runtimes]
python = "3.12.3"
node = "22.1.0"
uv = "0.4.30"

[packages.python_modules]
requests = "2.32.3"
numpy = "1.26.4"

[packages.node_packages]
playwright = "1.44.0"

[packages.system]
distro = "debian"
release = "bookworm"

[packages.system.apt]
ca-certificates = "20230311"
curl = "7.88.1-10+deb12u12"

[tools.capsem_doctor]
version = ">=1.0.0"
required = true
source = "guest"

[tools.browser]
version = ">=0.1.0"
required = true
source = "guest"

[security.capabilities]
credential_brokerage = "ask"
pii_detection = "ask"
mcp_rag = "allow"
mcp_tools = "allow"
network_egress = "ask"
file_boundaries = "ask"
audit = "audit"

[vm.assets.arm64.kernel]
url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/vmlinuz"
hash = "blake3:..."
signature_url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/vmlinuz.minisig"
size = 12345678
content_type = "application/octet-stream"

[vm.assets.arm64.initrd]
url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/initrd.img"
hash = "blake3:..."
signature_url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/initrd.img.minisig"
size = 12345678
content_type = "application/octet-stream"

[vm.assets.arm64.rootfs]
url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/rootfs.squashfs"
hash = "blake3:..."
signature_url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/arm64/rootfs.squashfs.minisig"
size = 12345678
content_type = "application/vnd.squashfs"

[vm.assets.x86_64.kernel]
url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/x86_64/vmlinuz"
hash = "blake3:..."
signature_url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/x86_64/vmlinuz.minisig"
size = 12345678
content_type = "application/octet-stream"

[vm.assets.x86_64.initrd]
url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/x86_64/initrd.img"
hash = "blake3:..."
signature_url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/x86_64/initrd.img.minisig"
size = 12345678
content_type = "application/octet-stream"

[vm.assets.x86_64.rootfs]
url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/x86_64/rootfs.squashfs"
hash = "blake3:..."
signature_url = "https://assets.capsem.dev/vm/everyday-work/2026.0520.1/x86_64/rootfs.squashfs.minisig"
size = 12345678
content_type = "application/vnd.squashfs"
```

Required validation rules:

- Unknown fields and unknown tables are rejected by JSON Schema. No open-ended
  maps are accepted except explicitly typed package-manager maps such as
  `packages.system.apt`, `packages.python_modules`, and
  `packages.node_packages`.
- `schema` must equal `capsem.profile.v2`; `version` must equal `2`.
- `id` must match the manifest `profile_id`. `revision` must match the
  manifest revision record and is immutable after signing.
- `revision` uses the catalog revision grammar
  `[0-9]{4}\.[0-9]{4}\.[0-9]+` until a later sprint deliberately changes it.
- Catalog-published profiles that inherit from a parent must include both
  `extends_profile_id` and `extends_profile_revision`. Local draft/user
  profiles that are not yet pinned may use an explicit draft mode in
  `capsem-admin`, but published payload validation requires both fields.
- `compatibility.min_binary` is required. `compatibility.max_binary` may be an
  empty string to mean unbounded; `null` is not valid TOML.
- Package versions are strings constrained by JSON Schema patterns where the
  grammar is regular enough, then parsed by package-type-specific validators:
  SemVer for Node/tool versions where applicable, PEP 440 for Python packages,
  Debian version syntax for apt packages, and a documented exact string escape
  only for package managers without a stable grammar.
- `tools.<tool>.version` is required and may be an exact version or a
  comparator range. `required` defaults to `true` only if omitted by generated
  built-in profiles; corp/user-authored profiles must write it explicitly.
- `vm.assets.<arch>` is required for every supported release arch unless the
  manifest marks the profile as arch-limited. Each arch table must contain
  exactly `kernel`, `initrd`, and `rootfs` asset records.
- Asset records require `url`, `hash`, `signature_url`, `size`, and
  `content_type`. `hash` must use the canonical `blake3:<hex>` form.
- Asset URLs must use an allowlisted scheme for the operating mode
  (`https`, signed local file paths, or explicit air-gapped file roots). Path
  traversal is rejected before any file access.
- Existing profile sections from S04 (`general`, `appearance`, `ai`, `mcp`,
  `skills`, `security`) remain part of the same schema and keep their existing
  validation rules.

The shipped schema must preserve these invariants:

- Profiles declare the guest package/tool versions their rules, skills, MCP
  connectors, and UI affordances assume.
- Profiles declare the VM assets and verification metadata that satisfy the
  package/tool contract.
- Profiles may inherit package/tool declarations from a parent and override them
  deterministically through the existing resolver pipeline.
- Effective settings and debug/status surfaces expose the package/tool contract
  and resolved asset identity.

## Trust Chain

Reference chain: `Capsem binary trust root -> signed manifest -> profile
id/revision/status -> verified profile payload -> package/tool contract + VM
asset declarations -> downloaded assets verified by signature/hash -> VM pinned
to profile revision + asset hashes -> boot`.

```mermaid
flowchart TD
    A["Capsem binary<br/>manifest signing public key"] --> B["manifest.json + minisig"]
    B --> C{"verify manifest"}
    C -- invalid --> X["reject catalog update"]
    C -- valid --> D["trusted profile catalog"]

    D --> E["profile id + revision + status"]
    E --> F["profile URL/hash/signature"]
    F --> G{"verify profile payload"}
    G -- invalid --> Y["reject profile revision"]
    G -- valid --> H["trusted profile revision"]

    H --> I["packages/tools contract"]
    H --> J["VM asset URLs + hashes + signatures"]
    J --> K["download on first profile use"]
    K --> L{"verify assets"}
    L -- invalid --> Z["reject VM creation"]
    L -- valid --> M["pin VM to profile revision + asset hashes"]

    M --> N["boot VM"]
    M --> O["asset retention root"]
    H --> O
```

## Architectural Gap Audit

These decisions must be closed before implementation can be called airtight:

- **Manifest v2 to v3 transition.** Existing asset-only manifests must either
  load through an explicit legacy/dev path or fail with a typed "manifest format
  unsupported" error. Release mode must not silently reinterpret a v2 manifest
  as an empty profile catalog.
- **Rollback protection.** A previously installed profile revision must not be
  replaced by an older revision unless an operator explicitly asks for rollback.
  Store the last trusted manifest identity and reject stale signed catalogs when
  they would downgrade an installed active profile.
- **Key identity and rotation.** Define whether profile payload signatures use
  the manifest signing key or a manifest-listed profile signing key id. If
  separate keys exist, the manifest must bind key id, algorithm, and allowed
  profile ids/revisions so a valid signature for one publisher cannot authorize
  another profile.
- **Canonical hash/signature formats.** Choose one canonical on-disk form
  (`blake3:<hex>` etc.) and reject ambiguous or truncated hashes. Profile and
  asset verification must happen before files move into the install location.
- **Atomic and concurrent downloads.** First-use downloads must use temp files,
  per-profile/per-asset locks, verification-before-rename, and retry-safe
  cleanup. Two simultaneous VM creates for the same profile revision must share
  the work or one must wait; they must not corrupt partial files.
- **Per-arch asset declarations.** Profiles need asset declarations per
  supported arch, not a single global URL set. Unsupported host arch fails
  before download with a typed error.
- **Profile inheritance for packages/assets.** Package/tool declarations and
  `vm.assets` must have deterministic parent/child merge semantics, conflict
  diagnostics, and provenance in effective settings.
- **Existing VM migration.** VMs created before S07a need a compatibility
  record: either a synthetic legacy profile revision pin or an explicit
  "unbound legacy VM" status. Resume must not silently bind them to today's
  catalog default.
- **Revocation semantics.** Revoked revisions block new VM creation. Existing VM
  behavior must be explicit: fail closed by default, or allow only with an
  operator override that is logged and visible in status/debug.
- **Asset retention races.** Cleanup must account for running VMs, persistent VM
  pins, installed profile revisions, and downloads in progress. It must never
  remove an asset between readiness check and process spawn.
- **Dev/offline/corp modes.** Dev local assets and air-gapped corp deployments
  need explicit modes, not accidental bypasses. Each mode must preserve the
  trust-chain vocabulary in status/debug.
- **In-guest package proof.** The package/tool contract is not proven by profile
  parsing alone. Add a VM/doctor probe that verifies the booted guest contains
  the declared package/tool versions and records mismatches as diagnostic
  failures.

## Service / Resolver Scope

- Add manifest parsing for profile catalog records and revision status.
- Add manifest format migration/compatibility handling for existing v2
  asset-only manifests.
- Add profile payload download/install/update logic.
- Extend profile schema and effective settings with packages/tools and VM asset
  declarations.
- Resolve the selected profile before provisioning a VM, then ensure that
  profile revision's assets are present. Missing assets download at first use.
- Replace global current-asset selection for profile-backed VMs with
  profile-driven asset resolution.
- Add atomic download, per-asset locking, verification-before-rename, retry, and
  cancellation-safe partial-file cleanup.
- Preserve the dev-mode local-asset path for developer builds, but make the
  release/install path profile-driven.
- Extend persistent VM registry with `profile_id`, `profile_revision`, package
  contract hash, and pinned asset hashes.
- Add existing-VM compatibility handling for pre-S07a VM records.
- Add explicit rebase/migrate semantics later; do not silently move existing
  VMs across profile revisions in this sprint.

## API / UX Hand-Offs

This sprint creates the contract consumed by later sprints:

- S07 exposes installed/catalog profiles, revisions, status, packages/tools,
  asset readiness, and profile-backed VM create/fork options over UDS.
- S07b provides `capsem-admin`, the corp/admin CLI that creates and validates
  profiles, derives image build plans from profiles, verifies built images, and
  generates/checks/signs manifests.
- S08 mirrors that surface over HTTP and streams asset download/readiness
  progress for profile-backed VM creation.
- S09 updates CLI profile and VM creation commands to select a profile
  explicitly and to show profile revision/package/asset readiness.
- S11 status/debug explains profile catalog state, installed revision, package
  contract, asset verification, VM pins, and drift/revocation warnings.
- S16 UI lets users pick a profile/revision when creating a VM, shows package
  and asset readiness, and blocks/labels deprecated or revoked profiles.
- S19 docs explain corporate profile catalog deployment and asset lifecycle.

## Tasks

- [ ] Design manifest v3 profile catalog schema.
- [ ] Add parser/validator tests for profile ids, immutable revisions, statuses,
      profile payload locations, hashes, signatures, and binary compatibility.
- [ ] Commit `schemas/capsem.profile.v2.schema.json` as JSON Schema Draft
      2020-12, with closed-field validation and golden valid/invalid fixtures.
- [ ] Add Rust and Python validation paths that parse TOML to the JSON-compatible
      data model. Python must immediately validate into Pydantic models,
      preferring `TypeAdapter.validate_json()` / `model_validate_json()` when
      JSON bytes are available; Rust validates against the standard JSON Schema
      artifact before semantic trust-chain checks.
- [ ] Extend profile TOML schema with typed packages/tools and per-arch VM
      asset declarations.
- [ ] Add resolver tests for inherited package/tool contracts and asset
      declarations.
- [ ] Add profile payload install/update/delete/revoke logic from manifest
      records.
- [ ] Add profile-driven asset resolution and first-use download.
- [ ] Add atomic first-use download locking and verification-before-rename for
      profile payloads and VM assets.
- [ ] Add cleanup retention for installed profile revisions plus existing VM
      pins.
- [ ] Add persistent VM profile/revision/package/asset pin metadata.
- [ ] Add existing-VM compatibility handling for pre-S07a registry records.
- [ ] Add functional tests for create VM with selected profile revision,
      first-use download, resume after profile update, deprecated profile, and
      revoked profile fail-closed behavior.
- [ ] Add concurrency tests for duplicate first-use downloads and cleanup while
      VM creation is in progress.
- [ ] Add in-guest package/tool contract verification through capsem-doctor or a
      focused VM probe.
- [ ] Update debug/status fixtures with profile catalog and asset readiness.

## Coverage Ledger

- Unit/contract: manifest v3 parser/validator, JSON Schema Draft 2020-12
  validation for `capsem.profile.v2`, Pydantic `validate_json()` /
  `model_dump_json()` fixture parity for Python admin models, valid/invalid
  schema fixture parity across Rust and Python validators, profile package/tool
  parser, asset declaration parser, resolver inheritance/override behavior,
  per-arch asset selection, rollback/stale-manifest rejection,
  signature-key identity, canonical hash format, and v2 manifest
  compatibility/fail-closed behavior.
- Functional: profile install/update/remove/revoke from manifest; selected
  profile VM creation pins revision and assets; resume preserves VM pins after a
  profile update; pre-S07a VM registry entries render explicit compatibility
  state instead of rebinding to the current default.
- Adversarial: bad profile id/revision, unknown fields/tables, wrong schema
  id/version, manifest/payload id mismatch, missing parent revision for a
  catalog-published inherited profile, downgrade attempts, bad signature/hash,
  incompatible binary, revoked profile, missing asset, asset hash mismatch,
  malformed package version, unsupported host arch, profile payload signed by an
  unauthorized key, profile/asset URL scheme rejection, path traversal in
  payload locations, interrupted downloads, and stale partial files.
- E2E/VM or integration: service-level VM create with profile-backed first-use
  asset download; resume an existing VM after catalog update; capsem-doctor or
  equivalent in-guest probe verifies declared package/tool versions match the
  booted VM.
- Telemetry/observability: status/debug report catalog state, installed
  revisions, package contract, asset readiness, VM pin drift/revocation, last
  manifest identity, verification failures, and operator override events.
- Performance: first-use download is not on hot list/status paths; list/status
  must use cached readiness. Resolver overhead for package/tool inheritance is
  bounded by existing profile-chain depth. Concurrent readiness checks must not
  perform duplicate network downloads for the same asset hash.
- Missing/deferred: explicit VM rebase/migration UX is deferred until profile
  create/update surfaces are stable; this sprint only pins and reports.
