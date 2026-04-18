# Release checklist

Concise pre- and post-release checklist for release managers. For deeper detail on CI, signing, notarization, and post-release verification, see the `/release-process` skill in `skills/release-process/`.

## Pre-release

- [ ] `main` is green on CI.
- [ ] `just doctor` passes with no warnings on your machine.
- [ ] `scripts/preflight.sh` passes (Apple cert, Tauri signing key, notarization creds available).
- [ ] `just test` passes locally (all tests: unit + integration + cross-compile + bench + Docker install e2e).
- [ ] `CHANGELOG.md` `[Unreleased]` section is populated and reads well.
- [ ] `CITATION.cff` `date-released` is current (`just _stamp-version` handles this alongside the other version files).
- [ ] Docs site has a release page at `docs/src/content/docs/releases/<major>-<minor>.md` if this is a minor bump.

## Cut the release

Preferred -- fully automated:

```sh
just cut-release
```

This runs `just test`, bumps the version in `Cargo.toml` / `crates/capsem-app/tauri.conf.json` / `pyproject.toml`, stamps `CHANGELOG.md`, commits, tags, and pushes. CI takes over from the tag push.

Manual path (if `cut-release` fails partway through): see `/release-process` -- it documents the precise steps.

## CI (`release.yaml`)

The tag push triggers the release pipeline (~18 min):

```
preflight ──> build-assets (arm64 + x86_64) ──> build-app-macos ──┐
         └──> test ──────────────────────────────────────────────├──> create-release
         └──> build-app-linux (arm64 + x86_64) ──────────────────┘
```

Watch CI. A failure in any job aborts the release. If `create-release` fails partway:

- The tag is pushed -- don't delete it. Instead, fix the issue and cut a new patch release.
- Re-tagging loses updater continuity (Tauri updater reads `latest.json`).

## Post-release

- [ ] GitHub release page has signed artifacts: `.dmg` (macOS arm64), `.deb` (linux arm64 + x86_64), manifest, checksums.
- [ ] `latest.json` for macOS (Tauri updater) is present and signed.
- [ ] `curl -fsSL https://capsem.org/install.sh | sh` on a clean VM installs the new version.
- [ ] The existing installed client auto-updates (or prompts) on next launch.
- [ ] Docs site rebuilds and the release page shows on `capsem.org`.
- [ ] Close out `[Unreleased]` follow-ups in `CHANGELOG.md` as new unreleased items for the next cycle.

## If things go wrong

- **Codesigning fails** -- `scripts/preflight.sh` should have caught it. If it did not, read `/release-process` and run the p12 conversion fallback.
- **Notarization hangs** -- CI uses `--skip-stapling`; first-time notarization is async and can take hours. Don't block the release on it.
- **Tag pushed but CI aborted** -- never force-push over a tag. Increment the patch version and cut again.
- **Self-updater regression** -- users on the prior version can always download the new release manually from GitHub.
