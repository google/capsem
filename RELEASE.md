# Release checklist

The normative contract is `tmp/release-spec.md`. Release managers have exactly
two commands:

```text
just release-binaries <channel>
just release-profile <channel> <profile>
```

There is no generic or combined release command.

## Before either release

- `main` and required PR gates are green.
- `just doctor` passes.
- The relevant focused tests pass.
- `just test` has proved the complete local construction pipeline after the
  final implementation change.
- The selected channel/profile and compatibility bounds are intentional.
- For a binary release, `[Unreleased]` contains the release notes.

## Binary release

Run:

```sh
just release-binaries nightly
# or
just release-binaries stable
```

The script validates repository state, stamps the version and release notes,
creates and pushes the immutable tag, dispatches the binary workflow, and waits
for it. The serialized workflow:

1. Acquires `capsem-release-<channel>`.
2. Resolves the latest channel source manifest inside that lock.
3. Pulls and verifies every referenced profile, including staged profiles.
4. Builds only the macOS and Linux package cohort.
5. Installs the exact publishable packages.
6. Runs shared static, artifact, complete functional, native/glow-up, and
   release-contract modules against the resolved profile set.
7. Mutates package, binary inventory, host SBOM, and existing attestation
   fields through `capsem-admin`.
8. Generates, verifies, and deploys the complete public distribution.

The binary workflow must never invoke profile, kernel, initrd, rootfs, or image
builders.

## Profile release

Run:

```sh
just release-profile nightly code
```

This calls `capsem-admin release`. The serialized workflow:

1. Acquires the same `capsem-release-<channel>` lock.
2. Resolves the latest source manifest and pulls its current package.
3. Builds exactly the selected channel/profile and its contained assets.
4. Runs the shared complete test modules against the pulled package.
5. Derives and verifies the immutable profile publication identity.
6. Mutates only the selected profile entry through `capsem-admin`.
7. Deploys immediately when the existing package satisfies the profile bounds.

The profile workflow must never build native packages or release binaries.

## Profile requiring new code

Run the normal commands in order:

```sh
just release-profile <channel> <profile>
just release-binaries <channel>
```

The first command builds and publishes the profile once but does not expose an
incompatible public graph. The second command pulls that staged profile, builds
the required package cohort, runs the complete functional and glow-up proof,
and activates the compatible graph. Nothing is rebuilt twice.

## Required proof before activation

Every activated pairing must pass:

- manifest, package, binary inventory, SBOM, profile config/image, OBOM,
  evidence, digest, architecture, and guest-boot validation;
- all VM suites, Winterfell, MCP lifecycle, IronBank, injection, integration,
  benchmarks, and full `capsem-doctor`;
- exact native install;
- manifest polling, binary-only update, profile-only update,
  profile-then-binary update, channel switching, tamper rejection, and
  preservation of the prior working state.

The manifest is the authority. SBOM, OBOM, attestations, and GitHub workflow
logs are the evidence. Do not create a parallel release ledger or result file.

## Failure rules

- Never move or reuse an immutable tag or profile identity.
- Fix forward with a new commit/version.
- Never bypass a red package, functional, glow-up, lane, or deployment gate.
- A failed or incompatible candidate may remain immutable and inactive; it must
  not alter the public channel.
- Production deployment is only through the generated-distribution workflow
  called by a parent holding the channel lock.
