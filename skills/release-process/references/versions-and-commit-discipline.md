# Versions, Changelog, and Commit Discipline

Read this reference before changing release-note validation, binary or runtime
versioning, compatibility bounds, runtime revision identity, release-set
identities, or release commit/staging practice.

## Documentation, changelog, and versions

Documentation and marketing deploy independently from binary/runtime release
rails. Their builds remain mandatory source gates.

Keep user-visible changes under `## [Unreleased]` in `CHANGELOG.md`. Historical
entries describe past behavior and are not normative release instructions.
The version cohort must agree in the selected commit. Release-note organization
is bookkeeping, not qualification authority: user-visible changes remain under
`Unreleased`, and `LATEST_RELEASE.md` may be rendered from them whenever useful.
Do not add a versioned release heading merely to let the gate start. The remote
immutable version tag is the sole release transition, and the GitHub release
title records the exact source commit.
`just release-binaries` never stamps, commits, resets, or pushes `main`.
Runtime releases are independent and do not require binary changelog text.

Every release command takes the full lowercase source commit explicitly. It
must already be reachable from local and fresh remote `main`. The full-SHA
prefix is the qualification subject, `capsem-source-<commit>` is the workflow
transport ref, and the manifest records that commit only on rows owned by the
publishing family. Attempt ids remain separate so retries never collide.

Binary and runtime versions are orthogonal:

- binary: the Capsem package/application version;
- runtime: the immutable runtime revision, `<workspace version>-<first 12 hex
  of the source commit>`, recorded by `capsem-admin manifest generate` and
  published as `runtime-<channel>-<revision>`.

Do not infer that a runtime change requires a binary rebuild, or that a binary
change requires rebuilding the runtime.

### Semver is mandatory

Every version in the release system is strict semver `MAJOR.MINOR.PATCH`:

- the Capsem binary, whose patch increments -- it is **not** a timestamp;
- the workspace version the runtime revision starts from (locally the runtime
  revision is the workspace version alone);
- `min_capsem_version` / `max_capsem_version`, which bound the **binary** and
  are a separate axis from the runtime revision.

The source-commit suffix gives every released commit its own runtime identity,
so a nightly re-release at an unchanged workspace version never collides with
the previous one. Reusing an identity for different bytes is refused.

This replaced a date-plus-counter scheme (`2026.06.08.9`) that could not order
releases. The date recorded when someone last edited the field rather than when
the assets were built, so a July build shipped wearing a June date; the counter
counted hand-edits, so revisions existed that were never published. Text
comparison also ranks `0.10.0` below `0.9.0`. Never reintroduce a version whose
components are dates, timestamps, or build counters.

## Commit discipline

1. Include the appropriate `CHANGELOG.md` entry for user-visible changes.
2. Stage files explicitly.
3. Use conventional commit subjects.
4. Never stage private release material, certificates, keys, tokens, or
   local-only credentials.
