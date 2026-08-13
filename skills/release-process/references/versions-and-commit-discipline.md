# Versions, Changelog, and Commit Discipline

Read this reference before changing release-note validation, binary or profile
versioning, compatibility bounds, profile revision advancement, release-set
identities, or release commit/staging practice.

## Documentation, changelog, and versions

Documentation and marketing deploy independently from binary/profile release
rails. Their builds remain mandatory source gates.

Keep user-visible changes under `## [Unreleased]` in `CHANGELOG.md`. Historical
entries describe past behavior and are not normative release instructions.
Before binary qualification, prepare one ordinary reviewed commit that moves
the notes into the selected version section, makes `LATEST_RELEASE.md` match
that section, and leaves `Unreleased` empty. The version cohort must already
agree in that commit. `just release-binaries` validates this before `just test`
when the immutable version tag is new; it never stamps, commits, resets, or
pushes `main`. Profile releases are independent and do not require binary
changelog text.

Every release command takes the full lowercase source commit explicitly. It
must already be reachable from local and fresh remote `main`. The full-SHA
prefix is the qualification subject, `capsem-source-<commit>` is the workflow
transport ref, and the manifest records that commit only on rows owned by the
publishing family. Attempt ids remain separate so retries never collide.

Binary and profile versions are orthogonal:

- binary: the Capsem package/application version;
- profile: the immutable channel/profile publication identity derived and
  authored by `capsem-admin`.

Do not infer that a profile change requires a binary rebuild, or that a binary
change requires rebuilding any profile.

### Semver is mandatory, and each profile versions independently

Every version in the release system is strict semver `MAJOR.MINOR.PATCH`:

- the Capsem binary, whose patch increments -- it is **not** a timestamp;
- every profile revision, first-party and corp-authored alike;
- `min_capsem_version` / `max_capsem_version`, which bound the **binary** and
  are a separate axis from the profile's own revision. A profile at `0.3.2` may
  require capsem `>= 0.6.0`; those numbers are unrelated.

Profiles are orthogonal, so each carries its own revision and advances on its
own schedule. `code` moving to `0.7.0` says nothing about `co-work`. A release
spanning profiles at different revisions has no single version to name and
collapses to a `profiles-<hash>` identifier; that identifier names a set, not a
version, and is deliberately exempt from semver.

`capsem-admin` enforces this: `parse_profile_revision` rejects anything that is
not semver, and `ensure_revision_advances` rejects a revision that does not move
past what is already published. Both run before a release is authored, so a corp
operator meets the same rule.

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
