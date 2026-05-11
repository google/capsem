# Docs and Release Metadata Findings

Status: completed; transferred to T7 FD02 and owner rows in
T0/T4/T6/T9/T10/T11/T12. T4 docs cleanup and T6 telemetry/tooling proof are
implemented; final T9/T11 release metadata and package proof remain open.

Agent: Copernicus (`019e1263-54c4-7292-8d50-9f818cf7779f`)

## Scope

- Docs overclaims around Policy V2 and hooks.
- Package/updater artifact language.
- Changelog, latest-release summary, release page metadata.
- Docs/site build proof.

## Findings

- [x] [P0] Hook dispatch is overclaimed while T8/T2 still say no production
  path loads hook endpoints or calls `PolicyHookClient`.
  - Paths: `docs/src/content/docs/releases/1-0.md`,
    `docs/src/content/docs/security/policy.md`.
  - Proof: stale-term search for hook endpoint/remote hook/external Policy
    Hook language after T8 scope decision.
  - Run:
    `rg -n "external Policy Hook|hook endpoint|hook attempts|remote hook|configured hook|Policy Hook Spec0 callouts|policy_action IN \\('ask', 'deny'|policy_action = 'deny'" LATEST_RELEASE.md docs/src/content/docs/releases/1-0.md docs/src/content/docs/security/policy.md docs/src/content/docs/architecture/session-telemetry.md`
    (no matches).
  - Sprint IDs: T4.1, T8.1, T9.2, T9.4.

- [x] [P0] Active docs/site still advertise stale artifacts and updater state:
  DMG, AppImage, and `latest.json` appear in current docs/site copy.
  - Paths: `README.md`, `docs/src/content/docs/getting-started.md`,
    `docs/src/content/docs/development/ci.md`,
    `docs/src/content/docs/development/stack.md`,
    `docs/src/content/docs/development/just-recipes.md`,
    `docs/src/content/docs/security/build-verification.md`,
    `docs/src/content/docs/architecture/service-architecture.md`,
    `site/src/components/CTA.svelte`.
  - Proof: stale-term search plus docs/site builds.
  - Run:
    `rg -n "dnsmasq|vsock:?5003|DMG|\\.dmg|AppImage|image < 12MB|12MB" README.md docs/src/content/docs site/src`
    (no matches),
    `rg -n "latest\\.json" README.md docs/src/content/docs site/src` (no
    matches), `pnpm -C docs run build` (45 pages), and
    `pnpm -C site run build` (1 page).
  - Sprint IDs: T4.2, T9.2, T10.6.

- [x] [P0] `latest.json` risk is active in config. Tauri updater artifacts are
  enabled and endpoint points at `latest.json`, but release workflow uploads
  `.pkg`, manifest files, SBOM, arch assets, and optional `.deb`.
  - Paths: `crates/capsem-app/tauri.conf.json`,
    `.github/workflows/release.yaml`, `scripts/check-release-workflow.sh`.
  - Proof: release artifact listing must show either real updater artifacts or
    updater disabled/honest UI.
  - Run: T0 disabled unsupported updater config/UI; T4 docs now say the
    desktop updater is disabled unless a future full-package updater feed ships.
  - Sprint IDs: T0.6, T4.2, T9.2, T10.1, T11.1.

- [x] [P1] Session telemetry docs are partially stale around tool-call origin,
  `mcp_call_id`, `policy_hook_events`, and `WriteOp::PolicyHookEvent`.
  - Paths: `docs/src/content/docs/architecture/session-telemetry.md`,
    `crates/capsem-logger`, `scripts/check_session.py`.
  - Proof: docs updated after T6 verifies schema/tooling behavior.
  - Run: `pnpm -C docs run build` (45 pages). T6 old-DB/tooling proof now
    passes; real-session product-path proof remains T8/T10.
  - Sprint IDs: T4.3, T6.3, T6.4.

- [x] [P1] Public site and benchmark docs still contain stale implementation
  claims.
  - Paths: `site/src/lib/data.ts`,
    `docs/src/content/docs/benchmarks/results.md`.
  - Proof: stale-term search for `dnsmasq`, fake DNS, and `12MB`.
  - Run:
    `rg -n "dnsmasq|vsock:?5003|DMG|\\.dmg|AppImage|image < 12MB|12MB" README.md docs/src/content/docs site/src`
    (no matches), `pnpm -C docs run build`, and `pnpm -C site run build`.
  - Sprint IDs: T4.4, T10.6.

- [ ] [P1] T9 needs curated release text, not a huge internal sprint dump.
  `LATEST_RELEASE.md` is generated from `CHANGELOG.md` through the next
  heading, so stale/internal terms become current release copy.
  - Paths: `CHANGELOG.md`, `LATEST_RELEASE.md`,
    `scripts/extract-release-notes.py`.
  - Proof: regenerate and review `LATEST_RELEASE.md` after changelog cleanup.
  - Sprint IDs: T9.2, T9.3.

- [ ] [P1] T9/T11 should pin artifact truth explicitly in release notes/page:
  required macOS `.pkg`, Linux `.deb` status, `manifest.json`,
  `manifest.json.minisig`, `capsem-sbom.spdx.json`, arch-prefixed VM assets,
  and no `latest.json` unless T0 ships it.
  - Paths: `CHANGELOG.md`, `LATEST_RELEASE.md`,
    `docs/src/content/docs/releases/1-0.md`,
    `.github/workflows/release.yaml`.
  - Proof: release artifact and docs stale-term checks.
  - Sprint IDs: T9.2, T9.4, T11.4.

- [ ] [P1] `just cut-release` staging discipline needs explicit extras:
  `uv.lock`, `docs/src/content/docs/releases/1-0.md`, and
  `config/policy-hook-openapi.json` when touched/required.
  - Paths: `justfile`, `scripts/check-release-workflow.sh`,
    `config/policy-hook-openapi.json`.
  - Proof: T9.5 names explicit staged files and `git diff --cached --name-only`.
  - Sprint IDs: T9.5, T11.4.

- [ ] [P1] `scripts/check-release-workflow.sh` still validates the Tauri
  signing key family and says DMG; T0/T11 should require manifest signing key
  validation against `config/manifest-sign.pub`.
  - Paths: `scripts/check-release-workflow.sh`,
    `config/manifest-sign.pub`.
  - Proof: manifest key check and no stale DMG wording.
  - Sprint IDs: T0.7, T11.1.

## Commands To Name

```bash
rg -n "dnsmasq|vsock:?5003|DMG|\\.dmg|AppImage|image < 12MB|12MB|latest\\.json" README.md docs/src/content/docs site/src
rg -n "external Policy Hook|hook endpoint|hook attempts|remote hook|latest\\.json|DMG|\\.dmg|AppImage" LATEST_RELEASE.md docs/src/content/docs/releases/1-0.md docs/src/content/docs/security/policy.md
pnpm -C docs run build
pnpm -C site run build
uv lock --check
uv run python3 scripts/extract-release-notes.py
git diff -- LATEST_RELEASE.md
scripts/check-release-workflow.sh
pkgutil --expand-full packages/Capsem-*.pkg /tmp/capsem-pkg
find /tmp/capsem-pkg -name manifest.json -o -name manifest.json.minisig
dpkg-deb --contents target/release/bundle/deb/*.deb | rg 'manifest\\.json(\\.minisig)?|capsem-mcp-(aggregator|builtin)'
find target/release/bundle -maxdepth 4 -type f | sort | rg 'latest\\.json|\\.tar\\.gz|\\.sig|\\.pkg|\\.deb'
just doctor
scripts/preflight.sh
env UV_CACHE_DIR=/private/tmp/capsem-uv-cache just test
just test-install
just run "capsem-doctor"
```
