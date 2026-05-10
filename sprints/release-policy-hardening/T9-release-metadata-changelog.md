# T9: Release Metadata and Changelog

## Objective

Keep version files, changelog, latest-release summary, and release page metadata
consistent after the code fixes land. This track should run late, because the
release text must describe the final shipped behavior rather than the initial
plan.

The target release line is `1.1.xxx`. T9 owns selecting the exact patch/build
suffix and updating release automation if the current stamping path still emits
`1.0.{timestamp}`.

## Owned Files

- `CHANGELOG.md`
- `LATEST_RELEASE.md`
- `Cargo.toml`
- `pyproject.toml`
- `uv.lock`
- `crates/capsem-app/tauri.conf.json`
- `docs/src/content/docs/releases/1-1.md`
- `scripts/extract-release-notes.py`
- `justfile` if release automation needs updates.

## Findings

- [P1] Release metadata existed only as a tracker row, not a dedicated
  sub-sprint with ownership and proof.
- [P1] The release process skill requires version synchronization across
  workspace Rust, Tauri, and Python package metadata.
- [P1] The sprint moved from `1.0.1778378133` to `1.1.xxx`; version files,
  docs, release pages, and stamp automation must be synchronized before tag.
- [P1] `CHANGELOG.md` and release notes currently risk overclaiming configured
  hook dispatch before T8 decides the shipping scope.
- [P2] `LATEST_RELEASE.md` must be regenerated after all user-visible fixes are
  known, not before.

## Task List

### T9.1 Version Synchronization

- [ ] Decide the exact `1.1.xxx` release version.
- [ ] Audit `_stamp-version`, `just install`, and `just cut-release`; update
  them if they still force the old `1.0.{timestamp}` pattern.
- [ ] Do not run `just cut-release` after docs are finalized unless the sprint
  intentionally accepts the exact `1.1.xxx` version produced by the recipe.
- [ ] Confirm the intended release version is `1.1.xxx`, with the exact suffix
  recorded in this file, `MASTER.md`, and `tracker.md`.
- [ ] Verify `Cargo.toml`, `pyproject.toml`, and
  `crates/capsem-app/tauri.conf.json` all carry the same binary version.
- [ ] Verify `uv.lock` reflects the Python package version after stamping.
- [ ] If version changes, run the existing stamp recipe instead of hand-editing
  individual files.

### T9.2 Changelog

- [ ] Move release-policy hardening entries under the correct
  `## [1.1.xxx] - 2026-05-10` section after T9.1 chooses the exact suffix.
- [ ] Use user-facing language: install bootability, signed manifests, Policy
  V2 UI correctness, hook/runtime security hardening, telemetry visibility, and
  docs accuracy.
- [ ] Do not claim configured external hook dispatch unless T8 ships and proves
  it.
- [ ] Include security-relevant fixes under `Security`.

### T9.3 Latest Release Summary

- [ ] Regenerate `LATEST_RELEASE.md` from `CHANGELOG.md` with
  `uv run python3 scripts/extract-release-notes.py`.
- [ ] Treat `LATEST_RELEASE.md` as derived output and review its diff.
- [ ] Ensure summary matches T0/T8 decisions about updater and hook dispatch.
- [ ] Include package install requirements and verification-relevant artifacts
  if user-visible.

### T9.4 Release Page Metadata

- [ ] Create or update `docs/src/content/docs/releases/1-1.md` after T4
  wording is final.
- [ ] Confirm release date is `2026-05-10`.
- [ ] Confirm release text does not contradict package/updater behavior from T0.
- [ ] Confirm release text does not contradict Policy V2/hook scope from T8.

### T9.5 Commit Discipline

- [ ] Include changelog/metadata updates in the same commit as their code fixes
  or in a final release-prep commit if the code is already grouped.
- [ ] Stage files explicitly; do not rely on `git add -A`.
- [ ] Use conventional commit message and configured author when committing.
- [ ] Confirm release commit includes the docs release page if T9 requires it;
  `just cut-release` may not stage that file automatically.
- [ ] Confirm author is `Elie Bursztein <github@elie.net>` and no
  `Co-Authored-By` trailers are added.

## Proof Matrix

| Category | Required proof |
|---|---|
| Unit/contract | version fields agree across Rust, Python, and Tauri metadata. |
| Functional | changelog/latest release/release page describe actual fixed behavior. |
| Adversarial | hook/updater language does not overclaim deferred behavior. |
| Release | commit includes release metadata and explicit staging. |

## Verification

- [ ] `rg -n "1\\.1\\.|1\\.0\\.1778378133|1\\.0\\.\\{timestamp\\}" Cargo.toml pyproject.toml crates/capsem-app/tauri.conf.json CHANGELOG.md LATEST_RELEASE.md docs/src/content/docs/releases`
- [ ] `uv lock --check`
- [ ] `scripts/check-release-workflow.sh`
- [ ] `uv run python3 scripts/extract-release-notes.py`
- [ ] `git diff -- LATEST_RELEASE.md`
- [ ] `rg -n "external Policy Hook|hook endpoint|hook attempts|remote hook|latest\\.json|DMG|\\.dmg|AppImage" CHANGELOG.md LATEST_RELEASE.md docs/src/content/docs/releases/1-0.md`
- [ ] `pnpm -C docs run build`
- [ ] `git status --short`
- [ ] `git diff --cached --name-only`
- [ ] `git var GIT_AUTHOR_IDENT`
- [ ] `cargo metadata --no-deps`
- [ ] `git diff --check -- CHANGELOG.md LATEST_RELEASE.md Cargo.toml pyproject.toml uv.lock crates/capsem-app/tauri.conf.json docs/src/content/docs/releases`

## Exit Criteria

- [ ] Version files are synchronized.
- [ ] No release-facing file still claims `1.0.1778378133` as the target
  release.
- [ ] Release notes match implementation and do not overclaim hook/updater
  behavior.
- [ ] Changelog and latest-release summary are ready for the release commit.
