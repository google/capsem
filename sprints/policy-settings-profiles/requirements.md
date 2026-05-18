# Policy, Settings, Profiles Requirements

Last updated: 2026-05-18

## Scope Boundary

Settings and profiles are different scopes.

- **Service settings** are service/app-scoped: app behavior, service behavior,
  profile roots, credential storage for now,
  telemetry export, remote policy plugin endpoints, and other host/service-wide
  integration settings.
- **Profiles** are VM/session-scoped: AI providers, MCP/connectors, skills, VM
  settings, guest package/tool contracts, VM asset declarations, security
  capabilities, canonical rules, and derived/generated rules.
- **VM-effective settings** are resolved from a selected profile and attached to
  the VM/session.

Telemetry export and remote policy plugin endpoints are service settings, not
profile settings. Credentials may live in TOML for the cutover. Keychain is
optional stretch work inside the credential brokerage sprint.

The signed manifest is the profile catalog. The binary owns the baked-in
manifest signing trust root. The manifest lists profile ids, immutable
revisions, lifecycle status, binary compatibility, profile payload locations,
payload hashes/signatures, and update/remove/revoke state. Profiles then declare
the package/tool contract and VM asset locations/signatures/hashes for that
revision. Debug output must explicitly show where the manifest came from, which
profile revision is installed/selected, the package/tool contract, boot asset
identity/readiness, telemetry endpoint, and remote decision endpoint.

Corp/admin image and manifest tooling is packaged and exposed as
`capsem-admin`. It is installed by bootstrap and included in release packages.
Profiles are the source of truth for image package/tool contracts, generated
image build plans, and manifest profile/asset entries. Hand-edited image
settings are not supported as a compatibility input.

## Removal Contract

Remove v1 completely:

- remove `config/defaults.json` as runtime/UI authority;
- remove ad hoc `settings.*` registry authority;
- remove standalone `[mcp]` config authority;
- remove legacy network/domain/http/MCP policy builders once replacements exist;
- remove old config-shape awareness completely;
- provide no migration layer and no compatibility diagnostics.

## Typed TOML Contract

Rust structs plus Serde/TOML parsing and Rust validators are the source of truth.
JSON Schema is not the enforcement source. Any UI descriptors or schema artifacts
must be generated from Rust-owned types/descriptors.

## Profile Contract

Profiles are first-class files and the only user-facing security level concept.

Required metadata:

- stable id;
- display name;
- short description;
- "best for" description;
- profile type (`everyday-work` or `coding` in v1);
- SVG icon with default fallback;
- optional appearance defaults (omitted child fields inherit parent first, then
  service defaults);
- version metadata.
- package/tool version contract for guest capabilities the profile assumes;
- VM asset declarations with locations, hashes, signatures, and guest ABI.

The first built-in profile is "Everyday Work" or equivalent. "Mid security" and
"High security" are not product concepts.

Profiles have immutable revisions once published through the signed manifest.
Updating a profile creates a new revision. Existing VMs pin the profile
id/revision and exact asset hashes they were created with; they do not silently
move when a newer revision lands.

## Manifest/Profile Lifecycle Contract

The signed manifest must support profile lifecycle state:

- `active`: install/update and allow new VMs.
- `deprecated`: keep usable for existing installs/VMs, warn, avoid as a new
  default.
- `removed`: stop offering/installing; local cleanup may remove when no VM pins
  it.
- `revoked`: block new use and surface high-severity warnings for existing VMs
  pinned to it.

Downloads are lazy: Capsem downloads profile-owned VM assets at first profile
use or explicit prefetch, not unconditionally for every catalog profile. Cleanup
must retain assets referenced by existing VM pins and by installed
active/deprecated profile revisions.

## Admin Tooling Contract

The Python admin tooling must be a uv-managed package with a public
`capsem-admin` CLI. Required flows:

- create and validate profile payloads;
- derive image build plans from profiles;
- build or fixture-build profile-derived images;
- verify image assets and in-guest package/tool versions against the profile;
- generate, check, and sign profile manifests;
- fast-check remote profile/assets with HTTP `HEAD` without downloading full
  assets;
- full-check remote profile/assets by downloading and verifying every referenced
  payload/asset.

The old model where release image settings are edited directly in builder config
is removed. Generated config may exist as build output only.

## Security UI Contract

Profile > Security starts with capabilities, not direct rule editing.
Capabilities cover
credential brokerage, PII detection/blocking/redaction, MCP retrieval/RAG,
MCP/local tool policy, network/domain/HTTP posture, model request/response
scanning, file boundaries, and audit expectations.

Canonical `security.rules.<type>.<rule_name>` tables remain below capabilities.
Generated rules are gray with provenance; corp/base inherited rules are locked.

## Debug Contract

Wrong settings must explain failures. Debug report and status must show service
settings, profile roots, manifest/catalog state, selected profile revision,
package/tool contracts, asset readiness/verification, VM-effective settings,
derived rules, locks, MCP/tools/skills, VM profile/revision/asset pins, and
policy assembly provenance.

## Documentation Contract

The public docs site must be updated as part of this redesign. It needs a
coherent section explaining the settings/profile/policy engine, corporate
deployment, signed profile catalogs, package/tool contracts, profile-owned VM
assets, lazy downloads, telemetry, remote policy decisions, custom
profiles/images/rootfs dependencies, debug-report provenance, and the new
architecture/security/configuration model.

Existing docs that describe v1 settings, old security levels, standalone `[mcp]`,
or defaults-json authority must be removed or rewritten before release.
