# Snapshot Restore Plan

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
- Profile/settings schemas and fixtures exist.
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
- Tests prove terminal shell, profile selection/readiness, and session status.

## S4: Linux/KVM/EROFS/LZ4HC And Benchmarks

Goal: respect Linux-team authoritative scoped work.

Required capabilities:

- KVM/filesystem/EROFS/LZ4HC changes from Linux-team commits are restored or
  ported in scoped files.
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
