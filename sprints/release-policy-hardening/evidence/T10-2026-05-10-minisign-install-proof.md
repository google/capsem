# T10 Evidence: Minisign, Package Install, and VM Command Proof

Date: 2026-05-10

## Scope

This note records focused proof gathered after the local manifest-signing gap
was found through `just exec`. It is a durable summary of the current run, not a
replacement for final Gate A screenshots, full T8 policy E2E proof, or T11
full-suite logs.

## Passing Proofs

- `bash -n bootstrap.sh scripts/doctor-common.sh scripts/doctor-macos.sh scripts/doctor-linux.sh scripts/sync-dev-assets.sh scripts/preflight.sh scripts/deb-postinst.sh scripts/pkg-scripts/postinstall`: pass.
- `uv run pytest tests/test_release_workflow_policy.py -q`: 12 passed.
- `uv run pytest tests/test_package_scripts.py -q`: 4 passed.
- `uv run pytest tests/capsem-install/test_asset_download.py -q`: 4 passed.
- `scripts/doctor-common.sh`: 42 passed, 0 skipped, 0 warnings; includes `Manifest Signing Tools: minisign` and `VM Assets: local asset manifest signature`.
- `bash scripts/sync-dev-assets.sh assets assets && minisign -Vm assets/manifest.json -x assets/manifest.json.minisig -p assets/manifest-sign.dev.pub`: pass, signature verifies with the local dev key.
- `docker build -t capsem-install-test -f docker/Dockerfile.install-test .`: pass after adding `minisign` to the install-test image.
- `just test-install`: pass with strict `.deb` install path, 33 passed and 31 skipped. The install step now uses `apt-get install -y "$DEB"` directly rather than hiding postinstall failures behind `apt-get install -f`.
- `just exec "echo cli-ok"`: pass, prints `cli-ok`.
- `just exec "capsem-doctor"`: pass, 308 passed and 4 skipped; `RESULT: PASS -- all diagnostics passed`.
- `uv run pytest tests/capsem-e2e/test_framed_mcp_mitm.py::test_framed_guest_mcp_policy_reload_blocks_existing_connection -q`: pass, 1 passed. The test now proves a live `/settings` policy update plus `/reload-config` blocks an already-open guest MCP connection and keeps denied arguments redacted in `mcp_calls.request_preview`.
- `just dev-frontend`: reached `http://localhost:5173/` after an escalated pnpm refresh. The standalone frontend surfaced a real gateway/service-unavailable state in Settings, so it was not used as completion proof by itself.
- `just ui`: launched after repacking/signing assets, starting `capsem-service`, serving Astro on `http://localhost:5173/`, building/running the Tauri app, and enabling browser verification against the connected gateway. The long-running recipe was terminated after visual proof was captured.
- Gate A Chrome visual proof under `just ui`: Settings -> Policy rendered, browser-side warning/error capture stayed empty after navigation and staging, generated policy rules rendered, a disposable `http.gate_a_block_example` rule was staged and shown as a reviewable unsaved rule, then discarded so the review list returned to zero dirty changes.
- `uv run pytest tests/capsem-rootfs-artifacts/test_rootfs_artifacts.py -q`: pass, 15 passed.
- `uv run pytest tests/capsem-build-chain/test_create_hash_assets.py tests/capsem-install/test_asset_download.py tests/capsem-install/test_installed_layout.py -q`: pass, 22 passed.
- Fresh macOS `.pkg` build/expansion proof:
  - `cargo build --release -p capsem-service -p capsem-process -p capsem -p capsem-mcp -p capsem-mcp-aggregator -p capsem-mcp-builtin -p capsem-gateway -p capsem-tray`: pass.
  - `pnpm -C frontend build`: pass.
  - `cargo tauri build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'`: pass; produced `target/release/bundle/macos/Capsem.app`.
  - `bash scripts/build-pkg.sh target/release/bundle/macos/Capsem.app target/release assets 1.1.1778445002`: pass; produced `packages/Capsem-1.1.1778445002.pkg`.
  - `pkgutil --expand-full packages/Capsem-1.1.1778445002.pkg /private/tmp/capsem-pkg-1.1.1778445002-expanded`: pass.
  - Expanded payload contains `manifest.json`, `manifest.json.minisig`, `manifest-sign.dev.pub`, and all helper binaries: `capsem`, `capsem-service`, `capsem-process`, `capsem-mcp`, `capsem-mcp-aggregator`, `capsem-mcp-builtin`, `capsem-gateway`, and `capsem-tray`.
  - `minisign -Vm /private/tmp/capsem-pkg-1.1.1778445002-expanded/capsem.pkg/Payload/usr/local/share/capsem/assets/manifest.json -x /private/tmp/capsem-pkg-1.1.1778445002-expanded/capsem.pkg/Payload/usr/local/share/capsem/assets/manifest.json.minisig -p /private/tmp/capsem-pkg-1.1.1778445002-expanded/capsem.pkg/Payload/usr/local/share/capsem/assets/manifest-sign.dev.pub`: pass, trusted comment `capsem dev key`.
- `pnpm -C frontend exec vitest run --coverage`: pass, 19 test files and 388 tests passed. This required adding the missing matching coverage provider `@vitest/coverage-v8@4.1.4`.
- `git diff --check`: pass.

## Visual Evidence

- `sprints/release-policy-hardening/evidence/T10-gate-a-policy-ui-just-ui.png`: Settings -> Policy screen under `just ui`, generated policy rules visible.
- `sprints/release-policy-hardening/evidence/T10-gate-a-policy-ui-staged-rule.png`: staged disposable policy rule visible as `staged add` with the unsaved-change footer before discard.

## Expected Local Failure

- `scripts/check-release-workflow.sh`: 11 passed, 1 failed because the local default-path `private/manifest-sign/capsem.key` is not present. `minisign` itself is installed and no longer a failed prerequisite.
- `scripts/preflight.sh`: 27 passed, 4 failed because the local default-path `private/` files are missing: `private/apple-certificate/capsem.p12`, `capsem-b64.txt`, `private/apple-certificate/capsem.p8`, and `private/manifest-sign/capsem.key`. The script dependency bug in the guest-binary check was fixed by running the import through `uv run python`. The user confirmed release secrets are available in CI, so this is a local private-dir recovery/audit issue, not a CI credential blocker.

## Remaining T10 Blockers

- Clean macOS `.pkg` install proof. The package expansion/signature proof is
  green, but running the installer is not an isolated empty-home proof here
  because the postinstall writes to `/Applications`, `/usr/local/share/capsem`,
  and the real console user's `~/.capsem`.
- Full `just test` moved to T11 and passed on 2026-05-10; see
  `sprints/release-policy-hardening/evidence/T11-2026-05-10-full-release-gate.md`.
