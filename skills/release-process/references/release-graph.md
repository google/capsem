# Release Graph and Channel Publishing

Reference for `/release-process`: manifest authority, channel runtime
membership, immutable artifact identity, orthogonal CI ownership, and public
activation.

## Authority and authoring

The selected channel source manifest is the sole mutable release authority.
It defines:

- channel identity and policy;
- package and per-binary inventory;
- the channel's one `runtime` document (absent until a runtime is published):
  images, inventory, OBOM, evidence, and revision;
- minimum/maximum compatibility bounds where declared;
- recorded digests and immutable URLs.

`capsem-admin` is the only manifest author. This applies to Capsem-owned
channels and to corporate manifests. Do not add alternate scripts,
workflow-only JSON mutation, a result file, approval ledger, or handwritten
manifest patch.

Corporate administrators own their corporate manifest. They may choose the
latest compatible Capsem package or pin a compatible version, and point it at a
runtime (`capsem-admin manifest corporate --runtime-manifest/--runtime-base`),
while copied official package rows preserve Capsem's provenance. They do not build or mutate Capsem-owned
binaries or public channels.

## Channel runtime model

Each channel carries at most one runtime:

- stable and nightly publish their runtimes independently;
- a binary-only first release has no `runtime` key;
- updating one channel's runtime cannot mutate another channel;
- every immutable runtime URL includes channel and revision
  (`/runtime/releases/<channel>/<revision>/<arch>/<file>`) so the same
  revision label in two channels cannot alias or overwrite bytes.

The root `channels.json` points to public channel manifests. Each selected
manifest owns its package inventory and its runtime. Retained manifest
records use one status value: `current`, `supported`, `deprecated`, or
`revoked`.

Packages are delivery containers. Each newly authored package row records the
binary lane's exact `source_commit`; binary inventory is nested under it with
version, installed path, digests, and SBOM component reference. The
`runtime` document records the runtime lane's exact `source_commit` and owns
its images, software inventory, OBOM/evidence, digests, architecture coverage,
and compatible Capsem version bounds. Legacy
rows may omit the field; top-level and per-binary source fields are forbidden.

SBOM, OBOM, existing attestations, the manifest, structured gate run log, and
GitHub workflow logs are the evidence. Attempt/run id remains separate from
source commit. Do not add a parallel provenance document.

## Immutable and mutable paths

Mutable pointers:

- `/channels.json`
- `/assets/<channel>/manifest.json`
- generated human channel pages

These must use no-cache/revalidation policy.

Immutable package and runtime artifacts use content-verified URLs and
immutable cache policy. Every public reference must resolve through the graph;
bare local paths are invalid.

The generated distribution lives under
`cache/target/release/distribution/assets/<channel>/manifest.json`. Public selectors are
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
operation; it is not a substitute package or a second ledger. Run the runtime
lane first so it serializes the replacement runtime. The binary lane must
refuse the retired empty source until that runtime exists, then supply and
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
runtime parent workflow. Preview deployment is separate and cannot mutate
production source state.

`release-channel-staging.yaml` proves the reusable deployer on a preview branch
without invoking VM asset builds or host package builds.

## Lane ownership

| Lane | May write | Must never touch | Required contract |
|---|---|---|---|
| Binary release | Selected channel packages, per-binary inventory, host SBOM, existing attestations | Runtime data or another channel | `test_binary_lane_gate` |
| Runtime release | One channel's runtime images, evidence, revision, matching digests | Packages, binaries, other channels | `test_runtime_lane_gate` |
| Manifest validation | Channel definitions, bounds, membership | Artifact bytes | `test_release_lane_diff_policy` |
| Channel deploy | Generated public distribution | Source manifests | `test_channel_deploy_contract` |
| Corporate authoring | Corporate manifest through `capsem-admin` | Capsem-owned channels and binaries | `test_corporate_manifest_contract` |

### Binary lane

`just release-binaries <channel> <source-commit>`:

1. acquires the channel lock;
2. reads the latest source manifest;
3. resolves the referenced runtime by digest, including a compatible staged
   runtime;
4. builds packages and binary evidence only;
5. runs the complete functional, native-install, and glow-up proof against the
   resulting runtime;
6. mutates only binary-owned fields;
7. deploys the generated channel after all gates pass.

It never invokes a runtime/image builder.

### Runtime lane

`just release-assets <channel> <source-commit>` calls:

```text
capsem-admin release --channel <channel> --source-commit <source-commit>
```

which dispatches `release-assets.yaml`. The locked workflow:

1. reads the latest source manifest;
2. resolves the existing package by digest;
3. builds the runtime for every published architecture;
4. publishes its immutable images/inventory/OBOM/evidence as
   `runtime-<channel>-<revision>`;
5. validates the unchanged package pairing;
6. mutates only the channel's `runtime` entry;
7. deploys immediately when compatible.

It never invokes a package builder.

### Dependent runtime then binary

If the runtime needs newer code, the runtime lane publishes the immutable
runtime once and persists it as staged source state without changing the
public channel. The subsequent binary lane resolves that exact staged runtime
by digest, builds the new packages, runs the complete final-pairing proof, and
activates the channel. Nothing is rebuilt twice.

## Test composition

Local `just test` rebuilds every package and the runtime, then runs all shared
modules. Release CI calls those modules against the exact lane output plus
digest-resolved complementary artifacts.

Every activated pairing requires artifact validation, all VM suites,
Winterfell/MCP lifecycle, IronBank, injection, integration, benchmarks, full
`capsem-doctor`, native installation, and update glow-up. A staged incompatible
runtime is not user-visible and does not count as an activated pairing.

## Public verification

After deployment, verify:

- root catalog and selected manifest shape;
- SHA-256/BLAKE3 and byte sizes for referenced artifacts;
- package and per-binary inventory;
- runtime images, architecture completeness, software inventory, OBOM, and
  evidence;
- attestation subjects and evidence references;
- mutable versus immutable cache headers;
- human HTML shows the same package/runtime state as the JSON graph;
- stateful installed transitions preserve the previous working state on any
  compatibility, integrity, or application failure.

VM asset attestations are incomplete unless
`github_attestations_vm_assets` is present and its `predicate_url` points at
the published VM OBOM evidence.
