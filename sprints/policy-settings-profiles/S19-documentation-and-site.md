# S19 - Documentation And Site

## Goal

Document the new settings/profile/policy engine and corporate deployment model
as first-class product documentation, then update the public docs site so users,
operators, and security reviewers can understand the system without reading
Rust code.

This sprint is release-blocking. The redesign is not done if the architecture,
security, and configuration pages still describe v1 settings, old security
levels, standalone `[mcp]`, or `config/defaults.json`.

## Scope

- Write a clear engine guide:
  - service settings versus VM/session profiles.
  - profile discovery across base/corp/user roots.
  - VM-effective settings attachment.
  - canonical rules, derived/generated rules, ownership locks, provenance, and
    "why is this here?"
    debugging.
  - canonical rule grammar:
    `security.rules.<type>.<rule_name>`, callback matrix, condition grammar,
    rewrite constraints, and default priority behavior.
  - `confirm()` semantics for `ask` decisions, including telemetry
    (`policy_confirm_events`) and current placeholder behavior.
  - how MCP, skills, credentials, telemetry, remote policy, and VM settings fit
    into the model.
- Write corporate system docs:
  - deploying base and corp profile directories.
  - forbidding user profile creation/fork/delete.
  - creating custom profiles.
  - setting service-scoped telemetry.
  - configuring remote policy decisions.
  - configuring custom manifest, asset, and image locations.
  - configuring custom images/rootfs dependencies.
  - credential storage for cutover and brokerage/keychain roadmap.
- Revamp existing docs pages:
  - architecture overview.
  - configuration/settings docs.
  - security overview.
  - network/policy docs.
  - custom images docs.
  - troubleshooting/debug-report docs.
- Add or update docs site navigation so the new model has a coherent section.
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
- `configuration/profiles.md` - profile TOML, profile CRUD/forking, VM-effective
  settings, custom profiles.
- `configuration/corporate-deployment.md` - corp roots, governance, locks,
  custom images, rollout patterns.
- `security/profile-capabilities.md` - credential brokerage, PII, MCP RAG/tools,
  network egress, file boundaries, audit posture.
- Updates to existing `architecture/settings.md`,
  `architecture/custom-images.md`, `security/overview.md`,
  `security/network-isolation.md`, and `debugging/troubleshooting.md` as needed.

Final paths should follow the actual docs tree present when this sprint starts.

## Tasks

- [ ] Audit existing docs for v1 settings/policy language.
- [ ] Define final docs information architecture and sidebar placement.
- [ ] Write engine overview with resolution/provenance diagrams.
- [ ] Write rule-engine grammar reference:
      callbacks, fields, decisions, rewrite rules, priority defaults.
- [ ] Write service settings reference with TOML examples.
- [ ] Write profile reference with TOML examples and custom-profile workflow.
- [ ] Write corporate deployment guide.
- [ ] Write telemetry and remote policy configuration guide.
- [ ] Write custom manifest/assets/images/rootfs dependency guide or update the
  existing page.
- [ ] Update architecture pages to reflect service/profile/VM-effective
  settings.
- [ ] Update security pages to reflect capabilities, credential brokerage,
  MCP/RAG/tools posture, and remote decisions.
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
      `mcp.connectors.<name>.rules.<type>.<name>`), with a
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

## Coverage Ledger

- Unit/contract: docs snippets match typed TOML structs, rule grammar,
  callback names, and CLI/API names.
- Functional: docs site builds successfully.
- Adversarial: docs explicitly cover bad config, forbidden corp actions, bad
  remote policy endpoint, missing profile roots, and debug-report diagnosis.
- E2E/VM: docs examples are checked against actual CLI/API once those sprints
  exist.
- Telemetry: docs cover OpenTelemetry endpoint behavior, redaction, retry, and
  failure semantics after S12.
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
  `mcp.connectors.github.allowed_tools`).
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
| 6b.7 | MCP `allowed_tools` rules at `0` | "MCP connectors" page showing rule emission |
| 6b.8 | Mutation gate (`Forbidden { owner_setting_path }`) | Error reference + "Why can't I edit this rule?" troubleshooting |
| 6b.9 | This block | Source of truth for S19 docs scope |
