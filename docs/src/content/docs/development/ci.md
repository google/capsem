---
title: CI/CD
description: How CI workflows run, what they test, and how to debug failures.
sidebar:
  order: 6
---

Capsem uses GitHub Actions for continuous integration and release automation.

## Workflows

| Workflow | Trigger | What it does |
|----------|---------|-------------|
| `ci.yaml` | Pull requests and push to main | PR quality gate: Rust unit/integration, frontend, Python contracts, install checks, and explicit runner substitutions |
| `release.yaml` | Tag push (`v*`) | Build apps (macOS + Linux), package with the current public asset manifest, create GitHub release, then update release.capsem.org binary metadata |
| `release-assets.yaml` | Manual | Build VM assets, generate `assets/manifest.json`, and optionally deploy the asset channel |
| `docs.yaml` | Push to main | Deploy docs.capsem.org on each main merge, then smoke the live docs site |
| `site.yaml` | Push to main | Deploy capsem.org on each main merge, then smoke the live marketing site |
| `release-channel.yaml` | Called by binary or asset release | Deploy release.capsem.org from the generated release-channel site artifact |

## CI workflow (`ci.yaml`)

Runs on every pull request and push to `main`. Pull requests should require the
stable `pr-gate` status before merge.

### test-linux (ubuntu-24.04-arm)

Tests the KVM backend, which only compiles on Linux:

1. Enable `/dev/kvm` via udev rules
2. Unit tests with coverage for every portable workspace crate
3. Verify KVM tests actually ran (not silently skipped)
4. Upload coverage to Codecov with `linux-unit` flag

### test (macos-14)

Hosted-runner quality suite on macOS:

1. **Dependency audit** -- `cargo audit` + `pnpm audit`
2. **Rust unit tests with coverage** -- every workspace crate, including macOS-only app/tray crates
3. **Rust integration tests** -- cross-crate tests from `tests/` directory
4. **Frontend** -- type check (`astro check` + `svelte-check`), vitest with coverage, production build
5. **Python schema tests** -- capsem-builder tests with 90% coverage floor
6. **Python integration tests** -- bootstrap, codesign, rootfs artifact suites
7. **Import verification** -- all test suites import cleanly
8. **Schema drift check** -- regenerates settings schema and verifies no uncommitted changes

### test-install (ubuntu-24.04-arm)

Installer/update package contract tests run in Docker with systemd. This proves
the `.deb` install layout, service unit, manifest/provenance payload, and update
path stay valid before a PR can merge.

### pr-gate (ubuntu-latest)

This is the stable branch-protection status for code PRs. It depends on
`test-linux`, `test`, `test-install`, `docs-build`, and `site-build`, runs even
when one dependency fails, and fails unless every dependency job reports
`success`.

`pr-gate` is the only status that should be required by branch protection for
the product CI workflow. Individual dependency job names may change as CI is
reshaped; `pr-gate` keeps branch protection stable while still failing closed
when any required lane fails. `pr-gate` depends on `docs-build` and
`site-build` so broken docs or marketing builds cannot merge even though the
Cloudflare deploy workflows are separate.

Before claiming release readiness, run the read-only live gate checker:

```bash
uv run python scripts/check-remote-release-readiness.py
```

It verifies that the local checkout has no unpublished commits relative to
`origin/main`; remote `ci.yaml` exposes `pr-gate`, aggregates `test-linux`,
`test`, `test-install`, `docs-build`, and `site-build`, runs with
`if: ${{ always() }}` and asserts every dependency result; branch protection or
active branch rulesets require `pr-gate`; `release.capsem.org` resolves and
serves the asset channel; and the public index, `health.json`, and manifest
agree on current binary, VM asset, asset release date, generated timestamp,
profile revision, profile catalog URL, and channel manifest path. It also
resolves published host SBOM and VM OBOM evidence artifacts, verifies their
advertised hashes and sizes, and
validates attestation subjects and predicate URLs against the published evidence
lists. It verifies live `Cache-Control` headers too: mutable release-channel
pointers must stay fresh, while immutable asset and profile artifacts must keep
long-lived immutable caching. If the local checkout has unpublished commits,
publish or merge those commits before changing remote protection. It does not
push, deploy, create tags, edit rulesets, or mutate Cloudflare.

### Live release activation order

Use this order when turning the 1.4 release rails on. Do not skip ahead because
later steps depend on earlier public state being true.

1. Publish or merge the release-rail commits to `main`.
2. Wait for the expanded `pr-gate` to pass on `main`.
3. Require only `pr-gate` in branch protection or active rulesets.
4. Provision the `release.capsem.org` Cloudflare Pages project and DNS for the
   generated `target/release-channel/` artifact.
5. Run `uv run python scripts/check-remote-release-readiness.py`; continue only
   after unpublished commits, remote fail-closed `pr-gate` shape, branch
   protection, `release.capsem.org` DNS, public cache headers, and
   release-channel content all pass.
6. Run the manual VM asset workflow as a dry run and review the
   `asset-release-plan`, `asset-release-delta`, and `asset-channel-preview`
   artifacts. For metadata-only asset release changes, review
   `asset-release-delta` and `asset-channel-preview`; no `asset-release-plan`
   is expected because there are no immutable VM blobs to republish.
7. Run the tag-triggered binary release rail only from an immutable `vX.Y.Z`
   tag after confirming the tag does not already exist remotely.
8. Run the manual VM asset workflow live only after reviewing
   `asset-release-plan` when `asset_blobs_changed` is true, or reviewing the
   metadata-only delta and channel preview when only release-channel metadata
   changed; it must publish changed VM blobs, attest them, and deploy
   `release.capsem.org`.
9. Run installed update smokes for the signed macOS `.pkg`, Linux `.deb`, VM
   asset refresh, profile update path, and staged cross-surface update state.

## PR gate compared with `just test`

`just test` is still the full local/release validation command. GitHub-hosted PR
CI splits that contract across jobs and names every runner substitution instead
of pretending the hosted lane is identical.

| `just test` stage | PR CI proof | Difference |
|-------------------|-------------|------------|
| Audits, lint, frontend check/test/build | `test` job: dependency audit, Python lint/type/skills, frontend check/vitest/build | Same signal, split for GitHub summaries |
| Cross-compile agent (both arches) | `test` job: musl target check for `capsem-agent`; `test-linux` covers Linux host crates | Hosted PR substitution for Docker release cross-compile |
| Rust workspace coverage | `test` and `test-linux` jobs run `cargo llvm-cov nextest` on macOS and Linux crate sets | Same coverage rail with runner-specific package sets |
| Host binary signing prerequisites | `test` job builds and ad-hoc signs host binaries before non-VM integration suites | Same PR prerequisite for artifact-dependent Python suites |
| Python schema and no-VM integration suites | `test` job runs schema coverage plus bootstrap, codesign, and rootfs artifact suites | Same no-VM suites, scoped to generated artifacts available in CI |
| Docs and marketing builds | `docs-build` and `site-build` jobs install and build `docs/` and `site/` before `pr-gate` can pass | Merge-blocking build proof; deploy happens only after merge |
| VM-heavy Python suites (`pytest tests/ -n 4`) | Import collection only on hosted PR runners | Runner substitution: full execution remains a local/release gate until PR runners can host Apple VZ reliably |
| Serial timing, build-chain, and route-health suites | Import collection only on hosted PR runners | Runner substitution: local `just test` and release gates remain authoritative |
| Legacy injection/integration scripts and benchmark recording | Not run in hosted PR CI | Runner substitution: still required by local `just test` before release work is claimed |
| Docker cross-compile and install e2e | `test-install` runs install e2e in Docker; release workflow owns full package matrix | Split by runner capability |

## Site deploy workflows

`docs.yaml` and `site.yaml` are independent from binary and VM asset release
rails. Pull requests build docs and marketing through the `ci.yaml`
`docs-build` and `site-build` jobs, which feed the required `pr-gate`. Pushes to
`main` deploy through Cloudflare Pages and then smoke the public custom domain:

| Workflow | Public smoke |
|----------|--------------|
| `docs.yaml` | `https://docs.capsem.org/`, content type `text/html`, landing tagline, and `/getting-started/` |
| `site.yaml` | `https://capsem.org/`, content type `text/html`, landing tagline, and product hero copy |

### Coverage

Coverage is uploaded to [Codecov](https://codecov.io) with flags:

| Flag | Source | Floor |
|------|--------|-------|
| `unit` | Rust unit tests (macOS) | 70% lines |
| `linux-unit` | Rust unit tests (Linux/KVM) | 70% lines |
| `integration` | Rust integration tests | -- |
| `unit` (frontend) | vitest coverage | -- |
| `unit` (Python) | pytest coverage | 90% |

Component-level targets in `codecov.yml`:

| Component | Path owner |
|-----------|------------|
| Network | MITM, TLS, DNS/HTTP/model network parsing and routing |
| Security | policy config, host config, profile/corp security contracts |
| Tooling | MCP, builtin tools, snapshots, FS monitor |
| Monitoring | logger DB, session index, log layer |
| Virtualization | VM lifecycle and hypervisor backends |
| Runtime | in-VM agent and shared protocol crates |
| Daemon | app shell and host orchestration |
| Service | service daemon and process manager |
| Admin | profile/materialization/image administration |
| CLI | command-line client |
| TUI | terminal UI |
| MCP Server | stdio JSON-RPC MCP server |
| Gateway | TCP-to-UDS gateway and terminal WebSocket |
| System Tray | menu-bar host |
| Guard | lifecycle guard primitives |
| UI | frontend app |
| Builder | Python builder/schema package |
| Mock Server | deterministic local fixture server |

`tests/capsem-build-chain/test_coverage_infra_contract.py` is the drift guard:
adding a workspace crate must update both the PR coverage commands and the
Codecov component map.

## Release workflow (`release.yaml`)

Triggered by pushing a `vX.Y.Z` tag. Parallelized pipeline:

```
preflight (30s) --> build-app-macos (15 min) --+
                +-> test (8 min) ---------------+--> create-release
                +-> test-install ---------------+
                +-> build-app-linux ------------+
```

| Job | Runner | What it produces |
|-----|--------|-----------------|
| `preflight` | macos-14 | Validates Apple cert, Tauri signing key, notarization creds |
| `test` | macos-14 | Unit tests + coverage + audit (gates release) |
| `test-install` | ubuntu arm64 | Installer/update smoke for package payload contracts |
| `build-app-macos` | macos-14 | `.pkg` installer, notarized + stapled |
| `build-app-linux` | ubuntu arm64 + x86_64 | `.deb` packages for both arches |
| `create-release` | ubuntu | Publishes packages and host SBOM |

### Apple code signing

The macOS build signs all binaries with a Developer ID certificate:

- Certificate stored as `APPLE_CERTIFICATE` secret (base64-encoded p12)
- Must be **legacy PKCS12** format (3DES/SHA1) -- OpenSSL 3.x defaults to PBES2/AES which macOS Keychain rejects
- Notarization via `xcrun notarytool` with Apple API key

### Release artifacts

Each release publishes:
- `Capsem-{version}.pkg` -- macOS installer, codesigned, notarized, and stapled
- `Capsem_{version}_amd64.deb` and `Capsem_{version}_arm64.deb` -- Linux packages
- `capsem-sbom.spdx.json` -- host SBOM

Installers carry host binaries and the selected manifest, plus materialized
profiles and `manifest-origin.json` provenance. Heavy VM assets are downloaded
from `release.capsem.org/assets/releases/<asset-version>/` on first use through
`capsem update --assets` and verified against the manifest before boot. Tag
releases do not rebuild or upload VM assets, and they do not publish
`latest.json`; binary freshness comes from the release-channel health index.

Release packaging materializes runtime profiles through the same profile-derived build rail as
local development: `capsem-admin profile materialize` copies checked-in config
into `target/config/` and pins profile asset descriptors to the current public
asset channel manifest at
`https://release.capsem.org/assets/stable/manifest.json`. CI must not hand-edit
profiles or bypass that step.

## Asset channel workflow (`release-channel.yaml`)

`release-assets.yaml` is the manual VM asset release entrypoint. It builds the
profile-owned VM assets for both supported architectures, generates the same
`assets/manifest.json` produced by `capsem-admin manifest generate`, and builds
a channel preview. By default it runs as a dry run; live publication calls
`release-channel.yaml`.

Local release preflight has one extra release-only OBOM prerequisite beyond the
normal developer bootstrap path: `bash scripts/check-release-workflow.sh`
expects `cdxgen` in `PATH`. Install it with
`npm install -g @cyclonedx/cdxgen` before local VM asset release dry runs; the
manual asset workflow installs `@cyclonedx/cdxgen@latest` in CI before invoking
the build with `CAPSEM_CDXGEN_CMD=cdxgen`.

`release.capsem.org` is the asset channel publication surface. It is generated
from the manifest and VM blobs produced by the asset workflow. The generated
deploy root is `target/release-channel/`; the machine manifest lives at:

```text
target/release-channel/assets/stable/manifest.json
```

After deployment, clients read it as:

```text
https://release.capsem.org/assets/stable/manifest.json
```

### Release-channel Cloudflare prerequisites

Before running a live binary or VM asset channel deploy, create or verify the
Cloudflare Pages project `capsem-release`, attach the `release.capsem.org`
custom domain, and configure these GitHub Actions secrets:

| Secret | Purpose |
|--------|---------|
| `CLOUDFLARE_ACCOUNT_ID` | Cloudflare account that owns the `capsem-release` Pages project |
| `CLOUDFLARE_API_TOKEN` | API token allowed to deploy the `capsem-release` Pages project |

`release-channel.yaml` fails before deploy if either secret is missing, then
smokes `https://release.capsem.org/`, `/health.json`, and the channel manifest
through the public custom domain after Cloudflare publishes the generated site.

The release discipline is that binary releases and VM asset releases both call
the channel workflow after updating their own part of the release-channel
manifest. A tag-triggered binary release records package hashes, host SBOM, and
attestation references, then mirrors the already-published VM blobs from
`assets/releases/<asset-version>/` without rebuilding them. That generated
channel output still includes the immutable profile catalog artifact under
`profiles/releases/<revision>/catalog.json`, so profile metadata can move with
the release channel independently from VM image rebuilds. A manual VM asset
release produces the manifest, immutable blob paths, and OBOM/provenance
evidence, then publishes an immutable GitHub Release tagged
`assets-v<asset-version>` with arch-prefixed `vmlinuz`, `initrd.img`,
`rootfs.erofs`, and `obom.cdx.json` artifacts before deploying the channel.
Before comparing the asset delta or building the channel preview, the asset
workflow overlays the live channel's `binaries` metadata into the generated
asset manifest so package hashes, host SBOM references, and binary attestation
state survive VM asset releases. The first channel publication can still
bootstrap when the previous manifest is unavailable. The first channel
bootstrap may have no host binary evidence yet because the tag-triggered binary
rail has not recorded package files, host SBOM references, or host binary
attestations; once binary files are published, missing host SBOM evidence is
release-blocking.
Manual VM asset releases do not accept or publish a binary-version override;
binary release metadata is owned by the tag-triggered binary rail.
Dry runs upload `asset-release-plan` with the generated upload script so the
planned `gh release` commands can be reviewed without scraping workflow logs.
Every asset release run also uploads `asset-release-delta` with the manifest
comparison result that decided whether the channel should publish.
The delta emits both `asset_changed` and `asset_blobs_changed`: metadata-only
asset release changes, such as deprecating an older VM asset release, still
deploy the release channel without republishing immutable VM blobs. The
`asset-release-plan`, GitHub Release upload, and provenance attestation steps
run only when `asset_blobs_changed` is true.
The first channel publication may continue when the previous
`release.capsem.org/assets/<channel>/manifest.json` is unavailable; the delta
gate records `previous_manifest_unavailable` as a changed asset release so the
initial site can bootstrap. Later publications still compare against the live
previous manifest and skip deployment only when current VM blob hashes, asset release metadata, and manifest policy are all unchanged. Manifest policy includes channel-visible fields such as `refresh_policy`.
Neither rail is complete until `release.capsem.org` reflects the new channel
state. After Cloudflare deploys, `release-channel.yaml` smoke checks the public
`https://release.capsem.org/` index, `/health.json`, and
`/assets/<channel>/manifest.json` before the workflow can pass. The smoke also
rejects stale public HTML: the human index must show the same current binary,
current VM asset version, asset release date, generated timestamp, profile
revision, profile catalog URL, and channel manifest path as the fetched health
JSON and manifest. It resolves published host SBOM and VM OBOM evidence artifacts from
`health.json`, verifies their advertised hashes and sizes, and validates
attestation subjects and predicate URLs against the published evidence lists.
It also verifies public `Cache-Control` headers: mutable release-channel
pointers (`/`, `/health.json`, and `/assets/<channel>/manifest.json`) must stay
`no-cache, must-revalidate`, while immutable asset and profile release
artifacts must stay `public, max-age=31536000, immutable`.

The generated `health.json` is the compact machine-readable release-site index.
It carries schema `capsem.assets_channel.health.v1`, the active manifest URL,
the immutable asset base URL, current binary and asset versions, current asset
file download URLs, VM OBOM references, host SBOM references, binary file
metadata when present, dated asset release history in `asset_releases`
including deprecated VM asset releases, and host plus VM asset attestation
references. It also carries an explicit `updates` block with
`latest` targets for binary/assets/profile/image freshness checks so clients do
not reverse-engineer status from unrelated fields. Use it for status/provenance
checks; use `assets/<channel>/manifest.json` as the compatibility and hash
authority.

Deprecated VM asset releases stay visible in `health.json` and the human index
for auditability, but runtime resolution and `capsem update --assets` skip them
for new sessions/downloads. Existing VM pins are preserved by the VM asset-pin
rail rather than by selecting deprecated releases again.

## Running CI checks locally

Before pushing a PR, run the same checks CI will:

```bash
# Full test suite (what CI runs)
just test

# Individual components
just test-unit          # Rust unit tests
just test-frontend      # Frontend type check + vitest + build
just test-python        # Python schema tests

# Hermetic smoke test
just smoke              # doctor + integration tests
```

### Debugging CI failures

Common failure patterns:

| Symptom | Cause | Fix |
|---------|-------|-----|
| "No Developer ID signing identity" | p12 uses PBES2/AES encryption | Re-export with `scripts/fix_p12_legacy.sh` |
| KVM tests skipped | `/dev/kvm` not available on runner | Check udev rules in workflow |
| Schema drift | `config/settings/schema.generated.json` out of sync | Run `just _generate-settings` and commit |
| Frontend build fails | Missing `@source` directive | Add pattern to `global.css` |
| Coverage below floor | New code without tests | Add tests to meet 70%/80%/90% threshold |
| Python import errors | New test file with bad import | Fix the import path |
