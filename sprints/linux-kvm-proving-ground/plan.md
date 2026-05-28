# Linux KVM Proving Ground Sprint

## Goal
Prove Capsem on a live Linux KVM host by standing up a fresh Google Compute
Engine Ubuntu x86_64 instance with nested virtualization enabled and running the
hard gate:

```bash
capsem run "capsem-doctor"
```

The local starting host is Ubuntu 26.04 on GCE but has no `/dev/kvm` and no
`vmx` CPU flags, so it is evidence only for the negative preflight case. A
fresh L1 host is required for real KVM proof.

## Realms
- Targaryen: fresh GCE rebuild proof, including org-policy and VM metadata.
- Stark: asset manifest, hashes, and signatures remain the source of truth.
- Baratheon: KVM device access, modules, and service boundaries fail closed.
- Greyjoy: service/process restart and cleanup are proven on Linux.
- Tyrell: boot and first-exec timing are captured from the proving run.
- Iron Bank: CI, tracker, changelog, and release/runbook language match the
  actual coverage.

## Planned Slices

### 1. Sprint scaffolding and negative host evidence
- Create `plan.md`, `tracker.md`, and `evidence/`.
- Capture current host facts: OS, GCE metadata, VMX count, `/dev/kvm`, modules,
  Docker/service state, and KVM diagnostic result.
- Update Linux setup diagnostics only if the negative preflight shows a missing
  actionable hint.

### 2. Fresh GCE proving host
- Check `constraints/compute.disableNestedVirtualization` is not enforced.
- Create or recreate `capsem-linux-kvm` in `us-central1-a` with:

```bash
gcloud compute instances create capsem-linux-kvm \
  --enable-nested-virtualization \
  --zone=us-central1-a \
  --min-cpu-platform="Intel Haswell"
```

- Verify `vmx`, `/dev/kvm`, `kvm_intel`, `vhost_vsock`, Docker, and current
  user KVM permissions.

### 3. Source build and KVM contract proof
- Sync source to the proving host.
- Run `./bootstrap.sh --yes`.
- Run `just build-assets` if assets are absent or stale.
- Run `cargo test -p capsem-core hypervisor`.
- Run `python3 scripts/kvm-diagnostic.py`.

### 4. Live Capsem boot gate
- Run `just run "capsem-doctor"` or `target/debug/capsem run "capsem-doctor"`.
- Inspect session output and `session.db` when available.
- Record boot time and first-exec latency from existing logs/session tooling.

### 5. Fix only reproduced Linux blockers
- KVM ioctl/device model bugs in `crates/capsem-core/src/hypervisor/kvm/`.
- Linux setup/service failures in CLI setup or service code.
- Package parity failures in `.deb`, postinstall, or install tests.
- Any new diagnostic copy must be actionable and tested.

### 6. Release confidence
- Add or tighten Linux KVM proving-ground docs/runbook language if live proof
  differs from existing CI/release expectations.
- Keep hosted CI skip behavior explicit unless a runner with reliable live KVM
  is available.
- Update `CHANGELOG.md` with each functional milestone.

## Proof Matrix
- Unit/contract: `cargo test -p capsem-core hypervisor`; focused tests for any
  changed KVM/Linux setup logic.
- Functional: `python3 scripts/kvm-diagnostic.py`; `just doctor`; service
  start/list over UDS if service setup changes.
- Adversarial: missing `/dev/kvm`, inaccessible `/dev/kvm`, missing
  `vhost_vsock`, restricted nested KVM, wrong permissions, missing assets.
- E2E/VM: `just run "capsem-doctor"` on the fresh GCE host.
- Telemetry: inspect generated `session.db` after a successful doctor run.
- Performance: record boot and first-exec timing from serial/session evidence.
- Final gate: `just smoke` if feasible; otherwise record the exact blocker and
  leave the sprint open.

## Done
- Fresh GCE L1 host has nested virtualization evidence.
- Capsem builds from source on that host.
- A real KVM-backed Capsem VM runs `capsem-doctor`, or a reproduced blocker is
  isolated with evidence, tests, and a scoped fix.
- Tracker, evidence, changelog, docs, and commits match reality.
