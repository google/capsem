# Release Artifact Install Gate

## Why

The same release SHA currently starts full CI from the `main` push and a
second release workflow from the version tag. The release workflow also runs
`test-install` before building the packages that it later publishes, so the
install test proves a separately rebuilt debug package rather than the exact
release artifacts.

## Decisions

- Full CI is pull-request-only. Releases are explicit `{tag, channel}`
  dispatches; neither main nor tag pushes start a competing full workflow.
- Build signed/notarized macOS and release Linux packages before install gates.
- Upload immutable workflow artifacts from each platform build.
- Install gates download and exercise those exact artifacts.
- GitHub release creation and the selected channel deployment depend on all exact-artifact
  install gates.
- Keep post-publication download/hash verification as a separate final proof.

## Files

- `.github/workflows/ci.yaml`
- `.github/workflows/release.yaml`
- `tests/test_release_doctor_contract.py`
- installer/release contract tests as required by the implementation
- `CHANGELOG.md`

## Ordering

1. Reproduce current trigger and dependency ordering with contract tests.
2. Correct CI triggers and the release-site Python environment.
3. Move release builds ahead of exact-artifact install jobs.
4. Gate publication and channel deployment on those jobs.
5. Verify contracts, installer suites, and release acceptance before replacing
   the cancelled tag.

## Done

- A release SHA does not start full CI from `main`, and a tag push does not
  implicitly start an unparameterized release.
- The tag workflow builds each ship artifact once.
- Linux and macOS install gates consume workflow-downloaded release artifacts.
- No release or channel publication can occur before those gates pass.
- The replacement tag has one release workflow and passes post-release checks.

## Proof Matrix

| Slice | Proof |
|---|---|
| Unit/contract | Workflow trigger, artifact upload/download, dependency-order, and publication-gate assertions |
| Functional | Existing macOS/Linux installer script tests against downloaded artifact inputs |
| Adversarial | Contract assertions reject pre-build install gates, missing artifacts, and publication without both platform gates |
| E2E/VM | GitHub Actions replacement release installs exact packages on platform runners |
| Ironbank | Release gate black-box proof: exact artifact identity flows build -> install -> publish; relevant installer/release tests must be non-skipped |
| Telemetry | Not applicable; no ledger behavior changes |
| Performance | Record workflow job durations; no release latency claim until measured |
