# Policy, Settings, Profiles Requirements

Last updated: 2026-05-14

## Scope Boundary

Settings and profiles are different scopes.

- **Service settings** are service/app-scoped: app behavior, service behavior,
  profile roots, asset/manifest/image locations, credential storage for now,
  telemetry export, remote policy plugin endpoints, and other host/service-wide
  integration settings.
- **Profiles** are VM/session-scoped: AI providers, MCP/connectors, skills, VM
  settings, security capabilities, canonical rules, and derived/generated rules.
- **VM-effective settings** are resolved from a selected profile and attached to
  the VM/session.

Telemetry export and remote policy plugin endpoints are service settings, not
profile settings. Credentials may live in TOML for the cutover. Keychain is
optional stretch work inside the credential brokerage sprint.

Manifests, downloaded VM assets, and custom/saved image roots are also
service/corporate settings. Corp and general service settings must be able to
point Capsem at custom local or remote manifest/image locations, and debug
output must explicitly show where the manifest, boot assets, image roots,
download endpoint, telemetry endpoint, and remote decision endpoint came from.

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

The first built-in profile is "Everyday Work" or equivalent. "Mid security" and
"High security" are not product concepts.

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
settings, profile roots, selected profiles, VM-effective settings, derived rules,
locks, MCP/tools/skills, and policy assembly provenance.

## Documentation Contract

The public docs site must be updated as part of this redesign. It needs a
coherent section explaining the settings/profile/policy engine, corporate
deployment, custom profiles, telemetry, remote policy decisions, custom
images/rootfs dependencies, debug-report provenance, and the new
architecture/security/configuration model.

Existing docs that describe v1 settings, old security levels, standalone `[mcp]`,
or defaults-json authority must be removed or rewritten before release.
