# Release checklist

The normative contract is `tmp/release-spec.md`. Release managers have exactly
two commands:

```text
just release-binaries <channel> <source-commit>
just release-profile <channel> <profile> <source-commit>
```

There is no generic or combined release command.

## One-command gate

Prepare the version cohort, changelog section, and `LATEST_RELEASE.md` in an
ordinary reviewed commit on `main`; there is no separate qualification
command. Pass that full lowercase commit to the release command. The command
requires it on local and fresh remote `main`, materializes a detached private
repository at `<prefix-parent>/<source-commit>`, and runs complete `just test`
there. The outer checkout and `main` may advance while it runs. A failure
creates no release ref, version tag, manifest mutation, or workflow dispatch.
After success, the command creates or verifies the immutable lightweight
`capsem-source-<source-commit>` transport ref. It never edits tracked source or
pushes `main`.

## Binary release

Run:

```sh
just release-binaries nightly <source-commit>
# or
just release-binaries stable <source-commit>
```

The command runs complete `just test`, verifies the prepared version and notes,
creates the immutable version tag when absent, dispatches the binary workflow
from the source transport ref, and waits for the exact SHA/ref/title run. The
serialized workflow:

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
just release-profile nightly code <source-commit>
```

The command runs complete `just test`, publishes the immutable source transport
ref, then calls `capsem-admin release` with that commit. The serialized workflow:

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
just release-profile <channel> <profile> <source-commit>
just release-binaries <channel> <source-commit>
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
logs are the evidence. Newly authored package rows record the binary source
commit; the selected profile document records the profile source commit. Run
ids remain retry identities, not source identities. Do not create a parallel
release ledger or result file.

## Failure rules

- Never move or reuse an immutable tag or profile identity.
- Fix forward with a new commit/version.
- Never bypass a red package, functional, glow-up, lane, or deployment gate.
- A failed or incompatible candidate may remain immutable and inactive; it must
  not alter the public channel.
- Production deployment is only through the generated-distribution workflow
  called by a parent holding the channel lock.
