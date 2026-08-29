# Release Graph and Channel Publishing

Reference for `/release-process`: manifest authority, channel/profile
membership, immutable artifact identity, orthogonal CI ownership, and public
activation.

## Authority and authoring

The selected channel source manifest is the sole mutable release authority.
It defines:

- channel identity and policy;
- package and per-binary inventory;
- profile membership;
- profile config, images, inventory, OBOM, evidence, and revision;
- minimum/maximum compatibility bounds where declared;
- recorded digests and immutable URLs.

`capsem-admin` is the only manifest author. This applies to Capsem-owned
channels and to corporate manifests/profiles. Do not add alternate scripts,
workflow-only JSON mutation, a result file, approval ledger, or handwritten
manifest patch.

Corporate administrators own their corporate manifest and profiles. They may
choose the latest compatible Capsem package or pin a compatible version. They
pass the exact commit that built their profiles, while copied official package
rows preserve Capsem's provenance. They do not build or mutate Capsem-owned
binaries or public channels.

## Channel/profile model

Profiles belong to channels:

- a profile may exist in stable and nightly independently;
- a profile may exist only in nightly;
- updating one channel/profile cannot mutate another profile or channel;
- every immutable profile URL must include channel and profile identity so the
  same revision label in two channels cannot alias or overwrite bytes.

The root `channels.json` points to public channel manifests. Each selected
manifest owns its package inventory and profile membership. Retained manifest
records use one status value: `current`, `supported`, `deprecated`, or
`revoked`.

Packages are delivery containers. Each newly authored package row records the
binary lane's exact `source_commit`; binary inventory is nested under it with
version, installed path, digests, and SBOM component reference. Each newly
authored selected profile document records the profile lane's exact
`source_commit` and owns its config, images, software inventory, OBOM/evidence,
digests, architecture coverage, and minimum compatible Capsem version. Legacy
rows may omit the field; top-level and per-binary source fields are forbidden.

SBOM, OBOM, existing attestations, the manifest, structured gate run log, and
GitHub workflow logs are the evidence. Attempt/run id remains separate from
source commit. Do not add a parallel provenance document.

## Immutable and mutable paths

Mutable pointers:

- `/channels.json`
- `/assets/<channel>/manifest.json`
- generated human channel/profile pages

These must use no-cache/revalidation policy.

Immutable package and profile artifacts use content-verified URLs and
immutable cache policy. Every public reference must resolve through the graph;
bare local paths are invalid.

The generated distribution lives under
`target/distribution/assets/<channel>/manifest.json`. Public selectors are
`https://release.capsem.org/channels.json`,
`https://release.capsem.org/assets/stable/manifest.json`, and
`https://release.capsem.org/assets/nightly/manifest.json`.

## Retiring an exact broken legacy graph

Never infer retirement from a missing artifact, HTTP status, or operator
judgment. The one migration rail is a config-owned first-party channel plus
the exact SHA-256 of its current public manifest. The catalog digest and the
freshly fetched payload digest must both match before `capsem-admin` may author
an empty, inactive, same-channel source. Any channel or byte drift is ordinary
public state and fails through the normal gate.

Retirement only removes the unusable source cohort from the next authoring
operation; it is not a substitute package or a second ledger. Run the profile
lane first so it serializes the replacement profile. The binary lane must
refuse the retired empty source until that profile exists, then supply and
activate the new package cohort through the ordinary complete proof.

## Shared serialization

Both production entry workflows hold:

```yaml
concurrency:
  group: capsem-release-${{ inputs.channel }}
  cancel-in-progress: false
```

The lock begins before the source manifest is read and remains through
resolution, tests, source-manifest mutation, generated-distribution assembly,
and production deployment. A queued job re-reads the manifest only after it
owns the lock.

`release-channel.yaml` deploys a generated distribution. It never authors a
source manifest. Production invocation is valid only from a locked binary or
profile parent workflow. Preview deployment is separate and cannot mutate
production source state.

`release-channel-staging.yaml` proves the reusable deployer on a preview branch
without invoking VM asset builds or host package builds.

## Lane ownership

| Lane | May write | Must never touch | Required contract |
|---|---|---|---|
| Binary release | Selected channel packages, per-binary inventory, host SBOM, existing attestations | Profile data or another channel | `test_binary_lane_gate` |
| Profile release | One channel/profile config, images, evidence, revision, matching digests | Packages, binaries, other profiles, other channels | `test_profile_lane_gate` |
| Manifest validation | Channel definitions, bounds, membership | Artifact bytes | `test_release_lane_diff_policy` |
| Channel deploy | Generated public distribution | Source manifests | `test_channel_deploy_contract` |
| Corporate authoring | Corporate manifest and profiles through `capsem-admin` | Capsem-owned channels and binaries | `test_corporate_manifest_contract` |

### Binary lane

`just release-binaries <channel> <source-commit>`:

1. acquires the channel lock;
2. reads the latest source manifest;
3. resolves every referenced profile by digest, including compatible staged
   profiles;
4. builds packages and binary evidence only;
5. runs the complete functional, native-install, and glow-up proof for every
   resulting profile;
6. mutates only binary-owned fields;
7. deploys the generated channel after all gates pass.

It never invokes a profile/image builder.

### Profile lane

`just release-profile <channel> <profile> <source-commit>` calls:

```text
capsem-admin release --channel <channel> --profile <profile> --source-commit <source-commit>
```

The locked workflow:

1. reads the latest source manifest;
2. resolves the existing package by digest;
3. builds exactly one channel/profile;
4. publishes its immutable config/images/inventory/OBOM/evidence;
5. validates the unchanged package pairing;
6. mutates only the selected profile;
7. deploys immediately when compatible.

It never invokes a package builder.

### Dependent profile then binary

If a profile needs newer code, the profile lane publishes the immutable
profile once and persists it as staged source state without changing the
public channel. The subsequent binary lane resolves that exact staged profile
by digest, builds the new packages, runs the complete final-pairing proof, and
activates the channel. Nothing is rebuilt twice.

## Test composition

Local `just test-clean` rebuilds every package and profile, then runs all shared
modules. Release CI calls those modules against the exact lane output plus
digest-resolved complementary artifacts.

Every activated pairing requires artifact validation, all VM suites,
Winterfell/MCP lifecycle, IronBank, injection, integration, benchmarks, full
`capsem-doctor`, native installation, and update glow-up. A staged incompatible
profile is not user-visible and does not count as an activated pairing.

## Public verification

After deployment, verify:

- root catalog and selected manifest shape;
- SHA-256/BLAKE3 and byte sizes for referenced artifacts;
- package and per-binary inventory;
- profile config, images, architecture completeness, software inventory, OBOM,
  and evidence;
- attestation subjects and evidence references;
- mutable versus immutable cache headers;
- human HTML shows the same package/profile state as the JSON graph;
- stateful installed transitions preserve the previous working state on any
  compatibility, integrity, or application failure.

VM asset attestations are incomplete unless
`github_attestations_vm_assets` is present and its `predicate_url` points at
the published VM OBOM evidence.
