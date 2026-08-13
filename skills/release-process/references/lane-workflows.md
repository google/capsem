# Release Lane Workflows

Read this reference before changing per-channel locking, preview deployment,
profile or binary lane ownership, nightly sequencing, staged-profile
activation, base-image materialization, or corporate authoring.

## Shared per-channel serialization

Both production entry workflows use exactly:

```yaml
concurrency:
  group: capsem-release-${{ inputs.channel }}
  cancel-in-progress: false
```

The workflow acquires the lock before reading the source manifest and holds it
through artifact resolution, tests, source-manifest mutation, generated
distribution assembly, and production deployment.

Consequences:

- binary and profile release work for one channel cannot overlap;
- two profile releases for one channel cannot overlap;
- queued work re-reads the manifest only after acquiring the lock;
- stable and nightly may proceed concurrently;
- preview deployment cannot mutate production source manifests;
- `release-channel.yaml` may deploy production only for a serialized parent
  binary or profile workflow.

`release-channel-staging.yaml` is the preview-only proof of the reusable
deployer. It renders a deterministic generated distribution and deploys a
non-production branch without invoking VM asset builds or host package builds.

The selected channel source manifest is the sole mutable release authority. Do
not add a release result file, pending ledger, last-known-good graph, manual
diff approval record, or parallel authoring path.

`scripts/check-release-workflow.sh` is platform-aware without weakening the
publication boundary. Linux reports absent Apple signing material and CI-owned
`cargo-sbom`/`cdxgen` as not applicable; it never fabricates a key or installs a
secret-dependent macOS toolchain. Present tools are validated, and macOS still
fails closed when the signing key is absent or malformed.

## Profile release

`just release-profile nightly code <source-commit>` invokes:

```bash
capsem-admin release --channel nightly --profile code
```

The locked profile workflow:

1. reads the latest nightly source manifest;
2. resolves and verifies its current package;
3. builds only the `nightly + code` config, images, inventory, OBOM, evidence,
   and architecture cohort;
4. creates an immutable identity containing channel and profile identity;
5. validates digests, bootability, and the unchanged package pairing;
6. mutates only the selected profile entry;
7. deploys immediately when the public package satisfies the profile's
   declared minimum Capsem version.

`capsem-admin release` supplies a unique dispatch identity, finds the workflow
run with that exact identity, and watches it with failure propagation. The
public command does not report success merely because GitHub accepted a queued
dispatch; this preserves serialized profile-then-binary ordering even when the
same channel already has pending work.

If the public package is too old, publish the immutable profile artifacts and
persist the staged source-manifest state, but do not deploy that incompatible
pairing. Other profiles, channels, packages, and binaries remain untouched.

The nightly lane always rebuilds its selected profile assets. The prior-run
artifact resolver is a stable retry mechanism only; using it for nightly would
turn the daily asset build into a no-op and lose the hermetic reproducibility
signal.

Each architecture's CI rail may build the kernel and rootfs as separate
private commands, but `build-assets rootfs` must finish by injecting the
config-owned guest payload into that kernel's minimal initrd and regenerating
the manifest/hash aliases. Uploading the kernel-stage initrd directly would
publish bytes the complete local IronBank graph never qualified.

Before either architecture lane builds, the shared asset rail materializes the
checked-in per-platform base child manifests by exact digest through Docker's
daemon boundary. A cold runner and a warm developer host therefore select the
same base bytes; this is not a reason to widen the release egress helper.

All corporate manifest and profile authoring also goes through `capsem-admin`.
A corporation owns its manifest and profile definitions, may use the latest
compatible Capsem package or pin a compatible version, and never builds or
mutates Capsem-owned binaries or public channels.

## Binary release

`just release-binaries nightly <source-commit>` invokes the checked-in, adversarially tested
binary release script. The locked binary workflow:

1. reads the latest nightly source manifest;
2. resolves every referenced profile by recorded digest, including compatible
   staged profiles;
3. builds only candidate packages, per-binary inventory, host SBOM, and
   existing attestation evidence;
4. runs the complete functional suite for every resulting channel profile;
5. installs the exact native packages and runs binary-update plus
   profile-then-binary glow-up;
6. mutates only package, per-binary inventory, host SBOM, and existing
   attestation fields;
7. assembles and deploys the completed channel only after every gate passes.

The workflow must never invoke a profile/image builder.

Daily nightly automation calls this same binary command path after the profile
commands terminate and queues behind other nightly release work. The binary
workflow always rebuilds and runs native install, functional, Winterfell,
IronBank, and glow-up proof. If the version tag is already immutable, the
correlated workflow runs with publication disabled; fresh Apple signing and
notarization timestamps are not byte-identical and may never overwrite an
existing release. A new version identity takes the normal publish-and-activate
path. Stable uses the same command explicitly and the same quality gates, and
has no schedule.

`release-nightly.yaml` owns only sequencing: selected profile commands run with
`max-parallel: 1`, each waits for its exact run, and the binary command runs
after the profile matrix terminates even if a profile failed. The scheduler's
`capsem-nightly-release-scheduler` lock prevents overlapping orchestrators; it
does not replace the downstream workflows' shared
`capsem-release-nightly` transaction lock.

## Dependent profile then binary release

When a profile requires new Capsem code:

1. run `just release-profile <channel> <profile> <source-commit>`;
2. publish the immutable assets once and withhold the incompatible public
   channel;
3. run `just release-binaries <channel> <source-commit>`;
4. resolve the already-built staged profile by digest;
5. run the full functional, native install, and glow-up proof over the
   completed pairing;
6. activate the channel only after success.

Neither artifact family is rebuilt twice.
