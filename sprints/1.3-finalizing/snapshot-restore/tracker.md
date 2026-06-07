# Snapshot Restore Tracker

## S0: Inventory And Classification

- [ ] Capture `git diff --name-status 82e7a58c^1 82e7a58c` into this
  sub-sprint or a generated evidence file.
- [ ] Mark every deleted cluster as exact restore, conceptual port,
  intentional burn, or Linux handoff.
- [ ] Confirm restore work will not change the current security event object,
  plugin contract, rule format, detection format, or plugin/rule/detection
  corp/profile file locations. If blocked, stop and ask; no schema migration
  escape hatch.
- [ ] Confirm old policy-v2/domain/MCP decision rails stay burned.
- [ ] Confirm old `capsem setup` and provider onboarding wizard stay burned.
- [ ] Commit S0.

## Commit Inspection Ledger

Each checkbox means we inspected the commit and recorded one of:
`exact_restore`, `conceptual_port`, `intentional_burn`, or `linux_handoff`.
Write the decision inline after the checkbox before marking it complete, for
example:

```text
- [x] `048d7cf5 ...` decision: conceptual_port. Notes: restore
  profile-selected asset requirements, but wire them into current profile
  routes and asset manager.
```

Do not check a commit just because a later commit appears to supersede it. If it
introduced a test, contract, command, or benchmark, inspect it and either port
the guarantee or explicitly burn it.

### S1 Profile/Admin/Asset Pipeline Commits

- [ ] `9ca1bbed release: v1.2.1779658398`
- [ ] `1bdd27cb bench: record macos arm64 benchmark results`
- [ ] `89b04f87 perf: tune rootfs squashfs block size`
- [ ] `6823cf1f feat: package capsem tui binary`
- [ ] `03fcce34 fix: skip asset alias directories in install profiles`
- [ ] `b8ca8589 fix: ignore manifest aliases in install profiles`
- [ ] `6daf264a fix: point package profiles at release assets`
- [ ] `a841716f fix: sign packaged admin python extensions`
- [ ] `718981b1 docs: record admin release gate proof`
- [ ] `24c846e8 refactor: rename admin policy packs to enforcement`
- [ ] `923d603f test: add session process policy corpus`
- [ ] `63eccc3f feat: support admin model tool policy paths`
- [ ] `9944c7ba feat: expand admin policy context parity`
- [ ] `391eaece fix: compile-check policy backtests before replay`
- [ ] `b07101ed test: tighten admin policy path compile`
- [ ] `2f9b0fd0 test: expand s08c policy corpus diversity`
- [ ] `80a416be feat: add admin policy compile`
- [ ] `2db1259a test: pin s08c detection ir parity`
- [ ] `099152a4 feat: add admin policy backtest corpus`
- [ ] `7b14ccb4 feat: add admin detection backtest corpus`
- [ ] `2bedce99 feat: seed policy context rule corpus`
- [ ] `b0eecdd7 feat: add admin doctor closeout`
- [ ] `0e1e6b1b feat: add detection ir parity`
- [ ] `66141eee feat: compile detection packs`
- [ ] `d773481f feat: validate security packs`
- [ ] `7277c17b feat: generate guest image sboms`
- [ ] `3a37d704 feat: verify doctor bundle probes`
- [ ] `2d02b6e0 fix: require image inventory proof`
- [ ] `33c83bd0 feat: verify per-arch image inventories`
- [ ] `a1dab24f feat: extract image inventory from rootfs`
- [ ] `0ffb816a feat: verify image package inventory`
- [ ] `c9fd7b4b feat: require profiles for asset builds`
- [ ] `fd86e8ed feat: derive built-in profiles from guest config`
- [ ] `5b4e4274 feat: generate profile ui base profiles`
- [ ] `a02537ad feat: add profile-derived image build command`
- [ ] `31425d04 feat: materialize profile image workspaces`
- [ ] `879c9d59 test: prove packages include capsem-admin`
- [ ] `22016426 feat: add capsem-admin manifest crypto`
- [ ] `6559bf3b feat: add capsem-admin manifest generate`
- [ ] `3e5bb3cb feat: add capsem-admin manifest download check`
- [ ] `e2946acd feat: add capsem-admin manifest fast check`
- [ ] `2cc49f7a feat: add capsem-admin image verify`
- [ ] `2fb45076 feat: add capsem-admin image plan`
- [ ] `0e9442e4 test: pin admin init json toml parity`
- [ ] `53065265 test: pin profile toml json round trip`
- [ ] `c9e227c1 test: pin service settings toml json round trip`
- [ ] `839c1114 feat: add capsem-admin settings init`
- [ ] `d2834490 feat: add capsem-admin profile init`
- [ ] `be6909a0 feat: add profile section editability gates`
- [ ] `634b9730 feat: add capsem-admin profile validation`
- [ ] `810b417a test: pin service settings default parity`
- [ ] `d0c1c988 feat: wire capsem-admin settings commands`
- [ ] `d39756f3 feat: add service settings admin contract`
- [ ] `be0741e1 feat: verify admin profile payload installs`
- [ ] `25eb08d9 feat: align admin profile lifecycle gates`
- [ ] `f3fdbf0a chore: make profile manifest canonical`
- [ ] `b04cb88c feat: add pydantic profile contracts`
- [ ] `a8f712d5 feat: add profile v2 schema artifact`
- [ ] `4cdba35f refactor install asset prep into scripts`
- [ ] `d4d2bb3a fix: harden release package verification`
- [ ] `5d7e58ce fix: harden installer downloads and release package checks`
- [ ] `22096b7f fix: harden release install deb repack`

### S2 Runtime Profile Assets/Pins Commits

- [ ] `b2fb7e33 feat: export session policy contexts`
- [ ] `7a5afc9c test: prove process enforcement logs in real vm`
- [ ] `f2a6247f docs: close s07 debt ledger`
- [ ] `f5aea0fc test: gate release image boot proof`
- [ ] `dcba8776 feat: harden profile trust and policy runtime`
- [ ] `e3be977e feat: prove s08 profile-selected gateway create`
- [ ] `694aa75b feat: select profiles during vm create`
- [ ] `2a1d079d test: prove vm fork lineage`
- [ ] `204ce825 feat: schedule profile catalog reconciliation`
- [ ] `438c9642 feat: fetch profile catalogs from URL`
- [ ] `3204f27a test: prove profile asset boot flow`
- [ ] `95155405 feat: expose profile asset provenance`
- [ ] `0a87e26a test: harden profile asset reconcile races`
- [ ] `deb1b083 refactor: remove legacy asset manifest runtime`
- [ ] `d069710f feat: trigger profile asset reconcile from update`
- [ ] `2d7e1470 feat: derive profile asset retention roots`
- [ ] `911d6a67 feat: fetch signed profile payloads`
- [ ] `dd42a2d4 feat: verify profile payload signatures`
- [ ] `237d2bbc feat: materialize verified profile payloads`
- [ ] `152c7780 feat: verify installable profile payloads`
- [ ] `d50d8a13 feat: add profile catalog lifecycle gates`
- [ ] `048d7cf5 feat: drive runtime assets from profiles`
- [ ] `d759668c feat: validate profile payload schema in rust`
- [ ] `996de225 feat: add profile manifest catalog types`
- [ ] `f3578c3d release-debug-loop: finalize saved VM asset tracking and status surfaces`

### S3 TUI/Shell And Lower-Priority Debug Commits

- [ ] `0a425541 chore: merge main into tui control`
- [ ] `a476d7a7 chore: merge main into tui control branch`
- [ ] `9ca1bbed release: v1.2.1779658398`
- [ ] `32102d6d fix: purge broken persistent tui sessions`
- [ ] `2b6a2edc fix: offer tui recovery create and purge`
- [ ] `0cf0a9a0 fix: keep tui create focus pending`
- [ ] `6902dc4b fix: show full-screen tui suspend progress`
- [ ] `b50c811d fix: reconnect tui terminal after resume`
- [ ] `9b168fd5 fix: focus tui create and hide corrupt tabs`
- [ ] `860cc8ea feat: make capsem shell launch tui`
- [ ] `f3068301 fix: prompt tui service start when offline`
- [ ] `53862ec2 fix: block tui create without profiles`
- [ ] `92143119 fix: open tui new session on empty state`
- [ ] `c2fb4b77 fix: move tui help hint to session stats`
- [ ] `e3d0312f fix: polish tui controls and overlays`
- [ ] `fb98b2d1 fix: add tui fork flow`
- [ ] `f5a73773 fix: make tui create profile aware`
- [ ] `d47a889a fix: pin tui suspend hint left`
- [ ] `f60bb671 fix: surface tui suspend shortcut`
- [ ] `1299bd5c fix: render stopped tui sessions`
- [ ] `6138c0b9 fix: gate endpoint latency hot paths`
- [ ] `a21e269c fix: stabilize tui latency display`
- [ ] `161e40f4 fix: simplify tui tab colors and modal input`
- [ ] `43716abb fix: harden tui modal and resize behavior`
- [ ] `91a9cf93 fix: make tui shell controls alt-only`
- [ ] `f54d94a0 fix: stabilize tui session navigation`
- [ ] `ec0c7152 fix: use vt parser for tui terminal`
- [ ] `c93351ee fix: finish tui live terminal proof`
- [ ] `6823cf1f feat: package capsem tui binary`
- [ ] `ec473982 feat: add confirmed capsem tui service actions`
- [ ] `92a9992f feat: add capsem mcp terminal snapshot`
- [ ] `921b941f feat: add capsem tui gateway terminal shell`
- [ ] `2e79056b style: simplify capsem tui chrome`
- [ ] `c6a70081 feat: add standalone capsem tui shell`
- [ ] `1845ec83 fix: stop install harness service before error tests`
- [ ] `33684fcd fix: compile debug report disk stats on macos`
- [ ] `2322fbf2 feat: surface security health in status`
- [ ] `27e985d8 feat: expose runtime security debug health`
- [ ] `ddaf358c test: extend s08 gateway diagnostics coverage`
- [ ] `be5f902b feat(settings-profiles): add debug provenance`
- [ ] `77ec3abf feat: add structured debug report`
- [ ] `fe7a4071 fix: harden local install diagnostics`
- [ ] `9713a49e fix(setup): split install vs. onboarding flags so reinstall stops re-showing wizard`
- [ ] `0dd1d8ed test(install): self-heal layout fixture, gate intrusive auto-launch tests`
- [ ] `5c897436 fix: switch pytest to importlib mode + package-relative conftest imports`
- [ ] `ae888779 feat: wire real .pkg/.deb install paths, harden installer pipeline`
- [ ] `6c1a639e feat: capsem setup interactive wizard`

### S4 Linux/KVM/EROFS/LZ4HC/Benchmark Commits

- [ ] `0a425541 chore: merge main into tui control`
- [ ] `9ca1bbed release: v1.2.1779658398`
- [ ] `4d133bb7 bench: rerun mac benchmark after linux merge`
- [ ] `b4ba5ce6 bench: record linux wrap-up benchmark artifacts`
- [ ] `b6f9b6e2 bench: preserve artifacts before benchmark reruns`
- [ ] `8e8c4a77 bench: archive superseded benchmark artifacts`
- [ ] `05df4127 docs: add hypervisor improvement sprint`
- [ ] `56b61a22 bench: record default off io_uring results`
- [ ] `803bfbac perf: make kvm io_uring block opt in`
- [ ] `7233acf9 bench: record gated kvm io_uring results`
- [ ] `c2422adf perf: gate kvm io_uring block to writable disks`
- [ ] `a0ef66bb bench: record kvm io_uring block results`
- [ ] `7037bac3 perf: add kvm virtio block io_uring backend`
- [ ] `0bbd5397 bench: record virtio block telemetry results`
- [ ] `4ca0fb0a feat: add kvm virtio block telemetry`
- [ ] `a0f8df6b bench: record kvm event index results`
- [ ] `3b2c7390 perf: add kvm virtio block event index`
- [ ] `9d4c1f2a bench: record combined kvm block stack results`
- [ ] `ba8f260e perf: combine kvm ioeventfd block batching`
- [ ] `20bb3483 Revert "perf: route kvm block notify through ioeventfd"`
- [ ] `7e7c470c perf: route kvm block notify through ioeventfd`
- [ ] `14dc4562 Revert "perf: batch kvm block used ring updates"`
- [ ] `589494f5 perf: batch kvm block used ring updates`
- [ ] `2d56217c Revert "perf: move kvm block io off vcpu notify"`
- [ ] `8a391cb1 perf: move kvm block io off vcpu notify`
- [ ] `c4b07da8 bench: record vectored kvm block io results`
- [ ] `0dbd5099 perf: use vectored kvm block io`
- [ ] `c093f4b4 bench: include storage diagnostics in canonical run`
- [ ] `f4308f01 perf: trim kvm rootfs overlays before fork`
- [ ] `4c75cbfe bench: enforce benchmark artifact contract`
- [ ] `d5f67d78 bench: compare linux and mac artifacts`
- [ ] `968ae891 bench: archive criterion artifacts`
- [ ] `ab03714d bench: record linux benchmark artifacts`
- [ ] `d56e07ac bench: parse git status paths correctly`
- [ ] `67add8b4 bench: distinguish source dirtiness in artifacts`
- [ ] `8286bd34 bench: use project filesystem for native baseline`
- [ ] `8e4e645d bench: record host native baselines`
- [ ] `5b9ee2c2 bench: standardize benchmark recipe`
- [ ] `3d5a8745 bench: split rootfs workload diagnostics`
- [ ] `a52f7aab perf: negotiate larger virtiofs requests`
- [ ] `b9716188 perf: use positional virtiofs io`
- [ ] `31b96ebd bench: record storage tuning context`
- [ ] `d3c7d6d2 bench: profile storage iops`
- [ ] `9e996102 bench: add storage split diagnostics`
- [ ] `f4ea4037 test: harden linux benchmark artifacts`
- [ ] `d9429e1f fix: stabilize linux kvm test gate`
- [ ] `5a1397f1 fix: resume kvm guests from warm checkpoints`
- [ ] `3bf9f18f fix: expand kvm warm restore state`
- [ ] `bdedb26a fix: preserve kvm vcpu mp state in checkpoints`
- [ ] `e34817ae docs: record linux kvm doctor pass`
- [ ] `e046977e test: cover tmp symlinks in linux kvm doctor`
- [ ] `61b775a2 fix: trust git workspaces in linux kvm guests`
- [ ] `6be2d86a fix: keep uv cache off virtiofs workspace`
- [ ] `eb76d419 fix: use linux readlink opcode for virtiofs`
- [ ] `5cee8c99 fix: preserve virtiofs inode paths on rename`
- [ ] `06cc31e5 feat: checkpoint linux kvm proving ground`
- [ ] `ea1e7e6c test: align release gate with hardened cli`
- [ ] `49bcf13d test: stabilize release gate hot paths`
- [ ] `cffc9fbf chore: checkpoint remaining S5/S6 backend and artifact updates`
- [ ] `c215b6d9 fix: keep pr linux kvm tests compile-only`
- [ ] `41be412a fix: restore linux kvm test compilation`
- [ ] `92a388ef chore(bench): refresh fork/lifecycle/capsem-bench data snapshots`
- [ ] `ffef142b test(bench): add parallel VM benchmark + preserve-always tmp dir flag`
- [ ] `48104328 refactor: move inline test modules to sibling tests.rs files`
- [ ] `e7a80751 feat(tests): archive in-VM capsem-bench baseline on every just test`
- [ ] `2d94b0a9 chore(bench): record 1.0.1776445634 lifecycle and fork bench data`
- [ ] `ae888779 feat: wire real .pkg/.deb install paths, harden installer pipeline`
- [ ] `2e4a7a50 docs: update benchmark data for 0.16.1`
- [ ] `662edecc fix: cold boot 6x faster (6.2s -> 1.0s), deduplicate backoff`
- [ ] `9b110812 docs: fork benchmark data, results page, and release process updates`
- [ ] `031aafa6 feat: v0.16.1 -- KVM diagnostics, doctor rewrite, platform-specific boot errors`
- [ ] `dae43aa9 fix: optional PIT for CI KVM, boot test in cross-compile, GNU cross-linker`
- [ ] `6039e821 fix: x86_64 Linux build -- cfg-gate aarch64 boot module, cross-linker config`
- [ ] `717d03e5 feat: x86_64 KVM boot fixes, arch validation, cross-compile Docker image`
- [ ] `f68bc9fc feat: x86_64 release boot test, compile-time KVM guardrails, arch-mismatch detection`
- [ ] `db1a82c5 feat: add x86_64 KVM backend -- bzImage boot, IRQCHIP, 16550 UART, PIO bus`
- [ ] `5811282e feat: capsem-builder integration, multi-arch CI, per-arch asset layout`
- [ ] `3cb8e44a feat: hypervisor abstraction layer with Apple VZ and KVM backends`
- [ ] `525b59bf feat: async VirtioFS worker thread with irqfd interrupts`

### S5 Security Corpus/Rules/Bench Commits

- [ ] `24c846e8 refactor: rename admin policy packs to enforcement`
- [ ] `923d603f test: add session process policy corpus`
- [ ] `63eccc3f feat: support admin model tool policy paths`
- [ ] `9944c7ba feat: expand admin policy context parity`
- [ ] `391eaece fix: compile-check policy backtests before replay`
- [ ] `b07101ed test: tighten admin policy path compile`
- [ ] `2f9b0fd0 test: expand s08c policy corpus diversity`
- [ ] `80a416be feat: add admin policy compile`
- [ ] `2db1259a test: pin s08c detection ir parity`
- [ ] `099152a4 feat: add admin policy backtest corpus`
- [ ] `7b14ccb4 feat: add admin detection backtest corpus`
- [ ] `2bedce99 feat: seed policy context rule corpus`
- [ ] `0e1e6b1b feat: add detection ir parity`
- [ ] `66141eee feat: compile detection packs`
- [ ] `d773481f feat: validate security packs`

## S1: Profile/Admin Command Spine

- [ ] Restore base profile files as profile-owned release inputs.
- [x] Write canonical `config/settings.toml`, `config/profiles/code.toml`, and
  `config/corp.toml`; remove stale `config/user.toml.default`.
- [ ] Restore profile/settings schemas and fixtures updated to the modern 1.3
  profile contract.
- [ ] Restore per-architecture profile asset declarations and update/catalog
  metadata in profile syntax.
- [ ] Ensure profile syntax carries modern default rules, enforcement rules,
  detection levels, AI/provider convenience declarations, MCP, skills,
  credential broker config, and plugin config.
- [ ] Validate profile parsing compiles into the new `SecurityRuleSet`/CEL rail;
  no second policy syntax or compatibility rail.
- [ ] Restore `capsem-admin` CLI package and entry point.
- [ ] Restore profile/settings `init|schema|validate|doctor` commands.
- [ ] Restore image `plan|verify|workspace|build` commands.
- [ ] Restore manifest `check|download-check|generate|sign|verify` commands.
- [ ] Restore `scripts/build-assets.sh --profile <profile>` or equivalent
  `just build-assets profile=...` typed rail.
- [ ] Restore package/bootstrap proof that `capsem-admin` is installed and
  runnable.
- [ ] Restore CI/release calls to `capsem-admin` for profile-derived assets.
- [ ] Add tests proving raw asset builds without a profile fail closed.
- [ ] Commit S1.

## S2: Runtime Profile Assets And Pins

- [ ] Restore profile catalog/loader and remove all `default`-only profile code
  paths.
- [ ] Represent default/built-in profiles as real catalog/profile entries using
  the same loader/status/asset machinery as every other profile.
- [ ] Restore service profile inventory/status surface: profile id,
  name/description/icon, revision, catalog status, installed status,
  launchability, asset readiness, reconcile/download state, and errors.
- [ ] Restore profile list/info/status/reload/reconcile/assets-ensure routes
  needed by UI, TUI, CLI, and install checks.
- [ ] Restore profile asset download/check/refresh management in the service.
- [ ] Ensure profile asset management verifies hashes/signatures and reports
  progress/errors per profile.
- [ ] Ensure VM launch fails closed on missing/corrupt profile-selected assets.
- [ ] Restore per-arch profile asset declarations with URL/hash/signature/size.
- [ ] Restore profile-aware asset supervisor/reconcile/status/ensure.
- [ ] Ensure VM create requires and persists immutable `profile_id`.
- [ ] Restore VM profile revision/payload hash/base-asset pins.
- [ ] Make resume/fork/save fail closed on missing/corrupt/revoked/mismatched
  profile or base-asset pins.
- [ ] Expose profile id/revision/status/pins in service/gateway/client DTOs.
- [ ] Add adversarial tests for fake profiles, two profiles with different
  assets, corrupt assets, missing pins, and revoked/deprecated profiles.
- [ ] Commit S2.

## S3: TUI And Terminal Shell

- [ ] Restore `crates/capsem-tui` or accepted replacement.
- [ ] Restore workspace/package references for TUI.
- [ ] Restore `capsem shell` TUI launch path.
- [ ] Ensure TUI reads backend profile/session/asset contracts directly.
- [ ] Restore multi-VM/session navigation and keyboard shortcuts.
- [ ] Restore TUI VM manipulation flows: create, start, pause, resume, stop,
  save, fork, delete, and recovery where supported.
- [ ] Restore terminal attach/reconnect behavior.
- [ ] Restore profile selection/readiness/status display.
- [ ] Add regression coverage that status/readiness hotpaths do not query the
  session DB on every frame.
- [ ] Add tests for terminal shell launch, profile readiness display,
  multi-VM/session navigation, lifecycle actions, shortcuts, and corrupt/stopped
  session recovery.
- [ ] Commit S3.

## S4: Linux/KVM/EROFS/LZ4HC And Benchmarks

- [ ] Inventory Linux-team scoped commits/files.
- [ ] Restore/port Linux-team KVM/filesystem changes in scoped files.
- [ ] Preserve modern `iptables-nft` path; do not restore legacy path.
- [ ] Restore/verify EROFS/LZ4HC as accepted 1.3 runtime asset format on every
  supported architecture.
- [ ] Ensure profile/admin asset generation emits EROFS/LZ4HC for every
  supported architecture.
- [ ] Restore/verify multi-arch asset proof.
- [ ] Restore advanced benchmark harness/artifacts for EROFS/LZ4HC.
- [ ] Record zstd comparison evidence and decision.
- [ ] Mark Linux-only execution proof as passed or owner-accepted handoff
  blocker.
- [ ] Commit S4.

## S5: Security Corpus And Bench Gates

- [ ] Restore detection/enforcement corpus in the new rule format.
- [ ] Restore Sigma facade/import/export tests for detection rules.
- [ ] Restore pack/corpus compile and backtest commands through `capsem-admin`
  or the accepted typed admin rail.
- [ ] Restore security-event benchmarks for HTTP, DNS, MCP, model, process, and
  file events.
- [ ] Add regression tests proving old policy-v2/domain/MCP decision rails stay
  absent.
- [ ] Commit S5.

## S6: Docs, Changelog, And Verification

- [ ] Restore current-truth profile/admin command docs.
- [ ] Restore profile assets/catalog docs against the current contract.
- [ ] Restore benchmark docs/page with current 1.3 numbers.
- [ ] Update changelog.
- [ ] Run focused tests for S1-S5.
- [ ] Run smoke.
- [ ] Run install cycle.
- [ ] Run UI and TUI sanity.
- [ ] Run benchmark gate or record Linux handoff.
- [ ] Commit S6.
