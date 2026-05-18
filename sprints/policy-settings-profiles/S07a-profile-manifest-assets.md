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
- Status is explicit:
  - `active`: install/update and allow new VMs.
  - `deprecated`: keep installed, warn, allow existing VMs, avoid as default.
  - `removed`: stop offering/installing; local cleanup may remove when unpinned.
  - `revoked`: block new use and surface a high-severity warning for existing
    VMs pinned to it.
- Profile payload identity is verified before the profile is installed or used.

## Profile Contract Additions

Extend profile TOML with a package/tool contract and asset declarations.

Package/tool contract:

```toml
[packages]
python = "3.12.3"
node = "22.1.0"
uv = "0.4.30"

[packages.python_modules]
requests = "2.32.3"
numpy = "1.26.4"

[packages.node_packages]
playwright = "1.44.0"

[tools]
capsem_doctor = ">=1.0.0"
browser = ">=0.1.0"
```

VM assets:

```toml
[vm.assets]
kernel_url = "https://assets.capsem.dev/vm/default/2026.0520.1/vmlinuz"
kernel_hash = "blake3:..."
kernel_signature_url = "https://assets.capsem.dev/vm/default/2026.0520.1/vmlinuz.minisig"

initrd_url = "https://assets.capsem.dev/vm/default/2026.0520.1/initrd.img"
initrd_hash = "blake3:..."
initrd_signature_url = "https://assets.capsem.dev/vm/default/2026.0520.1/initrd.img.minisig"

rootfs_url = "https://assets.capsem.dev/vm/default/2026.0520.1/rootfs.squashfs"
rootfs_hash = "blake3:..."
rootfs_signature_url = "https://assets.capsem.dev/vm/default/2026.0520.1/rootfs.squashfs.minisig"
guest_abi = "capsem-guest-v2"
```

Implementation may normalize the repeated asset fields into typed tables, but
the shipped schema must preserve these invariants:

- Profiles declare the guest package/tool versions their rules, skills, MCP
  connectors, and UI affordances assume.
- Profiles declare the VM assets and verification metadata that satisfy the
  package/tool contract.
- Profiles may inherit package/tool declarations from a parent and override them
  deterministically through the existing resolver pipeline.
- Effective settings and debug/status surfaces expose the package/tool contract
  and resolved asset identity.

## Trust Chain

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

## Service / Resolver Scope

- Add manifest parsing for profile catalog records and revision status.
- Add profile payload download/install/update logic.
- Extend profile schema and effective settings with packages/tools and VM asset
  declarations.
- Resolve the selected profile before provisioning a VM, then ensure that
  profile revision's assets are present. Missing assets download at first use.
- Replace global current-asset selection for profile-backed VMs with
  profile-driven asset resolution.
- Preserve the dev-mode local-asset path for developer builds, but make the
  release/install path profile-driven.
- Extend persistent VM registry with `profile_id`, `profile_revision`, package
  contract hash, and pinned asset hashes.
- Add explicit rebase/migrate semantics later; do not silently move existing
  VMs across profile revisions in this sprint.

## API / UX Hand-Offs

This sprint creates the contract consumed by later sprints:

- S07 exposes installed/catalog profiles, revisions, status, packages/tools,
  asset readiness, and profile-backed VM create/fork options over UDS.
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
- [ ] Extend profile TOML schema with packages/tools and VM asset declarations.
- [ ] Add resolver tests for inherited package/tool contracts and asset
      declarations.
- [ ] Add profile payload install/update/delete/revoke logic from manifest
      records.
- [ ] Add profile-driven asset resolution and first-use download.
- [ ] Add cleanup retention for installed profile revisions plus existing VM
      pins.
- [ ] Add persistent VM profile/revision/package/asset pin metadata.
- [ ] Add functional tests for create VM with selected profile revision,
      first-use download, resume after profile update, deprecated profile, and
      revoked profile fail-closed behavior.
- [ ] Update debug/status fixtures with profile catalog and asset readiness.

## Coverage Ledger

- Unit/contract: manifest v3 parser/validator, profile package/tool parser,
  asset declaration parser, resolver inheritance/override behavior.
- Functional: profile install/update/remove/revoke from manifest; selected
  profile VM creation pins revision and assets; resume preserves VM pins after a
  profile update.
- Adversarial: bad profile id/revision, downgrade attempts, bad signature/hash,
  incompatible binary, revoked profile, missing asset, asset hash mismatch,
  malformed package version.
- E2E/VM or integration: service-level VM create with profile-backed first-use
  asset download; resume an existing VM after catalog update.
- Telemetry/observability: status/debug report catalog state, installed
  revisions, package contract, asset readiness, and VM pin drift/revocation.
- Performance: first-use download is not on hot list/status paths; list/status
  must use cached readiness. Resolver overhead for package/tool inheritance is
  bounded by existing profile-chain depth.
- Missing/deferred: explicit VM rebase/migration UX is deferred until profile
  create/update surfaces are stable; this sprint only pins and reports.
