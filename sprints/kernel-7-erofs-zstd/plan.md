# Kernel 7.0 + EROFS zstd Sprint

## Goal

Upgrade the guest kernel build lane to the current stable kernel branch and add
a real EROFS zstd build path so filesystem benchmarking can compare squashfs
against modern EROFS compression instead of being stuck on the 6.6-era feature
set.

## Decisions

- Use kernel.org stable `7.0.x` for this experiment, not `7.1-rc`.
- Keep `auto` as LTS-only for conservative release automation, but allow an
  explicit stable branch such as `7.0`.
- Promote EROFS lz4hc as the canonical rootfs asset once Mac VM proof closes
  the path; keep squashfs only as a legacy read fallback.
- Emit EROFS under its real `rootfs.erofs` name, not by mislabeling an EROFS
  image as squashfs.
- Keep EROFS lz4hc-12 as the local Mac speed leader.
- Probe DAX separately. Linux's winning DAX lane uses virtio-pmem; Apple VZ
  does not expose a virtio-pmem device locally, so the Mac probe only tests
  whether `-o dax` works on the existing VZ rootfs block device.

## Files

- `guest/config/build.toml`
- `guest/config/kernel/defconfig.arm64`
- `guest/config/kernel/defconfig.x86_64`
- `guest/artifacts/capsem-init`
- `src/capsem/builder/docker.py`
- `tests/test_docker.py`
- `CHANGELOG.md`
- `crates/capsem-core/src/vm/boot.rs`
- `crates/capsem-core/src/asset_manager.rs`
- `crates/capsem-service/src/main.rs`
- `justfile`
- `.github/workflows/release.yaml`

## Done

- `kernel_branch = "7.0"` resolves to latest stable `7.0.x`.
- Kernel defconfigs enable EROFS zstd decompression.
- Builder can produce `rootfs.erofs` with `lz4`, `lz4hc`, or `zstd`.
- `just build-assets` uses EROFS lz4hc-12 for the canonical `rootfs.erofs`
  manifest asset.
- Opt-in `CAPSEM_EXPERIMENTAL_EROFS_DAX=1` appends
  `capsem.rootfs=erofs-dax` and init attempts an EROFS DAX mount.
- Focused tests prove resolver, builder command generation, and defconfig
  contract.
- Benchmark ledger records actual numbers or explicitly marks VM proof pending.
- Full `just test` passed after EROFS promotion and release packaging fixes.
- Local `just install` produced `packages/Capsem-1.0.1780609947.pkg`, installed
  the service, and verified hash-prefixed EROFS assets in `~/.capsem/assets`.
- macOS installer/dev-install now gates app launch on service/assets readiness
  and removes stale `~/.capsem/assets` symlinks before copying assets.
