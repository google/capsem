# Sprint: Release Artifact Install Gate

## Tasks

- [x] Cancel duplicate active CI and release workflows
- [x] Diagnose duplicate triggers and missing `blake3`
- [x] Add RED workflow contract tests
- [x] Make full CI pull-request-only
- [x] Fix release-site Python dependency setup
- [x] Parameterize one globally serialized release workflow by tag and channel
- [x] Build release artifacts before exact-artifact install gates
- [x] Gate publication on macOS and Linux exact-artifact installs
- [x] Focused verification
- [ ] Full release gate
- [ ] Changelog
- [ ] Commit and replacement release

## Notes

- Discovery: `main` and `v*` pushes independently started full workflows for
  the same SHA.
- Discovery: `release-site-build` called a Python script outside the locked uv
  environment and failed on the declared `blake3` dependency.
- Discovery: release `test-install` built a debug `.deb` before the real
  platform artifacts existed; it did not test the shipped package.
- Changed approach: releases are explicit `workflow_dispatch` runs with a tag
  and one stable/nightly channel. Tag pushes do not launch an unparameterized
  workflow, and a global concurrency group prevents shared-site races.

## Coverage Ledger

- Unit/contract: 187 workflow/installer contracts passed, including all 145
  release-doctor contracts.
- Functional: locked release-site fixture generation passed; parameterized
  `just release` dry-run contains one dispatch with tag and channel.
- Adversarial: contracts reject main/tag push triggers, dual-channel mutation,
  a mismatched dispatch ref/tag, optional Linux builds, and publication before
  exact-artifact install gates.
- E2E/VM: replacement release run pending.
- Ironbank: exact-artifact release acceptance in the replacement live workflow
  remains pending.
- Telemetry: not applicable.
- Performance: workflow timestamps will be recorded; no optimization claim.
- Missing/deferred: none currently.
