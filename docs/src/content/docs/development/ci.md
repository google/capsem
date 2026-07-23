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
| `ci.yaml` | Pull requests and pushes to `main` | Quality gate: Rust unit/integration, frontend, Python contracts, install checks, explicit runner substitutions, and a post-merge main signal |
| `security-audit.yaml` | Weekly schedule or manual dispatch | Blocking RustSec and frontend dependency advisories without coupling upstream advisory publication to every PR |
| `release.yaml` | Manual `{tag, channel}` dispatch | Run one globally serialized stable or nightly release: build apps, install-test the exact packages, publish them, update only the selected channel, and run public glow-up checks |
| `release-assets.yaml` | Manual | Build profile images/config/evidence, generate `assets/manifest.json`, and optionally deploy the asset channel |
| `release-channel-staging.yaml` | Manual | Build a deterministic staging asset channel fixture, deploy it to a Cloudflare Pages preview branch, and validate the same release-channel contract without invoking `build-assets`, `build-app-macos`, or `build-app-linux` |
| `release-binary-staging.yaml` | Manual | Build a deterministic binary-channel dry-run bundle from fake host packages and the live asset manifest, then prove profile image metadata is unchanged without creating a GitHub release or deploying release.capsem.org |
| `docs.yaml` | Push to main | Deploy docs.capsem.org on each main merge, then smoke the live docs site |
| `site.yaml` | Push to main | Deploy capsem.org on each main merge, then smoke the live marketing site |
| `release-channel.yaml` | Called by binary or asset release | Deploy release.capsem.org from the generated release-channel site artifact |

Installers carry host binaries and the selected manifest URL provenance, plus
materialized profiles. They do not carry a manifest snapshot or VM image blobs.
The manual VM asset workflow publishes changed image/evidence blobs to the
immutable GitHub Release tag `assets-v<asset-version>` using arch-prefixed
artifact names. The logical manifest names stay `vmlinuz`, `initrd.img`,
`rootfs.erofs`, `obom.cdx.json`, and `software-inventory.json`; published blob
names add the architecture prefix, such as `arm64-vmlinuz`,
`arm64-initrd.img`, `arm64-rootfs.erofs`, `arm64-obom.cdx.json`, and
`arm64-software-inventory.json`. The generated release channel then records
the verified manifest URLs and hashes.

## CI workflow (`ci.yaml`)

Runs on every pull request and push to `main`. Pull requests should require the
stable `pr-gate` status before merge. New pushes cancel superseded runs only on
the same PR; main runs always finish so each merged commit has a post-merge
signal and Codecov baseline. Docs and marketing retain their independent
deployment workflows.

### test-linux (ubuntu-24.04-arm)

Tests the KVM backend, which only compiles on Linux:

1. Enable `/dev/kvm` via udev rules
2. Unit tests with coverage for every portable workspace crate
3. Verify KVM tests actually ran (not silently skipped)
4. Upload coverage to Codecov with `linux-unit` flag

### test (macos-14)

Hosted-runner quality suite on macOS:

1. **Frontend dependency audit** -- the checked-in bulk advisory client
2. **Rust unit tests with coverage** -- every workspace crate, including macOS-only app/tray crates
3. **Rust integration tests** -- cross-crate tests from `tests/` directory
4. **Frontend** -- type check (`astro check` + `svelte-check`), vitest with coverage, production build
5. **Python schema tests** -- capsem-builder tests with 90% coverage floor
6. **Python integration tests** -- bootstrap, codesign, rootfs artifact suites
7. **Import verification** -- all test suites import cleanly
8. **Schema drift check** -- regenerates settings schema and verifies no uncommitted changes

The blocking RustSec audit runs weekly and on demand in
`security-audit.yaml`. This keeps a new upstream advisory loud and owned
without making every unrelated PR red at once.

### test-install (ubuntu-24.04-arm)

Installer/update package contract tests run in Docker with systemd. This proves
the `.deb` install layout, service unit, manifest URL provenance, channel
switching, and update path stay valid before a PR can merge. The same job also
runs the local glow-up gate: it builds a real `.deb`, generates the asset
manifest with `capsem-admin manifest generate`, records package metadata with
`capsem-admin assets channel record-binary`, builds local stable/nightly release
channels with `capsem-admin assets channel build`, serves them over a local HTTP
release site, then installs, switches channels, and upgrades from that hermetic
release.

### pr-gate (ubuntu-latest)

This is the stable branch-protection status for code PRs. It depends on
`test-linux`, `test`, `test-install`, `docs-build`, `site-build`, and
`release-site-build`, runs even when one dependency fails, and fails unless every dependency job reports
`success`.

`pr-gate` is the only status that should be required by branch protection for
the product CI workflow. Individual dependency job names may change as CI is
reshaped; `pr-gate` keeps branch protection stable while still failing closed
when any required lane fails. `pr-gate` depends on `docs-build`, `site-build`,
and `release-site-build` so broken docs, marketing, or release-channel pages
cannot merge even though the Cloudflare deploy workflows are separate.

Before claiming release readiness, run the read-only live gate checker:

```bash
uv run python scripts/check-remote-release-readiness.py
```

It verifies that the local checkout has no unpublished commits relative to
`origin/main`; remote `ci.yaml` exposes `pr-gate`, aggregates `test-linux`,
`test`, `test-install`, `docs-build`, `site-build`, and `release-site-build`, runs with
`if: ${{ always() }}` and asserts every dependency result; branch protection or
active branch rulesets require `pr-gate`; and `release.capsem.org` resolves and
serves the generated release graph. The public contract is the root
`channels.json`, one selectable channel manifest URL
`/assets/<channel>/manifest.json`, package-owned binary inventory, and
profile-owned config, image, software inventory, ABOM, and OBOM records inside
that manifest. Release checks fetch profile-owned config, image, ABOM, and OBOM
files from those manifest records.

The checker verifies every channel record's `version`, `status`, manifest URL,
SHA-256, and BLAKE3; confirms exactly one selectable record is `current`
per channel; and rejects revoked records as update targets. It then validates
the selected manifest's package artifacts, package-owned per-binary inventory,
host SBOM references, and binary attestation references independently from
profile records. Profile checks validate `min_capsem_version`, config file
metadata, profile image file URLs, BLAKE3 and SHA-256 hashes, byte sizes,
software inventory, ABOM/OBOM evidence, and profile image attestation predicate
URLs from the profile-owned records. It also verifies live `Cache-Control` headers: mutable
pointers (`/`, `/channels.json`, and `/assets/<channel>/manifest.json`) must
stay fresh, while immutable profile release artifacts keep long-lived immutable
caching. If the local checkout has unpublished commits, publish or merge those
commits before changing remote protection. It does not push, deploy, create
tags, edit rulesets, or mutate Cloudflare.

### Live release activation order

Use this order when turning the 1.4 release rails on. Do not skip ahead because
later steps depend on earlier public state being true.

1. Merge the release-rail commits to `main` only after the pull request's
   expanded `pr-gate` passes.
2. Require only `pr-gate` in branch protection or active rulesets.
3. Provision the `release.capsem.org` Cloudflare Pages project and DNS for the
   generated `target/release-channel/` artifact.
4. Run `uv run python scripts/check-remote-release-readiness.py`; continue only
   after unpublished commits, remote fail-closed `pr-gate` shape, branch
   protection, `release.capsem.org` DNS, public cache headers, and
   release-channel content all pass.
5. Run `release-channel-staging.yaml` against the Cloudflare Pages staging
   branch and verify it passes the same release-channel contract without
   invoking `build-assets`, `build-app-macos`, or `build-app-linux`.
6. Run the manual profile image workflow as a dry run and review the
   `asset-release-plan`, `asset-release-delta`, and `asset-channel-preview`
   artifacts. For metadata-only asset release changes, review
   `asset-release-delta` and `asset-channel-preview`; no `asset-release-plan`
   is expected because there are no immutable profile image blobs to republish.
7. Run `release-binary-staging.yaml` and review the
   `binary-channel-dry-run-bundle` artifact. It must contain package metadata,
   `capsem-sbom.spdx.json`, `manifest.before.json`, the updated manifest,
   `record-binary.json`, `proof.json`, and the release-site preview, while
   proving profile image metadata did not change. This is the safe binary dry-run
   path.
8. Push a new immutable `vX.Y.Z` tag, then explicitly dispatch `release.yaml`
   with that tag and exactly one `stable` or `nightly` channel. The workflow is
   globally serialized so channel deployments cannot race.
9. Run the manual profile image workflow live only after reviewing
   `asset-release-plan` when `asset_blobs_changed` is true, or reviewing the
   metadata-only delta and channel preview when only release-channel metadata
   changed; it must publish changed profile image blobs, attest them, and deploy
   `release.capsem.org`.
10. Run installed update smokes for the signed macOS `.pkg`, Linux `.deb`, VM
   asset refresh, profile update path, and staged cross-surface update state.

## PR gate compared with `just test`

The Ironbank parity rule is that every portable release gate is owned by
`just test`. The complete recipe must pass locally, and exact-SHA release
qualification runs that same recipe. GitHub-hosted PR CI may split feedback
across jobs, but those jobs reuse the same checked-in entrypoints and do not
replace the canonical gate. Unavoidable runner substitutions are named below.

| `just test` stage | PR CI proof | Difference |
|-------------------|-------------|------------|
| Audits, lint, and all web surfaces | `test`, `docs-build`, `site-build`, and `release-site-build` reuse `scripts/check-web-surface.sh` | Same checked-in entrypoint; `just test` remains the canonical owner |
| Cross-compile agent (both arches) | `test` job: musl target check for `capsem-agent`; `test-linux` covers Linux host crates | Hosted PR substitution for Docker release cross-compile |
| Rust workspace coverage | `test` and `test-linux` jobs run `cargo llvm-cov nextest` on macOS and Linux crate sets | Same coverage rail with runner-specific package sets |
| Host binary signing prerequisites | `test` job builds and ad-hoc signs host binaries before non-VM integration suites | Same PR prerequisite for artifact-dependent Python suites |
| Python schema and no-VM integration suites | `test` job runs schema coverage plus bootstrap, codesign, rootfs artifact, and release-channel suites | Same no-VM suites, scoped to generated artifacts available in CI |
| Docs, marketing, and release-channel site builds | `docs-build`, `site-build`, and `release-site-build` call the same web-surface entrypoint as `just test` before `pr-gate` can pass | Merge-blocking duplicate execution of the canonical local gate; deploy happens only after merge or explicit release-channel publication |
| VM-heavy Python suites (`pytest tests/ -n 4`) | Import collection only on hosted PR runners | Runner substitution: full execution remains a local/release gate until PR runners can host Apple VZ reliably |
| Serial timing, build-chain, release-channel, and route-health suites | Import collection only on hosted PR runners | Runner substitution: local `just test` and release gates remain authoritative |
| Legacy injection/integration scripts and benchmark recording | Not run in hosted PR CI | Runner substitution: still required by local `just test` before release work is claimed |
| Docker cross-compile and install e2e | `test-install` runs install e2e in Docker; release workflow owns full package matrix | Split by runner capability |

## Site deploy workflows

`docs.yaml` and `site.yaml` are independent from binary and profile image release
rails. Pull requests build docs and marketing through the `ci.yaml`
`docs-build`, `site-build`, and `release-site-build` jobs, which feed the required `pr-gate`. Pushes to
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

Explicitly dispatched with an immutable `vX.Y.Z` tag and one `stable` or
`nightly` channel. A global concurrency group permits only one binary release
to assemble and deploy the shared release site at a time:

```
preflight --> test --> build-app-macos (build, notarize, install exact .pkg) --+
                   +-> build-app-linux (build and install exact .deb matrix) -+--> create-release --> selected-channel deploy
```

| Job | Runner | What it produces |
|-----|--------|-----------------|
| `preflight` | macos-14 | Validates Apple cert, Tauri signing key, notarization creds |
| `test` | macos-14 | Unit tests + coverage + audit (gates release) |
| `build-app-macos` | macos-14 | `.pkg` installer, notarized + stapled, then installed from that exact file |
| `build-app-linux` | ubuntu arm64 + x86_64 | `.deb` packages installed from the exact matrix outputs |
| `create-release` | ubuntu | Publishes packages and host SBOM |
| `assemble-release-channel` | ubuntu | Records package/SBOM metadata into binary channel manifests without changing profile image metadata |
| `deploy-release-channel` | ubuntu | Deploys the generated release graph through `release-channel.yaml` |

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

Installers carry host binaries, materialized profiles, the selected manifest
URL, and `manifest-metadata.json` provenance. They do not carry
`assets/manifest.json`; postinstall hydrates the live channel with
`capsem update --assets --manifest <URL>`. Heavy profile image files are
downloaded through that same path and verified against the profile-owned file
metadata before boot. Tag releases do not rebuild or upload profile images, and
they do not publish `latest.json`; binary freshness comes from the selected
manifest in the release graph.

The binary rail is parameterized by channel. Each run records package/SBOM
metadata into exactly one selected channel; nightly can move daily while stable
is promoted on the weekly cadence. It fetches both current channel manifests to
rebuild a complete release-site snapshot, but only the selected manifest is
mutated. The workflow compares channel manifests and fails if profile image
metadata changes.

After `release.capsem.org` deploys, the glow-up gate downloads the public
install script and packages, verifies package-owned binary hashes, rejects any
packaged `assets/manifest.json`, checks `manifest-metadata.json` points at the
selected channel manifest URL, then runs Docker install, stable/nightly asset
switching, and the binary updater path against the public channel.

Before deployment, `just test` owns both native install boundaries. Linux uses
`just test-install`: the gate serves generated stable and nightly release
channels from package and manifest artifacts built in Docker, then proves
`install.sh`, `capsem update --assets --manifest`, and `capsem update --yes`.
On Apple Silicon macOS, `just test-macos-install` builds the real `.pkg`, mounts
that exact file into a disposable Tart Mac, installs it, verifies the receipt,
app bundle, binary cohort, and service/gateway status. Tart macOS guests cannot
expose nested virtualization, so the recipe then extracts the same package on
the physical Mac and boots a real Capsem guest from its exact binary/profile
payload to a shell marker. Tart stays out of `just smoke` so smoke remains a
fast feedback tier rather than a partial release gate.

Release packaging materializes runtime profiles through the same profile-derived build rail as
local development: `capsem-admin profile materialize` copies checked-in config
into `target/config/` and pins profile asset descriptors to the current public
asset channel manifest at
`https://release.capsem.org/assets/<selected-channel>/manifest.json`. CI must not
hand-edit profiles or bypass that step. Channel-specific profile image changes
remain owned by the manual profile image rail.

## Asset channel workflow (`release-channel.yaml`)

`release-assets.yaml` is the manual profile image release entrypoint. It builds
the profile-owned image files for both supported architectures, generates the
same `assets/manifest.json` produced by `capsem-admin manifest generate`, and
builds a channel preview. By default it runs as a dry run; live publication
calls `release-channel.yaml`.

Local release preflight has one extra release-only OBOM prerequisite beyond the
normal developer bootstrap path: `bash scripts/check-release-workflow.sh`
expects `cdxgen` in `PATH`. Install it with
`npm install -g @cyclonedx/cdxgen@12.7.0` before local profile image release dry runs; the
manual asset workflow installs `@cyclonedx/cdxgen@12.7.0` in CI before invoking
the build with `CAPSEM_CDXGEN_CMD=cdxgen`.

`release.capsem.org` is the asset channel publication surface. It is generated
from the release graph JSON and profile image files produced by the asset
workflow. The generated deploy root is `target/release-channel/`; the machine
manifests live at:

```text
target/release-channel/assets/stable/manifest.json
target/release-channel/assets/nightly/manifest.json
target/release-channel/channels.json
```

After deployment, clients read them as:

```text
https://release.capsem.org/assets/stable/manifest.json
https://release.capsem.org/assets/nightly/manifest.json
https://release.capsem.org/channels.json
```

### Release-channel Cloudflare prerequisites

Before running a live binary or profile image channel deploy, create or verify the
Cloudflare Pages project serving `release.capsem.org`, attach the
`release.capsem.org` custom domain, and configure these GitHub Actions secrets:

| Secret | Purpose |
|--------|---------|
| `CLOUDFLARE_ACCOUNT_ID` | Cloudflare account that owns the Pages project serving `release.capsem.org` |
| `CLOUDFLARE_API_TOKEN` | API token allowed to deploy the Pages project serving `release.capsem.org` |

`release-channel.yaml` fails before deploy if either secret is missing or
`scripts/check-cloudflare-pages-project.py` cannot see the Pages project serving
`release.capsem.org` through the configured account/token. After Cloudflare
publishes the generated site, it runs `scripts/check-release-site-contract.py` against
`https://release.capsem.org`. That Python validator reuses the remote release
readiness contract, so it checks the index, `channels.json`, selected channel
manifest records, package-owned binaries, profile-owned evidence documents,
BLAKE3/SHA-256 content, attestation references, and cache headers rather than
only checking that files exist.

VM asset hashing is owned by the asset build boundary. One streaming pass
records BLAKE3 and SHA-256 in `manifest.json`; stable/nightly graph assembly
reuses that evidence and test-only graph fixtures use deterministic prepared
assets instead of the checked-in multi-gigabyte rootfs. Local-copy mode hashes
while copying once and fails closed on byte drift.

Live profile image releases run the same Cloudflare Pages project preflight before
the matrix builds start. Dry runs skip that API check, but `dry_run=false` must
prove that the configured `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN`
can see the Pages project serving `release.capsem.org` before building profile
images, publishing immutable GitHub asset blobs, or writing provenance
attestations.

The release discipline is that binary releases and profile image releases both
call the channel workflow after updating only their own part of the release
graph. A manually dispatched binary release records package artifacts, host SBOM,
host attestations, and the per-binary inventory for one channel without
touching profiles, profile images, or other channels. Every executable inside
each package must be listed with SHA-256, BLAKE3, package provenance, and
an SBOM component reference so enterprise allowlists can reason about binaries
directly. A manual profile image release updates one channel/profile entry,
profile config files, profile images, software inventory, ABOM/OBOM evidence,
and matching manifest digests without mutating package metadata, per-binary
inventory, other profiles, or other channels. Profiles may declare
`min_capsem_version` when they need newer client behavior; they do not select a
Capsem binary.

The generated release graph is append-only for auditability. `channels.json`
lists all channels and their versioned manifest records. Each manifest record
has exactly one `status` enum value: `current`, `supported`, `deprecated`, or
`revoked`. A manifest record is never marked removed; absence from the channel
list means it is no longer published. Stable and nightly are separate channels,
so updating the co-work nightly profile can leave stable, packages, other
profiles, and other channels byte-for-byte unchanged. The stable-to-nightly
acceptance gate starts on
`https://release.capsem.org/assets/stable/manifest.json`, switches to
`https://release.capsem.org/assets/nightly/manifest.json`, verifies the nightly
binary/profile graph, proves stable cached data is unchanged, and switches back
to stable.

The first channel publication can still bootstrap when the previous manifest is
unavailable. The first channel bootstrap may have no host binary evidence yet
because the binary rail has not recorded package files, host SBOM
references, or host binary attestations; once binary files are published,
missing host SBOM evidence is release-blocking. Manual profile image releases
do not accept or publish a binary-version override; binary release metadata is
owned by the parameterized binary rail.
For `dry_run=false`, the workflow first verifies that the configured Cloudflare
account/token can see the Pages project serving `release.capsem.org`, so a bad release-site
binding fails before profile image builds or immutable GitHub asset publication.
Dry runs upload `asset-release-plan` with the generated upload script so the
planned `gh release` commands can be reviewed without scraping workflow logs.
Every asset release run also uploads `asset-release-delta` with the manifest
comparison result that decided whether the channel should publish.
The delta emits both `asset_changed` and `asset_blobs_changed`: metadata-only
asset release changes, such as deprecating an older profile image release, still
deploy the release channel without republishing immutable profile image blobs. The
`asset-release-plan`, GitHub Release upload, and provenance attestation steps
run only when `asset_blobs_changed` is true.
The first channel publication may continue when the previous
`release.capsem.org/assets/<channel>/manifest.json` is unavailable; the delta
gate records `previous_manifest_unavailable` as a changed asset release so the
initial site can bootstrap. Later publications still compare against the live
previous manifest and skip deployment only when current profile image hashes,
asset release metadata, and manifest policy are all unchanged. Manifest policy
includes channel-visible fields such as `refresh_policy`.
Neither rail is complete until `release.capsem.org` reflects the new channel
state. After Cloudflare deploys, `release-channel.yaml` smoke-checks the public
`https://release.capsem.org/` index, `/channels.json`, and
`/assets/<channel>/manifest.json` before the workflow can pass, using
`scripts/check-release-site-contract.py`. The checks also
reject stale public HTML: the human index must show the same generated
timestamp, channel list, manifest URL, manifest version, package inventory,
per-binary inventory, profile revision, image artifact URLs, and evidence URLs
as the fetched `channels.json` and selected manifest. It verifies that package
file metadata and per-binary metadata
match the canonical binary metadata; that profile image file URLs,
compatibility, BLAKE3, SHA-256, and byte sizes match the profile-owned records;
and that profile config files, software inventory, ABOM/OBOM evidence, and
`min_capsem_version` are rendered from the profile JSON rather than invented by
the site. It resolves published host SBOM and VM OBOM evidence artifacts from
the graph, verifies their advertised hashes and sizes, validates their SPDX 2.3
or CycloneDX document shape, and validates attestation subjects and predicate
URLs against the published evidence lists. Profile image attestations are incomplete unless
`github_attestations_vm_assets` is present and its `predicate_url` points at the
published VM OBOM evidence for the current profile image release.
It also verifies public `Cache-Control` headers: mutable release-channel
pointers (`/`, `/channels.json`, and `/assets/<channel>/manifest.json`) must stay
`no-cache, must-revalidate`, while immutable asset and profile release
artifacts must stay `public, max-age=31536000, immutable`.

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
