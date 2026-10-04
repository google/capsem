# Capsem Config Layout

`config/` contains source contracts and templates. Generated artifacts belong
under `cache/target/` and must be produced by `capsem-admin` or the gate.

There are exactly four top-level config directories:

- `settings/`
- `corp/`
- `docker/`
- `data/`

Do not add `admin/`, `default/`, `defaults/`, `guest/`, `preset/`,
`presets/`, `registry/`, `schemas/`, `templates/`, or provider-specific config
roots. If a new product input is needed, it belongs under settings or corp,
and the existing admin validation rail must learn it.

## Directories

- `settings/` contains the settings source and generated support artifacts.
  `settings.toml` is the only settings source file and holds the default
  UI/application preferences. `schema.generated.json` validates the settings
  shape.
  `ui-metadata.toml` and `ui-metadata.generated.json` exist only for UI
  rendering metadata; they must not control runtime behavior.
- `corp/` contains corporate source contracts such as `corp.toml`,
  `enforcement.toml`, and `detection.yaml`.
- `docker/` contains the Docker/Jinja templates and `config/docker/image/`,
  the source of the one VM runtime (package set, EROFS settings, kernel pin
  and patches). `[build.rootfs]` in `config/docker/image/build.toml` owns the independent
  raw-export and packed-EROFS ceilings plus forbidden payload prefixes. The
  ordinary build must enforce those limits before and after compression; they
  are release policy, not test data.
- `data/` contains project data embedded or loaded by code, such as model
  pricing tables.

## Policy at Runtime

A VM's policy is the built-in defaults compiled into the binary, overlaid by
the user's `~/.capsem/settings.toml`, overlaid by `corp.toml`. Corp wins, and
its locked rules cannot be overridden. The service merges the three into one
per-VM file, `vm/active_policy.toml`, at every boot. Applications come from
OCI images, not from config.

Do not hand-edit generated output under `cache/target/`. If a source payload
changes, fix the admin rail and its tests.

## Naming Contract

- `schema` validates the shape of one contract.
- `catalog` lists discovered or materialized instances.
- `metadata` describes UI rendering hints.

Do not introduce `admin`, `guest`, or `registry` as config authorities.
`capsem-admin` is a tool; it does not own product configuration. Settings and
corp own runtime behavior. Settings may have generated UI metadata and JSON
Schema, but those artifacts describe the settings shape only. Settings do not
have a registry.

## Admin Tool Surface

`capsem-admin` may validate, check, build, and generate artifacts from this
config. It must not scaffold product config or create a second source of
truth.

Supported public rails:

- `settings validate`
- `enforcement validate`
- `detection validate`
- `manifest check|generate|corporate`
- `assets channel build|check`
- `image build`

If a new product input is needed, add it to the settings or corp contract and
make the existing validation rail understand it. Do not add `init`, `new`,
`add`, provider-specific, or backend-workspace authoring commands.

## Non-Config

Developer skills live in the repository-level `skills/` directory. Product or
user skills are not mirrored under `config/skills`.

Test fixtures belong under `tests/fixtures/`, not in this source config tree.
