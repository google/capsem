# Capsem Config Layout

`config/` contains source contracts and templates. Generated runtime config
belongs under `target/config/` and must be produced by `capsem-admin`.

## Directories

- `settings/` contains UI/application preference source and generated support
  artifacts. `settings.toml` is the only settings source file.
  `schema.generated.json` validates the settings shape. `ui-metadata.toml` and
  `ui-metadata.generated.json` exist only for UI rendering metadata; they must
  not control profile runtime behavior.
- `corp/` contains corporate source contracts such as `corp.toml`,
  `enforcement.toml`, and `detection.yaml`.
- `profiles/<profile_id>/` contains profile source ledgers and profile-owned
  payloads: rules, Sigma detections, MCP declarations, package lists, build
  hooks, tips, and guest root seed manifests.
- `docker/` contains Docker/Jinja templates used by the profile image builder.
- `data/` contains project data embedded or loaded by code, such as model
  pricing tables.

## Source vs Runtime

Checked-in `config/profiles/<profile_id>/profile.toml` is source. It must not
contain asset or sibling-file `hash` or `size` pins. `capsem-admin` validates
source profiles, materializes hashes and sizes into `target/config/`, and uses
that same materialized output for local builds, CI, packages, and installed
runtime config.

Do not hand-edit generated `target/config` output. Do not hand-edit profile
hashes. If a source payload changes, fix the admin materialization rail and its
tests.

## Naming Contract

- `schema` validates the shape of one contract.
- `catalog` lists discovered or materialized instances.
- `metadata` describes UI rendering hints.

Do not introduce `admin` or `registry` as config authorities. `capsem-admin` is
a tool; it does not own product configuration. Profiles and corp own runtime
behavior.

## Admin Tool Surface

`capsem-admin` may validate, check, materialize, build, and generate artifacts
from this config. It must not scaffold product config or create a second source
of truth.

Supported public rails:

- `profile validate|check|materialize`
- `settings validate`
- `enforcement validate`
- `detection validate`
- `manifest check|generate`
- `image build`

If a new product input is needed, add it to the profile/corp/settings contract
and make the existing validation/materialization rail understand it. Do not add
`init`, `new`, `add`, provider-specific, or backend-workspace authoring
commands.

## Non-Config

Developer skills live in the repository-level `skills/` directory. Product or
user skills are not mirrored under `config/skills`; when implemented, they must
be profile-owned payloads with an explicit profile contract.

Test fixtures belong under `tests/fixtures/`, not in this source config tree.
