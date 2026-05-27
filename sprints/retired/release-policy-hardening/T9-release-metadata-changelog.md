# T9: Release Metadata and Changelog

## Objective

Keep version files, changelog, latest-release summary, and release page metadata
consistent after the code fixes land. This track should run late, because the
release text must describe the final shipped behavior rather than the initial
plan.

The target release line is `1.1.1778445002`. T9 owns selecting the exact patch/build
suffix and updating release automation if the current stamping path still emits
`1.0.{timestamp}`.

## Owned Files

- `CHANGELOG.md`
- `LATEST_RELEASE.md`
- `Cargo.toml`
- `pyproject.toml`
- `uv.lock`
- `crates/capsem-app/tauri.conf.json`
- `crates/capsem-mcp-aggregator/Cargo.toml`
- `crates/capsem-mcp-builtin/Cargo.toml`
- `docs/src/content/docs/releases/1-1.md`
- `scripts/extract-release-notes.py`
- `justfile` if release automation needs updates.

## Findings

- [P1] Release metadata existed only as a tracker row, not a dedicated
  sub-sprint with ownership and proof.
- [P1] The release process skill requires version synchronization across
  workspace Rust, Tauri, and Python package metadata.
- [P1] The sprint moved from `1.0.1778378133` to `1.1.1778445002`; version files,
  docs, release pages, and stamp automation must be synchronized before tag.
- [P1] `CHANGELOG.md` and release notes currently risk overclaiming configured
  hook dispatch before T8 decides the shipping scope.
- [P2] `LATEST_RELEASE.md` must be regenerated after all user-visible fixes are
  known, not before.

## Swarm Transfer Tracker

| Source | Priority | Owner task | Required transfer point | Required proof |
|---|---:|---|---|---|
| FD02 docs-release-metadata | P0 | T9.2, T9.4 | Release metadata must not overclaim hook dispatch unless T8 ships it. | Changelog, latest release, and release page stale-term searches pass after T8 scope decision. |
| FD02 docs-release-metadata | P0 | T9.2 | Release copy must not advertise stale artifacts/updater state from DMG/AppImage/`latest.json`. | Artifact/updater stale-term search passes or only contains explicit deferred updater wording. |
| FD02 docs-release-metadata | P1 | T9.2, T9.3 | `LATEST_RELEASE.md` is generated from `CHANGELOG.md`; internal sprint dump or stale terms can leak into public release copy. | Regenerate `LATEST_RELEASE.md`, review diff, and confirm curated user-facing text. |
| FD02 docs-release-metadata | P1 | T9.2, T9.4 | Release notes/page must pin artifact truth for `.pkg`, `.deb`, signed manifest, SBOM, arch VM assets, and no updater feed unless shipped. | T9 release text matches T0/T12 asset expectations. |
| FD02 docs-release-metadata | P1 | T9.5 | `just cut-release` staging discipline must include `uv.lock`, release docs, and `config/policy-hook-openapi.json` when touched/required. | `git diff --cached --name-only` is recorded and contains all required release files. |
| FD06 cli-updater-install | P2 | T9.1, T9.3 | User-facing version/update truth is stale or misleading. | About/CLI/release copy show stamped version and honest update support. |
| FD13 ci-release-landing-1-1 | P0 | T9.1 | Version stamping still emits old `1.0.{timestamp}` release line. | `_stamp-version`, metadata files, and release-facing docs select exact `1.1.1778445002` before tag. |
| FD14 swarm-transfer-closeout | P1 | T9.1 | Planning docs must consistently target `1.1.1778445002` until exact suffix is selected. | `rg` for old target shows only historical/stale-reference checks. |

## Task List

### T9.1 Version Synchronization

- [x] Decide the exact `1.1.1778445002` release version.
- [x] Audit `_stamp-version`, `just install`, and `just cut-release`; update
  them if they still force the old `1.0.{timestamp}` pattern.
- [x] Do not run `just cut-release` after docs are finalized unless the sprint
  intentionally accepts the exact `1.1.1778445002` version produced by the recipe.
- [x] Confirm the intended release version is `1.1.1778445002`, with the exact suffix
  recorded in this file, `MASTER.md`, and `tracker.md`.
- [x] Verify `Cargo.toml`, `pyproject.toml`, and
  `crates/capsem-app/tauri.conf.json` all carry the same binary version.
- [x] Verify `uv.lock` reflects the Python package version after stamping.
- [x] Verify internal path dependencies do not pin stale `capsem-guard`
  versions from the old release line.
- [x] If version changes, run the existing stamp recipe instead of hand-editing
  individual files.

### T9.2 Changelog

- [x] Move release-policy hardening entries under the correct
  `## [1.1.1778445002] - 2026-05-10` section after T9.1 chooses the exact suffix.
- [x] Use user-facing language: install bootability, signed manifests, Policy
  V2 UI correctness, hook/runtime security hardening, telemetry visibility, and
  docs accuracy.
- [x] Do not claim configured external hook dispatch; T8.1 defers it for
  `1.1.1778445002` and ships only Spec0/client/audit infrastructure plus non-hook
  Policy V2 enforcement.
- [x] Include security-relevant fixes under `Security`.

### T9.3 Latest Release Summary

- [x] Regenerate `LATEST_RELEASE.md` from `CHANGELOG.md` with
  `uv run python3 scripts/extract-release-notes.py`.
- [x] Treat `LATEST_RELEASE.md` as derived output and review its diff.
- [x] Ensure summary matches T0/T8 decisions about updater and deferred hook
  dispatch.
- [x] Include package install requirements and verification-relevant artifacts
  if user-visible.

### T9.4 Release Page Metadata

- [x] Create or update `docs/src/content/docs/releases/1-1.md` after T4
  wording is final.
- [x] Confirm release date is `2026-05-10`.
- [x] Confirm release text does not contradict package/updater behavior from T0.
- [x] Confirm release text does not contradict Policy V2/hook scope from T8:
  non-hook Policy V2 ships, configured external hook dispatch does not.

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

- [x] `CAPSEM_RELEASE_VERSION=1.1.1778445002 just _stamp-version`
- [x] `env UV_CACHE_DIR=/private/tmp/capsem-uv-cache uv lock --check`
- [ ] `scripts/check-release-workflow.sh`
  - Current T11-era result: 11 passed, 1 failed because the local default-path
    `private/manifest-sign/capsem.key` is not present. `minisign` is now
    installed and local/dev manifest signing works; production release secrets
    are CI-provided.
- [x] `env UV_CACHE_DIR=/private/tmp/capsem-uv-cache uv run python3 scripts/extract-release-notes.py`
- [x] `git diff -- LATEST_RELEASE.md`
- [x] `rg -n "1\\.0\\.1776688771|1\\.0\\.1778378133|1\\.0\\.\\{timestamp\\}" Cargo.toml crates/*/Cargo.toml pyproject.toml crates/capsem-app/tauri.conf.json uv.lock LATEST_RELEASE.md docs/src/content/docs/releases/1-1.md`
  - No output; historical `CHANGELOG.md`/`docs/releases/1-0.md` old-release
    entries are intentionally outside this active-release scan.
- [x] `rg -n "external Policy Hook|hook endpoint|hook attempts|remote hook|latest\\.json|DMG|\\.dmg|AppImage" LATEST_RELEASE.md docs/src/content/docs/releases/1-1.md`
  - No output.
- [x] `pnpm -C docs run build`
- [x] `git status --short`
- [ ] `git diff --cached --name-only`
- [x] `git var GIT_AUTHOR_IDENT`
- [x] `cargo metadata --no-deps --format-version 1`
- [x] `git diff --check -- CHANGELOG.md LATEST_RELEASE.md Cargo.toml pyproject.toml uv.lock crates/capsem-app/tauri.conf.json crates/capsem-mcp-aggregator/Cargo.toml crates/capsem-mcp-builtin/Cargo.toml docs/src/content/docs/releases justfile scripts/check-release-workflow.sh skills/release-process/SKILL.md sprints/release-policy-hardening`

## Exit Criteria

- [x] Version files are synchronized.
- [x] No release-facing file still claims `1.0.1778378133` as the target
  release.
- [x] Release notes match implementation and do not overclaim hook/updater
  behavior.
- [x] Changelog and latest-release summary are ready for the release commit;
  explicit staging/commit proof remains T9.5/T11 debt.
