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
- Profile syntax supports per-architecture asset declarations and update/catalog
  metadata.
- Profile syntax carries the modern security rule system, including default
  rules, detection levels, AI/provider convenience declarations, MCP, skills,
  credential broker config, and plugin config.
- Profile parsing/validation merges old profile/admin guarantees with the new
  security-event/CEL engine. There must not be a second policy syntax or hidden
  compatibility rail.
- `capsem-admin` exposes typed profile/settings validation.
- `capsem-admin` exposes image plan/verify/workspace/build commands.
- `capsem-admin` exposes manifest check/download-check/generate/sign/verify.
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
- Per-arch profile asset declarations include URL/hash/signature/size metadata.
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
- Modern `iptables-nft` path stays; legacy iptables paths do not return.
- Multi-arch asset proof remains.
- EROFS/LZ4HC benchmark harness and artifacts are restored.
- zstd comparison evidence is recorded as "not worth it for 1.3" with numbers
  if available.
- Linux-only run proof is either passed by Linux or tracked as a release
  blocker owned by Linux.

## S5: Security Corpus And Bench Gates

Goal: restore release evidence without resurrecting old policy engines.

Required capabilities:

- Detection/enforcement corpus exists for the new rule format.
- Sigma facade/import/export tests exist where detection level is present.
- Backtests compile and execute against `SecurityRuleSet`.
- Benchmarks cover HTTP, DNS, MCP, model, process/file security events.
- Old policy-v2/domain/MCP decision rails remain burned.

## S6: Docs, Changelog, And Verification

Goal: make the release auditable.

- Update docs to describe the current profile/admin/security architecture.
- Restore command-line docs for changed admin/build/test commands.
- Update changelog with implemented behavior only.
- Run focused unit/integration tests for each restored rail.
- Run smoke, install, UI/TUI sanity, and benchmark gates before closing.
