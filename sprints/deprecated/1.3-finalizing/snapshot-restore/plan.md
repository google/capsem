# Snapshot Restore Plan

## Execution Rules

This is a restore sprint, not a merge sprint.

For each commit in `tracker.md`:

1. Inspect the diff and the tests it introduced.
2. Decide whether the capability is an exact restore, conceptual port,
   intentional burn, or Linux handoff.
3. Record that decision beside the checkbox before checking it.
4. Restore the smallest coherent capability slice.
5. Run focused tests before committing the slice.

When old code conflicts with the current design, the current design wins, but
the old behavioral guarantee must not disappear. Example: old policy pack
commands should not bring back old policy-v2 runtime, but their corpus/backtest
discipline must come back on `SecurityRuleSet`.

No fallback, no compatibility shape, no second decision engine. The restored
system should be simpler after the port, not a layer cake.

Do not change the current 1.3 security event object, plugin contract, rule
format, detection format, or plugin/rule/detection corp/profile file locations.
If a restore slice appears blocked by those contracts, stop and ask. There is no
schema migration escape hatch in this sprint.

## S0: Inventory And Classification

Goal: make the blast radius auditable before restoring code.

- Generate the deleted-file inventory from `82e7a58c^1..82e7a58c`.
- Classify each cluster:
  - `exact_restore`: same file/command should come back.
  - `conceptual_port`: behavior must come back in current architecture.
  - `intentional_burn`: old code stays gone.
  - `linux_handoff`: Linux-owned proof/run required, code still restored/ported.
- Record decisions in `tracker.md`.

## S1: Profile/Admin Command Spine

Goal: restore the profile/admin rail that makes profiles the root of assets,
corp/user personalization, and release packaging.

Required capabilities:

- Profile base files exist and are first-class release inputs.
- Profile/settings schemas and fixtures exist and match the modern 1.3
  contract, not the old profile-v2 surface verbatim.
- Profile syntax supports per-architecture asset declarations, top-level
  `refresh_policy`, and `[assets].refresh_policy`. Channel, manifest URL, and
  trust keys are catalog/manifest-owned, not self-referential profile fields.
- S1 warning: do not restore manifest signing, profile payload signing,
  minisign pubkeys, URL+pubkey catalog fetch, or `sign|verify` command
  semantics that recreate the burned signing authority rail. Admin manifest
  work may restore only non-signing validation concepts: BLAKE3 hash checks,
  asset inventory, SBOM, and build provenance.
- Profile syntax carries the modern security rule system, including default
  rules, detection levels, provider control rules, MCP, credential broker plugin
  config, and plugin-owned HTTP materialization behavior.
- Profile/corp plugin config tracks plugin policy/config only. A typed plugin
  registry owns plugin `name`, `description`, `info`, status schema, stats
  schema, capabilities, benchmark spec, semver `version`, typed execution
  `stages`, and default config so UI/API surfaces reflect plugin truth instead
  of invented labels.
- Plugin stages are explicit typed values: `pre_decision`, `post_decision`, and
  `runtime_status`. Operators must be able to see whether a plugin can mutate
  before CEL enforcement, mutate after CEL enforcement, or only report runtime
  state.
- Static `[ai.*]` provider metadata stays burned. Provider-scoped rule syntax
  may exist as one real control rule, while configured/credentialed/routed state
  is computed from runtime evidence, VM plugin runtime status, routing config,
  and security events.
- Credential state is not a profile credential API. Delete
  `/profiles/{profile_id}/credentials/*` and expose opaque credential broker
  state only through VM plugin runtime status/stats.
- VM `info` and `status` expose active plugin descriptors, versions, modes,
  stages, health, and in-memory status snapshots. These hot-path routes must
  not read `session.db`; ledger/latest routes are separate.
- HTTP gateway route exposure is explicit allowlist only. Every service route
  that is reachable over HTTP must be named in `capsem-gateway`; unknown paths,
  retired paths, and typo paths must hard 404 without contacting the UDS
  service.
- MCP profile syntax represents the real built-in `mcp.local` server
  (`/run/capsem-mcp-server` / `capsem-mcp-builtin`) with HTTP fetch and
  workspace snapshot tools. It must not model fake filesystem MCP tools or hide
  built-in server injection outside profile ownership.
- Profile parsing/validation merges old profile/admin guarantees with the new
  security-event/CEL engine. There must not be a second policy syntax or hidden
  compatibility rail.
- `capsem-admin` exposes typed profile/settings validation.
- `capsem-admin` exposes image plan/verify/workspace/build commands.
- `capsem-admin` exposes manifest check/download-check/generate/verify only for
  BLAKE3, asset inventory, SBOM, and provenance validation; no signing rail.
- Package/bootstrap tests prove `capsem-admin` is installed and runnable.
- `just` and CI call the typed admin rail instead of re-implementing it in
  shell.

Do not bring back provider onboarding or `capsem setup`.

## S2: Runtime Profile Assets And Pins

Goal: restore the runtime chain:

```text
vm.profile_id
-> load profile manifest/config
-> profile.assets selects asset release/logical assets
-> asset manifest/cache resolves hashes
-> boot uses those resolved paths
```

Required capabilities:

- Profile catalog/loader replaces `default`-only route validation.
- Default-only profile code is removed. A default profile can exist only as a
  real catalog/profile entry.
- Service status/profile routes expose the profile inventory: profile id,
  name/description/icon from profile, revision, catalog status, installed
  status, launchability, asset readiness, reconcile/download state, and errors.
- Profile routes support list/info/status/reload/reconcile/asset ensure flows
  needed by UI, TUI, CLI, and install checks.
- Profile asset management is active service behavior: download missing assets,
  verify BLAKE3 hashes, check existing assets, refresh stale or updated
  assets, surface progress/errors, and never launch a VM on missing/corrupt
  profile-selected assets.
- Per-arch profile asset declarations include URL/hash/size metadata.
- Profile-aware asset reconcile/status/ensure returns profile-specific truth.
- VM creation stores immutable profile id.
- Persistent VMs store profile revision/payload hash and base-asset pins.
- Resume/fork/save fail closed when pins are missing, corrupt, revoked, or
  mismatched.
- Service/gateway/client DTOs expose profile id/revision/status/pins.

## S3: TUI And Terminal Shell

Goal: restore terminal operation.

Required capabilities:

- `crates/capsem-tui` or its accepted replacement is back in the workspace.
- `capsem shell` launches the TUI-backed shell path.
- TUI reads profile/session/asset readiness from backend contracts.
- TUI does not invent profile names/descriptions/icons.
- TUI is functionally equivalent to the lost multi-VM control surface:
  keyboard shortcuts, multi-VM/session navigation, create/start/pause/resume/
  stop/save/fork/delete flows where supported, terminal attach/reconnect,
  profile selection, readiness/status display, and recovery from corrupt or
  stopped sessions.
- TUI status paths must preserve the previous hotpath fixes: status/readiness
  refresh must not touch the session DB on every frame.
- Tests prove terminal shell, profile selection/readiness, session status,
  lifecycle actions, shortcut behavior, and DB-hotpath regressions.

## S4: Linux/KVM/EROFS/LZ4HC And Benchmarks

Goal: respect Linux-team authoritative scoped work.

Required capabilities:

- KVM/filesystem/EROFS/LZ4HC changes from Linux-team commits are restored or
  ported in scoped files.
- Capsem boots from EROFS/LZ4HC assets on every supported architecture.
- Profile/admin asset generation emits EROFS/LZ4HC as the accepted 1.3 runtime
  format for every supported architecture.
- The same `capsem-admin`/just rail used by CI/release materializes generated
  runtime config under `target/config/`. Checked-in `config/` is source/support
  only; no hand-edited source profile may stand in for current build output.
- Modern `iptables-nft` path stays; legacy iptables paths do not return.
- Multi-arch asset proof remains.
- EROFS/LZ4HC benchmark harness and artifacts are restored.
- zstd comparison evidence is recorded as "not worth it for 1.3" with numbers
  if available.
- EROFS/LZ4HC build output is verified from the generated `target/config`
  profile asset chain, not just from benchmark artifacts or a manually patched
  checked-in profile.
- Benchmark output records the exact image format, compression, compression
  level, architecture, kernel, host OS, and command line. Numbers must be
  compared against the accepted 1.3 baseline and called out if they are
  materially worse.
- Linux-only run proof is either passed by Linux or tracked as a release
  blocker owned by Linux.

## S5: Security Corpus And Bench Gates

Goal: preserve release evidence without resurrecting old policy engines.

Required posture:

- Reject old policy-pack, detection-pack, S08C corpus, policy-context JSONL, and
  admin policy backtest commits unless a piece already exists on the current
  `SecurityRuleSet`/CEL contract.
- Keep current enforcement TOML and Sigma YAML tests that compile directly into
  `SecurityRuleSet`; do not add another pack/backtest abstraction.
- Benchmarks cover the current hot paths: rule matching, plugin dispatch,
  credential-broker substitution, runtime event classification for HTTP, DNS,
  MCP, model, file, and process, local HTTP/model fixtures, MCP brokered auth,
  DNS load, DB writer, and EROFS/storage/lifecycle gates.
- Local network/model release proof uses `capsem-mock-server`: tiny HTTP,
  1 MiB body, gzip, SSE model stream, JSON model response, denied-target,
  credential-shaped response, and WebSocket control frames.
- DNS release proof runs `capsem-bench dns-load` inside a VM; public-network DNS
  numbers are not release proof.
- Old policy-v2/domain/MCP decision rails remain burned.

## S6: Docs, Changelog, And Verification

Goal: make the release auditable.

- Update docs to describe the current profile/admin/security architecture.
- Restore command-line docs for changed admin/build/test commands.
- Update changelog with implemented behavior only.
- Run focused unit/integration tests for each restored rail.
- Run gateway explicit-route tests proving all supported profile/plugin/VM
  routes are forwarded and unknown/retired paths are not forwarded.
- Run smoke, install, UI/TUI sanity, and benchmark gates before closing.
- Boot a profile-selected VM from restored EROFS/LZ4HC assets.
- Run `capsem-doctor` inside the VM and require green output.
- Prove file snapshot create/list/restore through the accepted runtime path.
- Record EROFS/LZ4HC benchmark numbers in the benchmark docs/page; do not close
  on missing or obviously bad numbers without an owner-accepted blocker.
- Record plugin and CEL/security-engine performance counters in the benchmark
  docs/page so latency regressions can be attributed to plugins, CEL/rules,
  logging enqueue, or runtime work.
