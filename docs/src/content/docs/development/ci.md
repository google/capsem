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
2. Unit tests with coverage for: capsem-core, capsem-logger, capsem-proto, capsem-service, capsem, capsem-mcp
3. Verify KVM tests actually ran (not silently skipped)
4. Upload coverage to Codecov with `linux-unit` flag

### test (macos-14)

Full test suite on macOS (Apple VZ backend):

1. **Dependency audit** -- `cargo audit` + `pnpm audit`
2. **Rust unit tests with coverage** -- all 10 crates: capsem-core, capsem-agent, capsem-logger, capsem-proto, capsem-gateway, capsem-service, capsem, capsem-mcp, capsem-tray, capsem-process
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

| Component | Target |
|-----------|--------|
| capsem-service | 80% |
| capsem-mcp | 80% |
| capsem-gateway | 80% |
| capsem (CLI) | 80% |
| capsem-core | 70% |
| capsem-agent | 70% |

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
| `build-assets` | ubuntu arm64 + x86_64 | vmlinuz, initrd.img, rootfs.squashfs per arch |
| `test` | macos-14 | Unit tests + coverage + audit (gates release) |
| `build-app-macos` | macos-14 | DMG (codesigned + notarized), host binaries, latest.json |
| `build-app-linux` | ubuntu arm64 + x86_64 | deb packages (both arches), latest.json |
| `create-release` | ubuntu | Merges latest.json, signs manifest, creates GitHub release |

### Apple code signing

The macOS build signs all binaries with a Developer ID certificate:

- Certificate stored as `APPLE_CERTIFICATE` secret (base64-encoded p12)
- Must be **legacy PKCS12** format (3DES/SHA1) -- OpenSSL 3.x defaults to PBES2/AES which macOS Keychain rejects
- Notarization via `xcrun notarytool` with Apple API key

### Release artifacts

Each release publishes:
- `capsem-{version}-{arch}.dmg` -- macOS desktop app
- `capsem_{version}_{arch}.deb` -- Linux package
- `{arch}-vmlinuz`, `{arch}-initrd.img`, `{arch}-rootfs.squashfs` -- VM images
- `manifest.json` -- asset manifest with BLAKE3 hashes
- `latest.json` -- Tauri auto-updater metadata

## Running CI checks locally

Before pushing a PR, run the same checks CI will:

```bash
# Full test suite (what CI runs)
just test

# Individual components
just test-unit          # Rust unit tests
just test-frontend      # Frontend type check + vitest + build
just test-python        # Python schema tests

# Quick smoke test
just smoke              # Fast path: doctor + integration tests
```

### Debugging CI failures

Common failure patterns:

| Symptom | Cause | Fix |
|---------|-------|-----|
| "No Developer ID signing identity" | p12 uses PBES2/AES encryption | Re-export with `scripts/fix_p12_legacy.sh` |
| KVM tests skipped | `/dev/kvm` not available on runner | Check udev rules in workflow |
| Schema drift | `settings-schema.json` out of sync | Run `just schema` and commit |
| Frontend build fails | Missing `@source` directive | Add pattern to `global.css` |
| Coverage below floor | New code without tests | Add tests to meet 70%/80%/90% threshold |
| Python import errors | New test file with bad import | Fix the import path |
