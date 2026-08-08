# Sprint: Kernel 7.0 + EROFS zstd

## Tasks

- [x] Add failing tests for stable kernel branch resolution and EROFS zstd builder support.
- [x] Upgrade guest config to explicit stable kernel branch `7.0`.
- [x] Enable EROFS and EROFS zstd decompression in both kernel defconfigs.
- [x] Add EROFS image creation alongside the existing squashfs builder path.
- [x] Teach init to mount `capsem.rootfs=erofs` without breaking squashfs.
- [x] Update changelog.
- [x] Run focused tests.
- [x] Record VM/build benchmark proof and local Mac runtime speed numbers.
- [x] Add opt-in EROFS DAX probe lane for Mac/VZ and record whether it boots.
- [x] Move Linux 7.0 NAT setup to `iptables-nft`, strip legacy frontend binaries, and record VM proof.
- [x] Promote EROFS lz4hc into the normal manifest/service/release asset contract.
- [x] Run focused asset-contract tests after EROFS promotion.
- [x] Run full `just test`.
- [x] Verify a real session DB contains expected telemetry rows.
- [x] Run `just install` for local manual UI/terminal validation.
- [x] Fix macOS dev-install readiness: remove stale asset symlinks, sync local hash-named assets before app launch, and gate postinstall auto-launch on service/assets readiness.

## Coverage Ledger

- Unit/contract: `uv run pytest tests/test_docker.py -q`; `cargo test -p capsem-core cmdline -- --nocapture`.
- DAX unit/contract: `cargo test -p capsem-service process_env_allowlist_forwards_mcp_timeout_knobs -- --nocapture`;
  `cargo test -p capsem-service resolve_asset_paths_prefers_erofs_when_present -- --nocapture`;
  `cargo test -p capsem-service resolve_asset_paths_falls_back_to_squashfs -- --nocapture`.
- Functional: builder command generation is covered; live EROFS zstd creation passed on a tiny tar and full rootfs.
- Adversarial: invalid EROFS compression is rejected by `experimental_erofs_build_config`.
- E2E/VM: local Mac/arm64 guest booted kernel `7.0.11` across squashfs,
  EROFS zstd-15, and EROFS lz4hc-12 lanes. Host/container mount still fails
  with `fsconfig() failed: Operation not supported`, so Linux/KVM proof must
  run on Linux hardware. DAX probe intentionally failed with guest serial
  proof: `erofs: dax options not supported`; DAX-off lz4hc sanity boot passed.
- Performance: runtime rootfs/startup numbers are recorded in
  `benchmark-ledger.md`. EROFS lz4hc-12 is the local speed winner.
- Network: first kernel7 run exposed a real NAT-table regression. The final
  path uses `iptables-nft` only: defconfigs enable nf_tables plus
  `NFT_COMPAT`/`NETFILTER_XTABLES`/`NETFILTER_XT_TARGET_REDIRECT`, forbid the
  legacy `IP_NF_*` table path, `capsem-init` fails closed on rule append
  errors, and the rebuilt EROFS rootfs strips Debian's legacy frontend
  binaries. VM proof recorded `KERNEL=7.0.11`, all NAT REDIRECT rules,
  `legacy-absent`, DNS resolution, and HTTPS `200 OK`.
- Network performance: old HTTP/DNS/MCP/MITM load numbers in
  `benchmark-ledger.md` are historical from the temporary legacy netfix lane;
  rerun them on the final nft rootfs before using them as final benchmark
  numbers.
- Asset contract: `rootfs.erofs` is now the preferred hash-addressed manifest,
  service, setup status, release workflow, and install-download artifact.
  `rootfs.squashfs` remains only as a legacy read fallback.
- Focused asset-contract tests after promotion:
  `uv run pytest tests/test_docker.py::TestGenerateChecksums tests/test_gen_manifest.py tests/test_manifest.py -q`
  passed (`58 passed`); `cargo test -p capsem-core asset_manager -- --nocapture`
  passed (`41 passed`); `cargo test -p capsem-service resolve_asset_paths -- --nocapture`
  passed (`2 passed`); `uv run pytest tests/capsem-build-chain/test_sync_dev_assets.py -q`
  passed (`1 passed`).
- Full validation: `just test` passed on macOS after the release-script fixes.
  Key gates included frontend audit/check/test/build, Rust coverage
  (`67.14%`), Python suite (`1325 passed, 69 skipped`, coverage `91.15%`),
  build-chain serial (`21 passed`), integration proof (`40 passed`, `0 failed`,
  `3 warnings`), benchmark baseline (`1 passed`), Linux cross-compile and
  Docker/systemd install e2e (`30 passed, 34 skipped`).
- Session DB proof: integration session `whimsical-ivory-tmp` recorded
  `fs_events=21`, `net_events=15`, `dns_events=7`, `mcp_calls=610`,
  `model_calls=0`, `tool_calls=0`, `tool_responses=0`; main rollup row was
  `status=stopped`, `total_file_events=21`, `total_requests=15`,
  `total_mcp_calls=610`, `total_tool_calls=0`.
- Local install proof: `just install` built and opened
  `packages/Capsem-1.0.1780609947.pkg`; live `capsem status` reports service
  and gateway ok with assets `2026.0604.8` present under hash-prefixed EROFS
  paths (`vmlinuz-fa3b65bf6bb2b0ad`,
  `initrd-d6f86c4e39985c93.img`,
  `rootfs-b0a8616d5dd179a6.erofs`).
- Install bug fixed during closeout: a stale `~/.capsem/assets` symlink to an
  old worktree made the installed daemon report missing VM assets, and
  `postinstall` could auto-open the GUI before startup checks were ready.
  `scripts/sync-dev-assets.sh` now removes stale asset symlinks and validates
  the hash-prefixed rootfs path; `scripts/pkg-scripts/postinstall` removes
  stale asset symlinks and only auto-launches the GUI after `capsem status`
  shows service, gateway, and assets are ready. `just install` now syncs local
  dev assets before opening `Capsem.app`.
- Missing/deferred: run Linux/KVM x86_64 validation with the Linux team.
- DAX follow-up: Linux uses a separate virtio-pmem transport. Apple VZ does not
  expose virtio-pmem in the local bindings, so the Mac probe can only test
  whether the guest accepts `-o dax` on the VZ rootfs block device. Result:
  it does not; EROFS reports `dax options not supported`.
