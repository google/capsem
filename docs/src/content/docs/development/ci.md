---
title: CI/CD
description: How CI workflows run, what they test, and how to debug failures.
sidebar:
  order: 6
---

Capsem uses GitHub Actions for continuous integration and release automation. There are four workflows.

## Workflows

| Workflow | Trigger | What it does |
|----------|---------|-------------|
| `ci.yaml` | Pull requests | Full test suite: Rust unit/integration, frontend, Python, coverage |
| `release.yaml` | Tag push (`v*`) | Build assets, build apps (macOS + Linux), create GitHub release |
| `docs.yaml` | Push to main (docs changes) | Build and deploy docs.capsem.org |
| `site.yaml` | Push to main (site changes) | Build and deploy capsem.org |

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

Triggered by pushing a `vX.Y.Z` tag. Parallelized pipeline (~18 min wall clock):

```
preflight (30s) --> build-assets (arm64 + x86_64, 10 min) --> build-app-macos (15 min) --+
                +-> test (8 min) --------------------------------------------------------+--> create-release
                +-----------------------------------------> build-app-linux (10 min) ----+
```

| Job | Runner | What it produces |
|-----|--------|-----------------|
| `preflight` | macos-14 | Validates Apple cert, Tauri signing key, notarization creds |
| `build-assets` | ubuntu arm64 + x86_64 | vmlinuz, initrd.img, rootfs.erofs per arch |
| `test` | macos-14 | Unit tests + coverage + audit (gates release) |
| `build-app-macos` | macos-14 | `.pkg` installer, notarized + stapled |
| `build-app-linux` | ubuntu arm64 + x86_64 | `.deb` packages for both arches |
| `create-release` | ubuntu | Publishes packages, unified manifest, and arch-prefixed VM assets |

### Apple code signing

The macOS build signs all binaries with a Developer ID certificate:

- Certificate stored as `APPLE_CERTIFICATE` secret (base64-encoded p12)
- Must be **legacy PKCS12** format (3DES/SHA1) -- OpenSSL 3.x defaults to PBES2/AES which macOS Keychain rejects
- Notarization via `xcrun notarytool` with Apple API key

### Release artifacts

Each release publishes:
- `Capsem-{version}.pkg` -- macOS installer, codesigned, notarized, and stapled
- `Capsem_{version}_amd64.deb` and `Capsem_{version}_arm64.deb` -- Linux packages
- `{arch}-vmlinuz`, `{arch}-initrd.img`, `{arch}-rootfs.erofs` -- VM assets
- `{arch}-obom.cdx.json` -- per-arch rootfs OBOM
- `manifest.json` -- v2 asset/binary manifest with BLAKE3 hashes and sizes
- `capsem-sbom.spdx.json` -- host SBOM

Installers carry host binaries and the selected manifest. Heavy VM assets are
downloaded from the release on first use through `capsem update --assets` and
verified against the manifest before boot.

Release packaging materializes runtime profiles through the same profile-derived build rail as
local development: `capsem-admin profile materialize` copies checked-in config
into `target/config/` and pins profile asset descriptors to the current
`assets/manifest.json`. CI must not hand-edit profiles or bypass that step.

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
