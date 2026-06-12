# Capsem Config Layout

`config/` contains source contracts and templates. Generated runtime config
belongs under `target/config/` and must be produced by `capsem-admin`.

## Directories

- `admin/` contains admin/tooling source and generated settings registries.
  `settings.toml` is UI/application preference source. Generated files use the
  `.generated.*` suffix and are refreshed by the schema/admin rail.
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

## Non-Config

Developer skills live in the repository-level `skills/` directory. Product or
user skills are not mirrored under `config/skills`; when implemented, they must
be profile-owned payloads with an explicit profile contract.

Test fixtures belong under `tests/fixtures/`, not in this source config tree.
