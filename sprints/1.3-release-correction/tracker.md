# Sprint: 1.3 Release Correction

## Current Rule

No new AGY/Claude/Codex/OAuth manual run until the local due-diligence gates
below pass. Manual credentials are not the debugger.

## S0. Sprint Ledger and Release Hold

- [x] Create `sprints/1.3-release-correction/`.
- [x] Create `MASTER.md`, `plan.md`, and `tracker.md`.
- [x] Add guardrail notes to older debug/bootstrap trackers pointing here.
- [x] Snapshot branch and dirty tree before code changes.
  - Branch: `release/1.3-cleanup-pr-v2`.
  - Dirty tree already existed with code/config/test/docs/benchmark changes;
    this sprint creation added/updated sprint docs only.
- [x] Confirm no implementation starts before S0 tracker is coherent.

## S1. Profile/Config Authority

- [x] RED: test that any read/write/use of `user.toml`, `CAPSEM_USER_CONFIG`,
  `user_config_path`, or `load_settings_files` fails the contract.
- [x] GREEN: remove the legacy user config rail from service/runtime/broker/MCP
  tests/benchmarks/helpers.
- [x] RED/GREEN: prove old behavior-owned settings were not merely renamed to
  `settings.toml`; profile behavior belongs under profile files and settings
  remains UI/application preferences only.
- [ ] RED: malformed corp/settings/profile/rules/detection/MCP/plugin/assets
  files fail through the always-on admin/materialization path.
- [ ] GREEN: implement fast always-on profile/config linter in `capsem-admin`
  path, not as optional theater.
- [ ] RED/GREEN: profile/admin creation cannot emit invalid profile artifacts.
- [ ] Proof: linter covers corp, settings, profile catalog, profile files,
  rules, detection YAML, MCP config, plugins, assets, manifest, OBOM pins, and
  bootstrap root files.
  - 2026-06-11 progress: `capsem-admin profile check` now verifies copied
    workspace profiles with the same strict payload/hash/root-manifest rail as
    source profiles, rejects malformed pinned `mcp.json` even when its
    BLAKE3/size match, and rejects empty pinned package files through the same
    parser used by image workspace generation. Remaining S1 work: make
    profile catalog/corp semantic checks equally explicit before closing this
    checklist.

## S2. Materialization, Assets, VM Resources

- [ ] RED: `just _materialize-config` must materialize every checked-in profile
  and fail if `code` clobbers `co-work`.
- [ ] GREEN: `capsem-admin` materializes `code` and `co-work` with current
  `file://` EROFS/LZ4HC assets and matching BLAKE3 hashes.
- [ ] RED: package/profile tests fail if profile VM resource fields do not
  propagate to session creation.
- [ ] GREEN: new session rootfs image logical size matches
  `profile.vm.scratch_disk_size_gb`.
- [ ] RED/GREEN: doctor/status/debug report guest `df -h`, `df -i`, `/dev/vdb`,
  overlay mount options, host sparse-image logical/physical size, and host free
  space.
- [ ] RED/GREEN: bounded write/install probes cover `/usr/local`,
  `/var/cache/apt`, `/tmp`, `/var/tmp`, and `/root`.

## S3. Route Contract and API Coverage

- [ ] Inventory every UI/TUI/service route in one contract doc.
- [ ] RED: route test fails for missing profile overview/enforcement/detection
  /plugins/MCP/assets route for `code` and `co-work`.
- [ ] GREEN: implement routes with no 404/501 for declared UI/TUI surfaces.
- [ ] RED/GREEN: mutation routes either persist via profile object or do not
  exist; no fake success.
- [ ] RED/GREEN: session state enum controls available actions for running,
  stopped, incompatible, defunct, paused, and deleted sessions.
- [ ] Proof: profile routes are scoped by profile id; service-global routes are
  only service/runtime summaries.

## S4. Hermetic Protocol Lab and Recorder

- [ ] RED: integration tests fail if protocol paths hit public services.
- [ ] GREEN: one local protocol lab serves HTTP, HTTPS/MITM, DNS, SSE,
  WebSocket, MCP JSON-RPC, OAuth/OIDC, and model fixture replay.
- [ ] RED/GREEN: recorder creates sanitized fixtures with client/version,
  protocol family, auth mode, expected ledger rows, and expected visible bytes.
- [ ] RED/GREEN: replay covers Claude/Anthropic, OpenAI/Codex-compatible,
  Gemini/AGY-compatible, Ollama/OpenAI-compatible, MCP, and credential flows.
- [ ] RED/GREEN: live-local Ollama probe uses host `gemma4:latest` through the
  Capsem-routed path and records/replays the resulting native Ollama and
  OpenAI-compatible traffic without installing Ollama in the guest.
- [ ] Proof: lab is shared by doctor, integration tests, recorder, and
  benchmark.

## S5. Doctor, Just, E2E, Benchmark

- [ ] RED: `just smoke` fails if doctor is skipped or run in a reduced release
  mode.
- [ ] GREEN: remove release `--fast` escape and fold benchmark-only local
  server modes into standard `capsem-bench`.
- [ ] RED/GREEN: doctor exercises HTTP/HTTPS, gzip, chunked, SSE, WebSocket,
  DNS, MCP, model, OAuth/broker, file, process, import/export, local backend,
  snapshot route, blocked/error paths.
- [ ] RED/GREEN: doctor verifies DB ledger rows and rule/plugin evidence for
  allow/ask/block/disable/rewrite/pre/post/detection levels.
- [ ] RED/GREEN: doctor/toolchain probes cover apt/dpkg triggers, Python, pip,
  uv, Node, npm, npx, packaged CLIs, aliases, MCP bootstrap, DNS, TLS, FS
  writes.
- [ ] RED/GREEN: benchmarks use concurrency and request counts large enough to
  produce meaningful p50/p95/p99/rps for HTTP/SSE/WS/DNS/MCP/broker/model
  replay/storage/startup/lifecycle/fork.

## S6. CEL and Security Event Contract

- [ ] RED/GREEN: `ip`, `tcp`, and `udp` are first-party typed CEL facts.
- [ ] RED/GREEN: family and subobject `valid` booleans exist and are true CEL
  booleans.
- [ ] RED/GREEN: rule predicates cannot use `security.*`.
- [ ] RED/GREEN: default local/private/non-routable network rule is `ask`.
- [ ] RED/GREEN: Ollama/local backend access changes only through explicit
  profile-owned rule actions: `allow`, `ask`, `block`, `disable`.
- [ ] RED/GREEN: existing Ollama default/provider rules are audited so
  `localhost`, `127.0.0.1`, `host.docker.internal`, and `local.ollama` do not
  bypass the default local/private-network guard unless the profile's Ollama
  rule explicitly allows them.
- [ ] RED/GREEN: all security ledger rows retain event id, trace id, rule id,
  action, detection level, plugin evidence, and event payload needed for
  forensics.

## S7. Runtime Protocol Fixes

- [ ] RED/GREEN: AGY/Gemini SSE produces client-visible bytes, parsed model
  rows, and no `hyper serve error`.
- [ ] RED/GREEN: Claude/Anthropic streaming produces client-visible bytes,
  parsed model rows, and no header/EOF corruption.
- [ ] RED/GREEN: tool declarations are not counted as executed tool calls.
- [ ] RED/GREEN: executed model tool calls and MCP tools/call rows are linked
  without phantom calls.
- [ ] RED/GREEN: unknown AI-compatible protocol shape on unknown host emits
  model provider plus host and triggers detection.
- [ ] RED/GREEN: unknown remote MCP activity becomes route-visible profile
  evidence.
- [ ] RED/GREEN: credential broker logs `captured`, `brokered`, `injected`, and
  errors without raw secret leakage or generic status fields.

## S8. UI/TUI Contract Repair

- [ ] RED/GREEN: user-facing dashboard says sessions/profiles, not VMs, except
  internal/debug contexts.
- [ ] RED/GREEN: profile cards render name, description, icon, readiness, asset
  checklist, `New`, and `Customize` from route data.
- [ ] RED/GREEN: incompatible/defunct sessions are greyed and expose only valid
  actions.
- [ ] RED/GREEN: profile selection is route-backed and works with both `code`
  and `co-work`.
- [ ] RED/GREEN: enforcement/detection/plugins/MCP/assets pages load for both
  profiles with no 404/501.
- [ ] RED/GREEN: plugin/MCP/rule modes use enum-backed selects/icons and
  disabled rows are visibly disabled.
- [ ] RED/GREEN: stats detail panels show one canonical presentation and move
  raw JSON to debug-only.
- [ ] RED/GREEN: HTTP/DNS/file/process/security/credentials panels use correct
  labels, counts, syntax highlighting, and no duplicate payload fields.

## S9. Agent Bootstrap Repair

- [ ] RED/GREEN: profile root contains non-secret AGY config/wrapper and does
  not contain OAuth token/log/conversation/history/cache files.
- [ ] RED/GREEN: Claude install/bootstrap includes MCP approval and dangerous
  mode acknowledgement without first-run prompts.
- [ ] RED/GREEN: Claude binary/install path is valid or doctor reports exact
  remediation; no broken symlink in shipped profile.
- [ ] RED/GREEN: Codex config/MCP/bootstrap files are profile-owned and pinned.
- [ ] RED/GREEN: profile root manifest hashes every shipped bootstrap file.
- [ ] Proof: fresh VM can start AGY/Claude/Codex bootstrap paths without
  mutating unpinned profile state before first model request.

## S10. Packaging, Install, Docs, Release Gate

- [ ] RED/GREEN: `.pkg` and `.deb` fail if they contain rootfs/initrd/kernel
  asset blobs.
- [ ] GREEN: package accepts local/remote manifest override, copies it to the
  service-owned location, and records origin/hash in status/debug/install log.
- [ ] GREEN: install logs are timestamped and actionable.
- [ ] Proof: `just install` builds CI-like package and installs through package
  path.
- [ ] Proof: status/debug show service version, manifest origin/hash, profile
  status, plugin status, route status, doctor evidence, OBOM/SBOM references.
- [ ] Proof: changelog, docs, skills, and benchmark docs updated.
- [ ] Proof: full final gates pass and branch is pushed.

## Coverage Ledger

- Unit/contract: Pending. Must cover profile schema, route contracts, CEL
  objects, package payloads, plugin contracts, event ledgers.
- Functional: Pending. Must cover admin materialization, service routes, UI/TUI
  calls, doctor probes, broker flows.
- Adversarial: Pending. Must cover malformed configs, stale hashes, invalid
  rule roots, raw secret leak attempts, symlink escapes, bad streams, ENOSPC.
- E2E/VM/integration: Pending. Must cover fresh package install, fresh sessions,
  hermetic protocol lab, doctor, real-session DB proof.
- Telemetry/observability: Pending. Must cover structured gateway logs, plugin
  counters, security ledger, install logs, debug/status payloads.
- Performance: Pending. Must cover CEL, plugin latency, DB writer, HTTP/SSE/WS,
  DNS, MCP, broker, model replay, startup, lifecycle, fork, storage.

## Notes

- Manual evidence sessions must not be destroyed without user approval.
- S1 proof so far: `uv run python -m pytest
  tests/capsem-build-chain/test_no_legacy_user_config.py -q`; `cargo test -p
  capsem-core --lib policy_config -- --nocapture`; `cargo test -p
  capsem-core credential_broker -- --nocapture`; `cargo check -p capsem-core
  -p capsem-service -p capsem-process -p capsem-mcp-builtin`.
- S1 correction from review: any VM/profile behavior that survived as local
  settings is still debt. `settings.toml` is not a new name for `user.toml`;
  behavior must move to profile-owned artifacts or be rejected.
- S1 burn detail: runtime loaders now validate local settings and corp files
  with owner-specific contracts; credential broker no longer writes provider
  discovery into settings; no-argument profile batch-update/config-discovery
  helper symbols were removed. A broad `cargo test -p capsem-core policy_config`
  invocation tried to run the signed `mitm_integration` wrapper and failed at
  local codesign; the equivalent `--lib` policy-config proof is green.
- S1 admin/materialization proof: `cargo test -p capsem-admin -- --nocapture`
  passes after adding a failing/green check for malformed profile-owned MCP
  JSON and requiring generated image workspaces to pass `profile check` rather
  than parse-only validation.
- S1 package proof: `cargo test -p capsem-admin
  profile_check_rejects_empty_profile_package_file_even_when_hash_matches --
  --nocapture` passes; the full capsem-admin suite is now 29/29 green.
- `code-mq9ymjb2` shows apt/mandb permission and guest ENOSPC evidence.
- `code-mq9x5edq` shows AGY OAuth token reached guest disk; broker must own it.
- `code-mq9ye61s` shows Claude install/bootstrap and streaming failures.
- Host Ollama local baseline checked on 2026-06-11:
  `127.0.0.1:11434/api/tags` reports `gemma4:latest` with completion, tools,
  and thinking capabilities. This is the preferred local backend for hermetic
  model/protocol debugging, routed through Capsem.
- The current `sprints/1.3-debug-loop/current-hotlist.md` remains source
  evidence, but new implementation status belongs here.
