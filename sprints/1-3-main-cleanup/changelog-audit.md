# Changelog Audit: 1.3 Main Cleanup

## Verified

- Kernel 7.0 lane is present: `guest/config/build.toml` pins both guest
  architectures to `kernel_branch = "7.0"` and the builder fallback is
  `7.0.11`.
- NFT NAT lane is present: rootfs strips legacy iptables frontends and tests
  assert `iptables-nft` usage.
- Asset status is first-class: service and gateway expose asset status/ensure
  endpoints and the frontend renders missing/downloading asset state.
- `SecurityEvent.detections` is a vector and tests cover rule plus plugin
  detections on one event.
- PySigma fixture parsing exists and passes focused verification.
- Plugin endpoints have focused endpoint matrix coverage.

## Red Until Fixed

- EROFS release default is split. `just build-assets` forces `lz4hc` level `12`,
  but `guest/config/build.toml`, scaffolding, tests, and docs still advertise
  zstd level `15`.
- Setup wizard authority is removed from CLI/routes, but stale defaults and
  docs still expose or describe a setup wizard.
- The changelog says all protocol boundaries use one security-event rule
  spine, but runtime code still has Policy V2 HTTP/model/DNS/MCP decision
  rails.
- Benchmark evidence exists in sprint ledgers, but the docs benchmark results
  page is still stale and does not record the zstd rejection decision.

## Needs Final Gate

- Fresh benchmark artifacts must be generated or explicitly recorded as
  deferred before tagging.
- `just smoke` and `just test` remain release holds.
- Linux-only KVM/filesystem verification may need Monday Linux-team execution.
