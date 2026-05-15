version: 1.1.1778855131
---
### Added
- Added a dedicated marketing FAQ page with a hypervisor-vs-container answer
  as the first FAQ.
- Added `capsem status --json` with a typed `capsem.status.v1` health report
  for install verification and UI/test consumers.
- Added a Settings -> About debug report action that copies redacted
  version, runtime, and VM asset/initrd fingerprints for GitHub bug reports.
- Added `capsem debug` and the `capsem.debug.v1` JSON debug report so release
  bugs can include status/doctor readiness issues, setup-state, runtime, asset
  hash, host binary hash, disk-space, install-layout, process-liveness, and
  redacted log-tail evidence from the same `/debug/report` service endpoint
  used by the UI.
- Added `scripts/capture-install-status.py`, a release verification harness
  helper that captures `capsem status --json` into a structured evidence bundle
  with raw command output, parsed status JSON, metadata, version output, and a
  shallow `CAPSEM_HOME` tree snapshot. The bundle also captures optional
  `capsem debug` output and service/gateway pid, socket, and port breadcrumbs
  while redacting `gateway.token`, plus a focused installed-layout index for
  helper binaries, asset manifests, setup state, the platform service unit, and
  the macOS app bundle path. Saved VM registry and persistent-session summaries
  are captured without leaking saved VM environment variable values.
- Added a service-owned VM asset supervisor that reports `checking`,
  `updating`, `ready`, and `error` states with progress and retry detail.
- Added saved-VM base asset dependency tracking so persistent VMs can record the
  rootfs/kernel/initrd hashes, asset version, arch, and guest ABI they require.
- Added a reusable `.deb` payload verifier and wired release CI to validate
  Linux package helper binaries, signed manifests, and manifest signatures.
- Added a macOS release CI gate that requires a Developer ID Installer identity
  and runs `pkgutil --check-signature` plus Gatekeeper assessment after
  notarization and stapling.
- Added `capsem purge --product` for explicit whole-product resets that remove
  runtime files plus durable Capsem state after confirmation.
- Added an OpenTelemetry metrics handoff for the follow-up sprint, including
  the service/process IPC boundary, the live VM counter source of truth, and
  the split between JSON status surfaces and `/metrics`.

### Changed
- Changed `capsem uninstall` to remove the installed runtime while preserving
  durable user state such as config, setup state, assets, logs, session/audit
  data, and persistent VM state.
- Changed the runtime replacement proof to exercise uninstall plus fresh
  install while preserving user config, persistent VM state, and saved-VM asset
  blobs.
- Changed `capsem doctor` to preflight through the same typed health checks
  used by `capsem status` before provisioning a diagnostic VM. Status blockers
  now carry stable issue codes and severity before they are rendered.
- Changed `capsem status` to report missing or non-executable host helper
  binaries as typed health blockers.
- Changed `capsem status` to report stale `capsem-service` and
  `capsem-process` helper binary versions as typed health blockers.
- Changed `capsem status` to report stale/missing service units, asset manifest
  problems, and missing/corrupt/incomplete setup state as typed health blockers.
- Changed `capsem status` to report a missing `/Applications/Capsem.app` as a
  typed health blocker for real installed macOS runtimes.
- Changed `capsem status` to report stale `capsem-gateway` and `capsem-tray`
  helper binary versions as typed health blockers. Their `--version` paths now
  answer before runtime initialization, so status can check them safely.
- Changed `capsem status --json` to include a top-level `state` plus grouped
  `checks` for host binaries, service unit, setup, assets, app bundle, service
  endpoint, and gateway readiness.
- Changed service `/list`, gateway `/status`, and `capsem status --json` to
  preserve the service asset supervisor state instead of collapsing asset work
  into only ready/missing booleans.
- Changed the tray menu to show asset `checking`/`updating`/`error` states and
  disable New Session until VM assets are ready.
- Changed asset cleanup, saved-VM resume/fork, service `/list`, gateway
  `/status`, tray status, frontend types, and `capsem status --json` to preserve
  and report saved-VM asset dependencies. Missing saved-VM assets now surface as
  typed `saved_vm_asset_missing` status blockers without blocking new current-
  version VM creation.
- Hardened `just install` for local release reproduction: it now removes and
  verifies the old runtime while preserving durable state, installs through the
  same native package commands as `install.sh`, captures typed installed
  `capsem status --json` evidence, and fails if service, gateway, status, guest
  DNS, or guest HTTPS checks do not pass.
- Hardened the Python install-test fixture so local simulated install tests
  build the default host binaries once, then refresh installed helpers when
  they differ from `CAPSEM_BIN_SRC`, not only when missing.
- Hardened the install-status capture harness with dirty-state evidence for
  missing tray helpers and missing macOS app bundles without mutating
  `/Applications`.
- Hardened the install-status capture harness to preserve grouped status
  checks in metadata and capture saved-VM asset-reference fields when present,
  including file-state evidence for referenced asset paths.
- Added black-box simulated install coverage for reinstalling after
  `capsem uninstall` and reinstalling over a corrupted helper binary, both
  gated by `capsem status --json` runtime-layout issue codes.
- Changed service `/list` to avoid per-VM `session.db` telemetry scans on the
  hot status path. `/info` keeps the historical SQLite enrichment for now,
  while live list metrics are deferred to the OpenTelemetry sprint.
- Changed the full release gate so benchmark/doctor E2E checks run in the
  serial stage instead of racing the parallel Python shard, keeping the
  expensive VM and benchmark paths deterministic.

### Fixed
- Fixed first-run CLI auto-launch when `capsem-service` exits before binding
  its socket, so broken installed service binaries return a clear startup
  error instead of waiting through repeated socket timeouts.
- Fixed the built-in `local` MCP server toggle so
  `mcp.servers.local.enabled = false` persists, stays visible in settings, stops
  injecting or preserving the local stdio bridge in agent configs, and disables
  the runtime built-in server list entry.
- Fixed the marketing-site installer for the stamped v1.1 package assets:
  macOS now installs the downloaded `.pkg` with the native installer, and
  package downloads are checked against the release manifest when local tools
  are available.
- Fixed `capsem uninstall --yes` so it no longer recreates
  `~/.capsem/update-check.json` via the background update checker while
  uninstalling.
- Fixed repeat local installs when stale Tauri app bundles under
  `target/release/bundle/macos/` are not removable by the normal build step.
- Fixed `.deb` payload verification for zstd-compressed packages without an
  embedded content-size header, matching the published Debian package format.
- Fixed Linux KVM unit-test compilation issues surfaced by PR CI before the
  site/download installer hardening can merge.
- Fixed macOS PR CI's clean-checkout Rust unit gate by creating a minimal
  frontend dist before `capsem-app`'s Tauri test build runs.
- Fixed macOS PR CI codesigning races during `nextest` discovery by
  serializing the ad-hoc signing runner and preserving its build log on
  workflow failures.
- Fixed PR install E2E's clean-checkout host setup so missing VM assets can be
  built with `uv`, checked through pnpm-backed doctor paths, and signed with
  `minisign`.
- Fixed PR CI coverage drift by aligning the workflow's Rust coverage floor
  with the documented `just test` gate.
- Fixed clean-checkout install E2E asset alias creation by copying hash-named
  assets when Linux protected-hardlink rules reject Docker-produced files.
- Fixed PR install E2E's Docker test runner to include the project dev
  dependency group before invoking pytest inside the installed-package
  container.
- Fixed release-gate flakiness in gateway and install harness tests by making
  the mock Unix-socket gateway concurrent, restoring runtime fixtures after
  destructive uninstall/purge tests, and localizing the large-payload MITM
  upstream instead of relying on external network behavior.
- Fixed macOS PR CI's Python coverage step so it collects top-level Python
  contract tests without accidentally booting VM integration suites.
- Fixed the shared `just` execution lock on macOS hosts without a `flock`
  binary by falling back to a Python `fcntl` lock holder.
- Fixed macOS PR CI's scoped Python coverage floor so the top-level contract
  lane matches clean-runner coverage while the full `just test` gate stays at
  90%.
- Fixed macOS PR CI's no-VM Python integration lane so clean runners execute
  only suites without generated asset/signing prerequisites while still
  import-checking every integration suite.
- Fixed Linux PR CI so hosted ARM runners compile the KVM backend and test
  binaries without hanging in live KVM probes or unbounded hosted-runner test
  execution; release CI remains the real-KVM exercise gate.
- Fixed ordinary CI hardening gaps: Linux KVM diagnostics no longer emit red
  success annotations, Rust integration coverage is release-blocking, coverage
  summary errors are not hidden by `tee`, and Codecov test analytics use the
  supported uploader.
