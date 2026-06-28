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
| `ci.yaml` | Pull requests | Full test suite: Rust unit/integration, frontend, Python, coverage |
| `release.yaml` | Tag push (`v*`) | Build apps (macOS + Linux), package with the current public asset manifest, create GitHub release |
| `release-assets.yaml` | Manual | Build VM assets, generate `assets/manifest.json`, and optionally deploy the asset channel |
| `docs.yaml` | Pull requests and push to main (docs changes) | Build docs on PRs; deploy docs.capsem.org on main, then smoke the live docs site |
| `site.yaml` | Pull requests and push to main (site changes) | Build marketing site on PRs; deploy capsem.org on main, then smoke the live marketing site |
| `release-channel.yaml` | Called by asset release | Deploy release.capsem.org from the generated asset channel site artifact |

## CI workflow (`ci.yaml`)

Runs on every pull request. Two parallel jobs:

### test-linux (ubuntu-24.04-arm)

Tests the KVM backend, which only compiles on Linux:

1. Enable `/dev/kvm` via udev rules
2. Unit tests with coverage for every portable workspace crate
3. Verify KVM tests actually ran (not silently skipped)
4. Upload coverage to Codecov with `linux-unit` flag

### test (macos-14)

Full test suite on macOS (Apple VZ backend):

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
`test-linux`, `test`, and `test-install`, runs even when one dependency fails,
and fails unless all three dependency jobs report `success`.

## Site deploy workflows

`docs.yaml` and `site.yaml` are independent from binary and VM asset release
rails. Pull requests build the changed site without deploying. Pushes to `main`
deploy through Cloudflare Pages and then smoke the public custom domain:

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

Installers carry host binaries, materialized profiles, the selected channel
manifest, and `manifest-origin.json` provenance. Heavy VM assets are downloaded
from `release.capsem.org/assets/releases/<asset-version>/` on first use through
`capsem update --assets` and verified against the manifest before boot. Tag
releases do not rebuild or upload VM assets.

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

The release discipline is that VM asset releases call the channel workflow after
producing the manifest, immutable blob paths under
`assets/releases/<asset-version>/`, and OBOM/provenance evidence. A VM asset
release is not complete until `release.capsem.org` reflects the new channel
manifest and blobs. After Cloudflare deploys, `release-channel.yaml` smoke
checks the public `https://release.capsem.org/` index,
`/health.json`, and `/assets/<channel>/manifest.json` before the workflow can
pass. Binary releases remain tag-triggered and must not rebuild VM assets by
default.

The generated `health.json` is the compact machine-readable release-site index.
It carries schema `capsem.assets_channel.health.v1`, the active manifest URL,
the immutable asset base URL, current binary and asset versions, current asset
file download URLs, VM OBOM references, host SBOM references, binary file
metadata when present, and an attestations slot. It also carries an explicit `updates` block with
`latest` targets for binary/assets/profile/image freshness checks so clients do
not reverse-engineer status from unrelated fields. Use it for status/provenance checks; use
`assets/<channel>/manifest.json` as the compatibility and hash authority.

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
