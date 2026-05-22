---
title: Settings Architecture
description: Profile V2 service settings, profile discovery, and effective VM settings.
---

# Settings Architecture

Capsem settings are Profile V2-only. Host state lives in `service.toml` and
profile TOML files; VM runtime state is a resolved, session-local
`vm-effective-settings.toml` attachment.

There are two different contracts:

| Contract | Scope | Owned by |
|---|---|---|
| Service settings | App/service control plane: profile roots, default profile, catalog source, telemetry export, remote policy plugin config, credential references, and asset/cache locations. | `service.toml` plus `capsem.service-settings.v2` schema |
| Profiles | VM/session product policy: package and tool assumptions, VM resources, AI providers, MCP servers, skills, security capabilities, and policy rules. | Profile V2 payloads plus signed profile catalog |

Do not put VM/session policy into service settings. Do not put service-wide
profile roots, telemetry endpoints, or credential backend configuration into a
profile.

## Sources

```mermaid
flowchart TD
  S["service.toml"] --> R["Profile V2 resolver"]
  B["Built-in profiles"] --> R
  C["Corp profile dirs"] --> R
  U["User profile dirs"] --> R
  CD["corp_directives"] --> R
  R --> E["vm-effective-settings.toml"]
  E --> P["capsem-process policy and guest boot config"]
```

`service.toml` selects the default profile, declares profile roots, stores
credential references, and carries corp directives. Profile files describe
capabilities, AI providers, standard MCP servers, VM resources, and policy
rules.

Profiles also carry an `editable` block for section-level governance. Each
boolean marks whether user-facing mutation routes may change that section after
the profile is selected or forked. For example, a corp profile can allow
`editable.skills = true` and `editable.mcpServers = true` while keeping
`editable.ai = false` and `editable.security_rules = false`. Forks preserve the
same editability map, and profile update routes cannot mutate the map itself.

## Service Settings V2

Service settings use schema id `capsem.service-settings.v2`. The committed
schema artifact is:

```text
schemas/capsem.service-settings.v2.schema.json
```

The Python admin model is `ServiceSettingsV2` in
`src/capsem/builder/service_settings.py`. JSON enters through Pydantic
`model_validate_json()` and JSON leaves through `model_dump_json()`. TOML is
parsed once and immediately validated through the same Pydantic model.

The supported admin commands are:

```bash
capsem-admin settings init --out service.toml
capsem-admin settings schema
capsem-admin settings validate service.toml
capsem-admin settings validate service.toml --json
capsem-admin settings doctor service.toml
capsem-admin settings doctor service.toml --json
```

`settings init` writes a valid JSON or TOML draft from the typed
`ServiceSettingsV2` model. Use `--base-dir`, `--corp-dir`, `--user-dir`,
`--default-profile`, and `--assets-dir` to seed the service control plane
without hand-authoring the initial shape.

`settings doctor` reports the schema id, default profile, profile-catalog
configuration, telemetry state, remote-policy state, and credential backend
without printing credential values.

Profile V2 admin commands currently include:

```bash
capsem-admin profile init corp-dev --out corp-dev.profile.json
capsem-admin profile init corp-dev --out corp-dev.profile.toml
capsem-admin profile schema
capsem-admin profile validate corp-dev.profile.json
capsem-admin profile validate corp-dev.profile.json --json
capsem-admin image plan corp-dev.profile.toml --json
capsem-admin image build-workspace corp-dev.profile.toml --out build/corp-dev-image --arch all --json
capsem-admin image build corp-dev.profile.toml --out assets/ --arch all --template rootfs --json
capsem-admin image verify corp-dev.profile.toml --assets-dir assets/ --json
capsem-admin image sbom corp-dev.profile.toml --assets-dir assets/ --out-dir sboms/
capsem-admin image verify corp-dev.profile.toml --assets-dir assets/ --arch arm64 --inventory assets/arm64/image-inventory.json --json
capsem-admin image verify corp-dev.profile.toml --assets-dir assets/ --doctor-bundle doctor-bundle.tar --json
capsem-admin manifest generate --profiles profiles/ --base-url https://profiles.example.com/catalog/ --out manifest.json
capsem-admin manifest check manifest.json --fast --json
capsem-admin manifest check manifest.json --download --download-dir downloaded/ --pubkey profile-sign.pub --json
capsem-admin manifest sign manifest.json --key manifest-sign.key --out manifest.json.minisig
capsem-admin manifest verify-signature manifest.json --signature manifest.json.minisig --pubkey manifest-sign.pub --json
capsem-admin policy schema
capsem-admin policy validate corp-policy.toml --json
capsem-admin detection schema
capsem-admin detection validate corp-detections.yml --json
capsem-admin detection compile corp-detections.yml --out detection.ir.json --json
capsem-admin detection backtest corp-detections.yml --events policy-contexts.jsonl --json
```

`profile init` writes a valid JSON or TOML draft for the selected profile id.
The draft uses Profile V2 defaults, includes both release architectures, and
should be edited before signing or publishing. `image plan` derives a typed
build plan from the profile's package/tool contract, VM resources, and declared
per-architecture assets; it defaults to all supported release architectures and
can be narrowed with `--arch arm64` or `--arch x86_64`. `image build-workspace`
materializes a generated build workspace from the same profile contract, so the
profile is the source of truth and generated `guest/config` TOML is only an
intermediate for the current Docker templates. `image verify` consumes
the derived plan and checks local assets under
`<assets-dir>/<arch>/<asset filename>` for existence, declared byte size, and
BLAKE3 hash before a manifest or release workflow trusts them. Verification
also checks the profile's apt, Python, node, and required-tool contract through
`<assets-dir>/<arch>/image-inventory.json`; missing inventory for any selected
architecture fails verification. Passing `--inventory` is only needed for a
non-standard single-arch inventory file or alternate inventory directory.
Passing `--doctor-bundle` attaches the result of an in-VM
`capsem-doctor --bundle` probe so release checks can prove the image boots and
keeps Capsem's runtime invariants, not only that the built files hash correctly.
`image sbom` turns the same typed inventories into per-architecture SPDX 2.3
guest-image SBOMs tied to the profile id, revision, and package-contract hash.

`manifest check --fast` validates the signed profile-catalog manifest shape and
performs cheap reachability checks. Local `file://` profile payloads are hashed
and validated against their manifest profile id and revision; HTTP(S) profile
payload and signature URLs are checked with `HEAD` without downloading bytes.
`manifest check --download` fetches every referenced profile payload, profile
signature, VM asset, and VM asset signature, then verifies profile payload
hashes plus profile-declared VM asset sizes and BLAKE3 hashes. With `--pubkey`,
it also verifies downloaded profile and VM asset `.minisig` files with
`minisign`.

`manifest generate` creates the Profile V2 catalog manifest from local JSON or
TOML profile payloads. It hashes the exact payload bytes that will be published,
derives `.minisig` URLs, chooses the newest active revision as current unless
overridden with `--current profile=revision`, and supports
`--status profile@revision=deprecated|revoked` for lifecycle planning.

`manifest sign` and `manifest verify-signature` use the standard `minisign`
tool. Linux admins should install the distro package named `minisign` before
using signing or signature-verification commands.

Policy packs and detection packs are profile-owned security contracts. Policy
packs are enforcement rules and detection packs are finding rules. Detection
packs may contain Sigma YAML, but `capsem-admin detection compile` validates
that YAML with pySigma and emits `capsem.detection.ir.v1` before Rust runtime
code consumes it. See [Enforcement](/security/enforcement/) and
[Detection Format](/security/detection/).

Service settings accept only the V2 shape. Legacy defaults JSON, old v1 policy
config, asset-manifest settings, and ad hoc builder settings are not runtime
compatibility inputs.

## Resolution

1. Load `service.toml`, defaulting missing fields.
2. Discover built-in, corp, and user profiles from the configured roots.
3. Resolve the selected profile inheritance chain.
4. Merge profile values from base to leaf.
5. Apply corp directives after profile inheritance.
6. Emit `vm-effective-settings.toml` into the session directory.

The VM process reads only the session attachment. It does not reopen host
settings files at runtime.

## Policy

Policy rules are authored in Profile V2 sections such as:

```toml
[security.rules.http.block_secret]
on = "http.request"
if = "request.data.contains_secret"
decision = "block"
priority = 10
```

Provider and MCP server toggles can also emit derived rules. Corp profiles
may author corp-priority rules; user profiles are limited to user-priority
ranges.

## MCP

MCP runtime configuration is projected from the effective profile:

- server configuration comes from the profile's standard `mcpServers` map;
- default tool behavior comes from the `mcp_tools` capability;
- per-tool rules come from `mcp.request` rules.

`mcpServers` uses the same top-level shape as common MCP client configs:
stdio servers define `command`, `args`, and `env`; remote servers define `url`,
`headers`, and `bearerToken`. Capsem-only governance belongs under the adjacent
`capsem` object, for example `mcpServers.github.capsem.allowed_tools`.

No standalone MCP settings file is loaded by the VM process.

## Operational Rules

- Setup writes `service.toml` and installs corp profiles under configured
  corp profile roots.
- Support bundles redact `service.toml` and profile TOML.
- Runtime uninstall preserves `service.toml`, profile roots, assets, logs,
  sessions, and persistent VM state.
- Product purge removes the entire Capsem home.
