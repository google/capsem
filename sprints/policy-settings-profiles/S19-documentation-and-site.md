# S19 - Documentation And Site

## Goal

Document the new settings/profile/policy engine, signed profile catalog, and
corporate deployment model as first-class product documentation, then update the
public docs site so users, operators, and security reviewers can understand the
system without reading Rust code.

This sprint is release-blocking. The redesign is not done if the architecture,
security, and configuration pages still describe v1 settings, old security
levels, standalone `[mcp]`, or `config/defaults.json`.

## Scope

- Publish the chain of trust as the reference diagram for every profile/catalog
  doc:

  ```mermaid
  flowchart TD
      A["Capsem binary<br/>manifest signing public key"] --> B["signed manifest"]
      B --> C["profile id + revision + lifecycle status"]
      C --> D["signed/hashed profile payload"]
      D --> E["package/tool contract"]
      D --> F["VM asset declarations"]
      F --> G["downloaded assets verified by signature/hash"]
      G --> H["VM pinned to profile revision + asset hashes"]
      H --> I["boot with pinned verified assets"]
  ```

  Compact form: `binary trust root -> signed manifest -> profile
  id/revision/status -> verified profile payload -> package/tool contract +
  asset declarations -> verified downloaded assets -> VM profile/revision/asset
  pin -> boot`.
- Write a clear engine guide:
  - service settings versus VM/session profiles.
  - signed manifest as the profile catalog, including profile ids, immutable
    revisions, lifecycle status, payload identity, and compatibility.
  - profile discovery/install/update across base/corp/user roots.
  - profile package/tool contracts and how they map to VM asset requirements.
  - `capsem-admin` profile/image/manifest workflows for corp admins.
  - VM profile/revision/asset pinning and why existing VMs do not silently move
    when a profile updates.
  - VM-effective settings attachment.
  - canonical rules, derived/generated rules, ownership locks, provenance, and
    "why is this here?"
    debugging.
  - canonical rule grammar:
    `security.rules.<type>.<rule_name>`, callback matrix, condition grammar,
    rewrite constraints, and default priority behavior.
  - enforcement architecture: what is evaluated inline, what decisions exist,
    what can block/rewrite/ask, how ask/confirm is logged, and how the Security
    Engine emits the resolved event before telemetry/audit/logging.
  - detection architecture: what detection rules/finding formats are accepted
    after S08a, how they differ from enforcement policy, how findings attach to
    normalized events, and how detection packs live in signed profiles.
  - `confirm()` semantics for `ask` decisions, including telemetry
    (`policy_confirm_events`) and current placeholder behavior.
  - how MCP, skills, credentials, telemetry, remote policy, and VM settings fit
    into the model.
- Write corporate system docs:
  - deploying base and corp profile directories.
  - installing `capsem-admin` from PyPI for enterprise/corp administration and
    using it to create, validate, build, verify, generate, check, and sign
    profile catalogs.
  - deploying signed profile manifests and profile payloads.
  - rolling out active, deprecated, and revoked profile revisions.
  - lazy first-use VM asset download and asset cleanup retention.
  - forbidding user profile creation/fork/delete.
  - creating custom profiles.
  - building a profile from scratch, including package/tool contracts,
    per-arch VM asset declarations, custom images, and controls.
  - setting service-scoped telemetry.
  - configuring enforcement policy packs and detection packs with
    `capsem-admin` typed validation/schema/check commands.
  - configuring remote policy decisions.
  - configuring signed profile catalogs, profile payload hosting, asset
    locations, and custom images.
  - using `capsem-admin manifest check --fast` for HTTP HEAD reachability checks
    and `capsem-admin manifest check --download` for full-byte verification.
  - configuring custom images/rootfs dependencies.
  - credential storage for cutover and brokerage/keychain roadmap.
- Revamp existing docs pages:
  - architecture overview.
  - configuration/settings docs.
  - security overview.
  - enforcement docs.
  - detection format docs.
  - network/policy docs.
  - custom images docs.
  - troubleshooting/debug-report docs.
- Add or update docs site navigation so the new model has a coherent section.
- Add a developer documentation lane for `capsem-admin`: how the package is
  structured, how bootstrap installs it in editable mode, how Pydantic models,
  JSON Schema artifacts, builder modules, manifest modules, doctor checks, and
  tests fit together, and how to develop/debug it without using the released
  PyPI package.
- Remove or rewrite docs that reference v1 settings authority, old security
  levels, standalone `[mcp]`, or JSON-schema/defaults-json authority.

## Candidate Site Structure

Under `docs/src/content/docs/`, likely pages:

- `architecture/settings-profiles.md` - engine overview and resolution flow.
- `architecture/policy-engine.md` - canonical rules, derived/generated rules,
  provenance, enforcement points, remote policy interaction.
- `configuration/service-settings.md` - service TOML, profile roots, telemetry,
  remote policy, credentials, manifest source, asset directory, image roots, and
  asset download endpoint.
- `getting-started/custom-profiles-images.md` - first successful path for using
  Capsem with your own controls/images: install/select a profile, build or
  reference profile-owned assets, validate with `capsem-admin`, publish a
  manifest, and create a VM pinned to that profile.
- `configuration/profiles.md` - profile TOML, profile CRUD/forking, package/tool
  contracts, per-arch VM assets, VM-effective settings, custom profiles, and
  the JSON Schema Draft 2020-12 `capsem.profile.v2` schema reference.
- `configuration/building-profiles.md` - step-by-step profile authoring guide:
  choose id/revision, declare controls, package/tool contract, assets, status,
  validate, derive image plan, build/verify assets, and generate manifest.
- `configuration/profile-catalogs.md` - signed manifest profile catalog,
  revisions, `ProfileRevisionStatus` enum semantics, profile payload
  signatures, lazy download, and asset retention.
- `configuration/capsem-admin.md` - corp-admin CLI workflows for profile
  creation/validation, profile-derived image build/verify, manifest
  generate/check/sign, enforcement/detection pack validation, PyPI install for
  enterprise admins, bootstrap editable install for development, and release
  package verification.
- `configuration/corporate-deployment.md` - corp roots, governance, locks,
  custom images, rollout patterns.
- `configuration/corporate-profiles.md` - enterprise profile format guide:
  how profile payloads work, how statuses affect rollout, how profile-owned
  packages/assets map to VMs, and how corp admins use `capsem-admin` with them.
- `configuration/corporate-security.md` - corp admin entry page linking profile
  governance, enforcement policy packs, detection packs, remote policy,
  telemetry, and audit/export operations.
- `development/capsem-admin.md` - developer reference for the admin package:
  module layout, Pydantic models, JSON I/O boundaries, schema generation,
  builder integration, doctor integration, test fixtures, bootstrap editable
  install, and release packaging handoff.
- `security/profile-capabilities.md` - credential brokerage, PII, MCP RAG/tools,
  network egress, file boundaries, audit posture.
- `security/enforcement.md` - inline enforcement model: normalized security
  events, policy CEL, decisions, ask/confirm, rewrite/block semantics,
  profile-owned policy packs, and resolved-event evidence.
- `security/detection-format.md` - S08a-selected detection format:
  Sigma-compatible shape/import/compile path, normalized event fields,
  detection pack typing/signing, finding schema, telemetry/OTel mapping, and
  `capsem-admin` validation/check workflows.
- `observability/vm-health.md` - live VM status health and metrics: model call
  count, provider/model summaries, token counts, estimated cost, ask/policy,
  HTTP/DNS/MCP/file/process counters, OTel export rules, and the no-hot-SQL
  accumulator/boot-recompute contract.
- Updates to existing `architecture/settings.md`,
  `architecture/custom-images.md`, `security/overview.md`,
  `security/network-isolation.md`, and `debugging/troubleshooting.md` as needed.

Final paths should follow the actual docs tree present when this sprint starts.

## Tasks

- [ ] Audit existing docs for v1 settings/policy language.
- [ ] Define final docs information architecture and sidebar placement.
- [ ] Add the chain-of-trust diagram above to the engine overview,
      profile-catalog reference, corporate deployment guide, and security
      overview, using the same vocabulary in every page.
- [ ] Write engine overview with resolution/provenance diagrams.
- [ ] Write rule-engine grammar reference:
      callbacks, fields, decisions, rewrite rules, priority defaults.
- [ ] Write enforcement security page:
      Security Engine pipeline, inline policy evaluation, real CEL function
      set, allow/block/ask/rewrite semantics, ask/confirm logging, policy pack
      profile ownership, resolved-event evidence, and remote-policy boundary.
- [ ] Write detection format security page:
      S08a-selected `capsem.detection-pack.v1` format, Sigma-compatible
      import/compile path, `capsem.detection.ir.v1`, normalized event-field
      mapping, finding schema, detection pack profile ownership,
      schema/versioning rules, examples, validation errors, telemetry/OTel
      mapping, and why detections do not silently become enforcement.
- [ ] Write service settings reference with TOML examples.
- [ ] Write profile reference with TOML examples, custom-profile workflow, the
      closed `capsem.profile.v2` field table, JSON Schema Draft 2020-12
      artifact, and validation failure examples for unknown fields, wrong
      schema version, bad package versions, and incomplete per-arch asset
      declarations.
- [ ] Write signed profile catalog reference with manifest examples for
      profile ids, revisions, the `ProfileRevisionStatus` enum
      (`active`, `deprecated`, `revoked`), payload hashes/signatures,
      and compatibility.
- [ ] Write enterprise profile-format page under the corporate deployment docs:
      explain the profile TOML/JSON Schema contract, package/tool contracts,
      per-arch VM assets, status enum semantics, VM pinning, and how
      `capsem-admin` validates and publishes the profile.
- [ ] Write corporate security/admin page that links enforcement policy packs,
      detection packs, profile governance, remote policy, VM health, telemetry,
      and `capsem-admin` validation workflows from the corp admin section.
- [ ] Write "build a profile" guide with a complete worked example:
      draft profile, add controls, declare package/tool contract, declare or
      build assets, run `capsem-admin profile validate`, derive/build/verify
      image assets, generate/check/sign manifest, and create a profile-backed
      VM.
- [ ] Write getting-started guide for custom images/controls via profiles:
      how an operator gets from a custom image requirement to a signed profile
      catalog and a VM pinned to the resulting profile.
- [ ] Write profile package/tool contract and VM asset declaration reference.
- [ ] Write `capsem-admin` reference:
      profile create/validate/schema, image plan/build/verify, manifest
      generate/check/sign, fast HTTP HEAD checks, full download checks, JSON
      reports, omitted `--arch` defaulting to all supported release arches,
      enforcement/detection pack validate/schema/check commands once S08a
      finalizes the format, enterprise PyPI install, bootstrap editable
      development install, packaged release usage, and the Pydantic model layer
      that backs
      validation/errors/reports through `model_validate_json()` /
      `TypeAdapter.validate_json()` and `model_dump_json()`.
- [ ] Write developer `capsem-admin` internals page:
      package/module layout, Pydantic model boundaries, JSON Schema artifact,
      profile/image/manifest/doctor modules, how Justfile/bootstrap integrate,
      how to run focused tests, how to add a new command, and how release
      packaging consumes the package.
- [ ] Document profile-backed VM create semantics:
      profile id/revision selection, first-use download, verification,
      persistent VM pins, and no implicit migration on profile update.
- [ ] Write corporate deployment guide.
- [ ] Write telemetry and remote policy configuration guide.
- [ ] Write VM health/metrics guide covering live status values, boot-time
      recompute/seed from `session.db`, no hot-path SQL reads, OTel labels,
      redaction/cardinality rules, model call count, provider/model summaries,
      token counts, estimated cost, and how those values appear in status,
      `/info`, `/metrics/json`, `/metrics`, gateway status, and UI panels.
- [ ] Write custom manifest/profile payload/assets/images/rootfs dependency
  guide or update the existing page.
- [ ] Remove docs that tell admins to edit `guest/config` image settings by hand
      for release images; replace with profile-derived `capsem-admin` flows.
- [ ] Update architecture pages to reflect service/profile/VM-effective
  settings.
- [ ] Update security pages to reflect capabilities, credential brokerage,
  MCP/RAG/tools posture, and remote decisions.
- [ ] Update security navigation so enforcement and detection format are
      separate first-class pages and are linked from corporate admin docs.
- [ ] Update configuration/troubleshooting pages to point to debug-report and
  provenance output.
- [ ] Document `ask -> confirm()` behavior and `policy_confirm_events` telemetry
      query/debug workflows.
- [ ] Document the rule priority tiers (corp `[-1000, -1]`,
      toggle-derived `0`, user `[1, 999]`, catch-all `1000`) and
      the corp-exclusive enforcement gate. See
      [S06b Decisions To Document](#s06b-decisions-to-document).
- [ ] Document `corp_directives` priority window `[-1000, 0]`
      and the catch-all reservation, with worked TOML examples.
- [ ] Document rule ownership metadata
      (`owner_setting_path`, `owner_setting_label`, `editable`),
      including the four ownership classes (hand-authored,
      capability-derived, toggle-derived, corp-directive).
- [ ] Document nestable rules under setting hosts
      (`ai.providers.<name>.rules.<type>.<name>`,
      `mcpServers.<name>.capsem.rules.<type>.<name>`), with a
      worked corp-profile TOML example and the
      "rules follow the file structure" provenance rule.
- [ ] Document `http.read` / `http.write` callback split
      (read = GET/HEAD/OPTIONS; write = POST/PUT/PATCH/DELETE)
      with the catch-all worked example.
- [ ] Document the per-type catch-all rules at priority `1000`
      and their capability-derivation mapping
      (`network_egress` -> dns/http/model catch-alls;
      `mcp_tools` -> mcp catch-all).
- [ ] Document the mutation gate error
      (`Forbidden { owner_setting_path }`) and the
      "Why can't I edit this rule?" troubleshooting flow.
- [ ] Document the two explicit non-migrations: the legacy
      default allow/block lists (`domain_policy::default_*_list`,
      `NetworkPolicy::default_dev`) are NOT ported to rules;
      `NetworkPolicy::http_upstream_ports` exits with S06c.
- [ ] Build docs site and fix broken links/sidebar issues.
- [ ] Add docs review checklist to the release gate.

## Profile Status Enum To Document

Use the canonical `ProfileRevisionStatus` enum name and these exact values in
all docs, examples, CLI snippets, API payloads, UI copy, and troubleshooting
tables:

| Enum value | Meaning |
|---|---|
| `active` | Install/update this revision and allow new VMs. This is the normal offered state. |
| `deprecated` | Keep installed, warn, allow existing VMs, and avoid as the default/current recommendation. |
| `revoked` | Block install/update and block VM launch. Show a high-severity warning for existing VMs pinned to it. Existing VM override behavior, if any, must match the S07a contract and be logged. |

There is no `removed` status. A revision missing from the manifest is absent; a
listed revision that must not be installed or launched is `revoked`.

## Coverage Ledger

- Unit/contract: docs snippets match typed TOML structs, profile catalog
  manifest structs, `ProfileRevisionStatus` enum values, package/tool
  declarations, asset declarations, rule grammar, callback names,
  `capsem-admin` commands, and CLI/API names.
- Functional: docs site builds successfully.
- Adversarial: docs explicitly cover bad config, forbidden corp actions, bad
  remote policy endpoint, missing profile roots, bad profile/asset signatures,
  revoked profiles, rollback/downgrade attempts, missing assets, concurrent
  first-use downloads, cleanup retention races, and debug-report diagnosis.
- E2E/VM: docs examples are checked against actual CLI/API once those sprints
  exist. `capsem-admin` docs examples are checked against the packaged CLI once
  S07b lands.
- Telemetry: docs cover OpenTelemetry endpoint behavior, redaction, retry, and
  failure semantics after S12, including live VM health model metrics
  (provider, model, call count, token counts, estimated cost) and detection
  finding attribution.
- Performance: docs mention profile discovery/remote policy timeout behavior
  only once measured or specified.
- Missing/deferred: concrete page paths and examples must be finalized after
  S07-S13 stabilize public interfaces.

## S06b Decisions To Document

This block captures rule-system design decisions locked during
S06b so S19 can publish them without re-deriving the model
from code. Each bullet is a docs page or section that must
ship before the release gate; cross-link from
[S19 tasks](#tasks) when scheduling.

### Priority tiers

The canonical priority model. Sort is ascending -- **lower
number = higher precedence**. Constants exported from
`capsem_core::settings_profiles`:
`RULE_PRIORITY_RANGE`, `RULE_CORP_PRIORITY_RANGE`,
`RULE_CATCH_ALL_PRIORITY`.

| Range | Owner | Notes |
|---|---|---|
| `-1000` to `-1` | **corp-exclusive** | Only valid in `ProfileSource::Corp` profiles or `corp_directives` entries. `discover_profiles` rejects these priorities in user/base/built-in profiles. |
| `0` | **toggle-derived** | Reserved by convention for system-generated rules (provider toggles, MCP `allowed_tools`). Users CAN write here if they hand-edit their file; the UI defaults to `1`. |
| `1` to `999` | **user-authored** | Recommended range for interactive rule editing. UI default = `1`. |
| `1000` | **catch-all** | System-emitted only. Manual authoring at `1000` is rejected. Per-type catch-alls (`dns.default`, `http.default_read`, `http.default_write`, `model.default`, `mcp.default`) emit here from profile capabilities. |

Document the rationale: corp speaks first (lowest number),
catch-alls speak last (highest number), users live in the wide
middle. Users hand-editing files at "wrong" priorities is
tolerated, not policed; the system places things at reasonable
spots and the validators enforce only the hard boundaries.

### `corp_directives` rule priority

Corp directives that author a `ProfileRule` value must use
priority in `[-1000, 0]` (the corp tier plus the toggle-derived
slot). Enforced by `parse_rule_for_directive` in
`settings_profiles::corp`. Catch-all priority (`1000`) is
rejected.

Document with two TOML examples:

1. Corp directive that adds a deny rule at priority `-100`
   (corp speaks first).
2. Corp directive that re-asserts a toggle-derived allow at
   priority `0` (overrides system-generated default).

### Rule ownership metadata

Three fields on `EffectiveRule`:
- `owner_setting_path: Option<String>` -- dotted path of the
  owning setting when the rule is generated from a non-rule
  setting (e.g. `security.capabilities.network_egress`,
  `ai.providers.openai.enabled`,
  `mcpServers.github.capsem.allowed_tools`).
- `owner_setting_label: Option<String>` -- human-readable
  label rendered as "managed by <setting>" in the UI.
- `editable: bool` -- `false` for setting-derived rules; the
  mutation gate (slice 6b.8) refuses direct edits and points
  callers at `owner_setting_path`.

Document the three contracts:
- **Hand-authored** profile rules in `security.rules.<type>.<name>`
  blocks: `editable = true`, no owner.
- **Capability-derived** rules: `editable = false`, owner
  points at `security.capabilities.<field>`.
- **Toggle-derived** rules (slices 6b.6 / 6b.7): `editable = false`,
  owner points at the producing setting.
- **Corp directive** rule replacements: `editable = true` (corp
  can replace again via another directive).

The mutation gate (slice 6b.8) returns
`Forbidden { owner_setting_path }` for edit attempts on
`editable = false` rules. Docs should describe the error
shape so CLI/UDS clients render actionable messages.

### Rules can live under any setting

Profiles may nest rule blocks directly under any setting that
hosts them, not only under the top-level `security.rules.<type>`
table. Concretely (lands in slice 6b.3):

```toml
[ai.providers.openai]
enabled = true

[ai.providers.openai.rules.http.allow_api_egress]
on = "http.write"
if = "request.host == 'api.openai.com'"
decision = "allow"
priority = 0
```

The resolver walks every host and tags emitted rules with
`owner_setting_path = "ai.providers.openai"` so provenance
follows the file structure. Top-level
`security.rules.<type>.<name>` stays as the home for general /
user-authored rules. Both shapes coexist.

**Important constraint**: nestable rule blocks are a corp
profile feature in spirit (the corp tier authors rules
co-located with the setting they govern). User profiles can
nest rules too (their file, their choice), but the UI/CLI
won't write into nested blocks for user profiles by default.

### `http.read` vs `http.write` callbacks

HTTP catch-alls split into two callbacks (lands in slice
6b.4):

- `http.read` covers `GET`, `HEAD`, `OPTIONS`.
- `http.write` covers `POST`, `PUT`, `PATCH`, `DELETE`.

The MITM dispatcher routes `http.request` events to whichever
callback matches the request method. Rules using `http.request`
still match for both groups (no behavior change for existing
rules); the read/write split lets catch-all rules say
"`http.read: allow *` + `http.write: deny *`" without
boilerplate `request.method in [...]` conditions.

Docs page must include:
- The method->callback table above.
- A worked example: profile with `network_egress = "block"`
  produces `http.default_read` + `http.default_write` rules
  both at priority `1000` with `decision = "block"`.
- A worked example: read-only allow profile (`http.default_read`
  allows; `http.default_write` blocks) for "package manager
  installs are fine, mutating API calls aren't" semantics.

### Per-type catch-all rules at priority `1000`

The resolver emits one catch-all per rule type per profile.
Decision is derived from the relevant capability:
`network_egress` drives `dns.default`, `http.default_read`,
`http.default_write`, `model.default`; `mcp_tools` drives
`mcp.default`.

Document the catch-all -> capability mapping table and the
"runs only if nothing else matched" semantic.

### Sequence of rule resolution

A single matching pass evaluates rules in ascending priority.
Document the resolution order with a worked example showing
how corp at `-500` overrides user at `5`, how user at `5`
overrides toggle-derived at `0`, etc. Include a debug-report
snippet showing the rule list sorted by priority with
provenance annotations.

### Out-of-scope clarifications

Two design questions came up during S06b that landed as
"explicitly NOT migrated" decisions; document them so future
contributors don't re-litigate:

- **Default allow/block lists** (the historical github.com /
  npm / pypi / etc. metadata in `domain_policy::default_*_list`
  and `NetworkPolicy::default_dev`) are NOT migrated as
  per-host rules. The catch-all model at priority `1000`
  replaces "list of default hosts" entirely. Hosts the user
  wants reachable get rules (corp or user) at the right
  priority; everything else hits the catch-all. The legacy
  hardcoded lists exit with S06c (V1 runtime ablation) and
  do not survive as data.
- **`http_upstream_ports`** hardcoded `[80, 11434]` allowlist
  in `NetworkPolicy` is NOT migrated by S06b; it goes away
  entirely with S06c when `NetworkPolicy` is deleted.

### S06b slice -> docs mapping (for the docs author)

| Slice | What it added | S19 page section |
|---|---|---|
| 6b.0 | Deleted v1 settings registry | "Migrating from v1" footnote or appendix; no v1 surface to document |
| 6b.1 | Rule ownership metadata fields | "Rule ownership" / "Managed by ..." UI affordance |
| 6b.2 | Priority validation, constants, corp tier | "Priority tiers" page (the heart of the rule-system reference) |
| 6b.3 | Nestable rules under settings | "Rules can live under settings" page section |
| 6b.4 | `http.read` / `http.write` callbacks | Callback matrix in the rule-engine grammar reference |
| 6b.5 | Catch-all rules at `1000` | "Default action" / "What happens when nothing matches" section |
| 6b.6 | Provider-toggle rules at `0` | "AI providers" page showing rule emission from toggles |
| 6b.7 | MCP `allowed_tools` rules at `0` | "MCP servers" page showing rule emission |
| 6b.8 | Mutation gate (`Forbidden { owner_setting_path }`) | Error reference + "Why can't I edit this rule?" troubleshooting |
| 6b.9 | This block | Source of truth for S19 docs scope |
