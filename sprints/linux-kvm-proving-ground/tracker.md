# Sprint: Linux KVM Proving Ground

## Tasks
- [x] Create sprint plan, tracker, and evidence directory.
- [x] Capture negative evidence from current non-KVM GCE host.
- [x] Run `./bootstrap.sh --yes` on current host and capture blockers.
- [x] Fix Linux bootstrap blockers for missing C toolchain, stale pnpm v11
  shims, and package-level pnpm override compatibility.
- [x] Fix `doctor --fix` asset setup to build host-arch assets on first setup.
- [x] Fix offline KVM hypervisor regressions from the current host:
  vhost-vsock vring ioctl size, VirtioFS create traversal rejection, and
  namespace symlink/path handling.
- [x] Implement KVM `VmHandle` checkpoint trait surface:
  `supports_checkpoint`, `pause`, `resume`, and `save_state`.
- [x] Add KVM checkpoint format and state-machine tests.
- [x] Add adversarial KVM checkpoint tests for invalid state, bad paths,
  non-directory parents, and atomic save behavior.
- [x] Implement x86_64 KVM checkpoint restore wiring for RAM plus vCPU
  regs/sregs replay before vCPU threads start.
- [x] Add targeted vCPU thread kicks so KVM pause/stop can interrupt blocking
  `KVM_RUN` instead of waiting for incidental guest exits.
- [x] Fail closed for KVM checkpoint restore on unsupported architectures until
  their register/device state capture is implemented and proven.
- [ ] Check GCE org policy for nested virtualization.
- [ ] Provision fresh `capsem-linux-kvm` host with nested virtualization.
- [x] Verify proving-host KVM prerequisites.
- [ ] Bootstrap Capsem from source on proving host.
- [x] Run KVM contract tests and diagnostics.
- [x] Prove live x86_64 SMP boot with four visible guest CPUs.
- [x] Fix live doctor blockers for POSIX shell lookup, guest network proxies,
  and Python venv placement.
- [x] Fix live doctor blocker where Claude's atomic `.claude.json` rewrite
  disappeared after VirtioFS rename-over-existing.
- [x] Fix live doctor blocker where VirtioFS symlink reads returned
  `Function not implemented` because `READLINK` used the `GETXATTR` opcode.
- [x] Fix live doctor blocker where `uv pip install` used `/root/.cache/uv`
  on VirtioFS and failed wheel/archive cache symlinks with EINVAL.
- [x] Fix live doctor blocker where Git refused `/root` repos as dubious
  ownership because VirtioFS exposes host uid/gid while commands run as root.
- [ ] Run live boot gate: `capsem run "capsem-doctor"`.
- [ ] Inspect telemetry/session evidence after a successful boot.
- [ ] Record boot and first-exec timing.
- [x] Fix only reproduced Linux/KVM blockers found so far.
- [ ] Update Linux KVM runbook or diagnostics if evidence shows a gap.
- [x] Update `CHANGELOG.md` for the committed milestone.
- [x] Commit milestone changes.
- [ ] Final gate: `just smoke` or documented blocker.

## Notes
- Current local host is not a valid proving ground until recreated or replaced:
  `/dev/kvm` is absent and `grep -c vmx /proc/cpuinfo` returns `0`.
- GCE org-policy and instance-describe commands are blocked from this VM by
  insufficient service-account OAuth scopes, so fresh-host provisioning cannot
  proceed from the current credentials.
- KVM checkpoint trait implementation is being built as an offline-verifiable
  prerequisite. Live validation still needs the fresh nested-KVM host because
  this machine cannot execute a real KVM run loop.
- KVM checkpoint save/restore now has offline coverage on the x86_64 code path:
  checkpoints capture guest RAM plus vCPU regs/sregs and restore those before
  vCPU threads start. Pause/stop registers native vCPU threads and sends
  targeted no-op signals to interrupt blocking `KVM_RUN` calls. ARM64 KVM
  restore still fails closed pending GIC/one-reg capture. Live validation still
  needs a nested-KVM host because this machine cannot execute a real KVM run
  loop.
- `./bootstrap.sh --yes` initially failed on pnpm shell setup, then on missing
  `cc`, then on pnpm v11 ignoring package-level `pnpm.overrides`. Bootstrap now
  reaches the current-host KVM blocker.
- Installing Docker exposed that `doctor --fix` used bare `just build-assets`,
  which tried to build arm64 assets on x86_64 and failed without binfmt/qemu.
  The fix now builds the host architecture first.
- Live KVM boot now exposes all four configured vCPUs in the guest after adding
  synthetic ACPI MADT tables and guest CPUID topology. Application processors
  also stay alive across guest HLT and transient `KVM_RUN` `EAGAIN` exits.
- `capsem-doctor -x -v` now passes through the AI CLI, environment, injection,
  lifecycle, MCP S21 symlink revert, uv package install/add, and Git workflow
  blockers. The current open gate is a fresh full-suite run after the Git and
  `/tmp` symlink diagnostic updates.
- Claude's config rewrite failure was reproduced with strace: the CLI wrote a
  `.claude.json.tmp.*` file, renamed it over `.claude.json`, then subsequent
  opens used the moved temp inode's stale old host path and returned ENOENT.
  VirtioFS now updates moved inode paths and evicts overwritten target mappings
  after successful rename.
- The symlink revert failure was first visible in snapshot diagnostics, but the
  lower-level bug was VirtioFS protocol dispatch: Linux sends `READLINK` as
  opcode 5, while opcode 22 is `GETXATTR`. The wrong constant made real
  symlinks unreadable and made `ls` xattr probes report `Invalid argument`.
  The S21 doctor scenario now asserts symlink creation and restore under
  `/root`. A separate environment diagnostic covers symlink creation under
  overlay-backed `/tmp` so link-heavy caches/tools can stay off VirtioFS without
  weakening snapshot symlink coverage.
- `uv pip install wheel` then failed because uv's default cache lived under
  `/root/.cache/uv`, which is the VirtioFS workspace on Linux KVM. The guest
  boot contract now sets `UV_CACHE_DIR=/var/cache/capsem/uv`, and PID 1 creates
  that overlay-backed cache before venv/CLI use.
- Git workflow diagnostics then failed after `git init`: the repo was owned by
  the VirtioFS host uid/gid, but commands run as guest root, triggering Git's
  dubious-ownership guard. Guest PID 1 now writes `/etc/gitconfig` with
  `safe.directory=*` and `init.defaultBranch=main` inside the isolated VM.
- Guest DNS/MITM path is live: the net proxy listens on 10443/10080, the DNS
  proxy listens on UDP/TCP 1053, `getent hosts generativelanguage.googleapis.com`
  resolves, and `curl -sI https://generativelanguage.googleapis.com` reaches
  the MITM path.
- Python venv now lives at `/var/lib/capsem/venv` on the guest overlay because
  `/root` is the VirtioFS workspace on Linux KVM and produced `Invalid argument`
  executing venv interpreter links.

## Coverage Ledger
- Unit/contract: `cargo test -p capsem-core kvm` passes locally
  (267 passed, 0 failed) after the offline KVM checkpoint save/restore work.
- Unit/contract: `cargo test -p capsem-core hypervisor` passes locally
  (350 passed, 0 failed) after the broader hypervisor gate.
- Functional: `./bootstrap.sh --yes` progressed through Rust, Python, pnpm, and
  cargo tool setup; Docker works after manual install; current-host blocker is
  KVM.
- Adversarial: Current host missing-KVM negative case captured in
  `evidence/current-host-negative.txt`.
- Unit/contract: `cargo test -p capsem-core boot_x86_64 -- --nocapture`
  passed after ACPI/CPUID SMP work.
- Unit/contract: `cargo test -p capsem-core kvm_run_eagain -- --nocapture`
  passed after transient KVM_RUN handling.
- Unit/contract: `cargo test -p capsem-core hlt_exit -- --nocapture` passed
  after AP HLT handling.
- Unit/contract: `cargo test -p capsem-core rename_over_existing_rebinds_source_inode_to_target_path -- --nocapture`
  failed with ENOENT before the VirtioFS rename fix and passed after it.
- Unit/contract: `cargo test -p capsem-core virtio_fs -- --nocapture` passed
  with 55 VirtioFS tests after the rename fix.
- Unit/contract: `cargo test -p capsem-core virtio_fs -- --nocapture` passed
  with 56 VirtioFS tests after the FUSE `READLINK` opcode fix.
- Unit/contract: `cargo test -p capsem-process load_runtime_policy_state_builds_guest_boot_contract_from_v2_effective_settings -- --nocapture`
  passed after the guest venv boot-contract change.
- Unit/contract: `cargo test -p capsem-process load_runtime_policy_state_builds_guest_boot_contract_from_v2_effective_settings -- --nocapture`
  passed after adding `UV_CACHE_DIR` to the guest boot contract.
- Functional: `uv run pytest -q tests/capsem-rootfs-artifacts/test_rootfs_artifacts.py -k 'network_proxies or python_venv'`
  passed for init-script proxy and venv contracts.
- Functional: `uv run pytest -q tests/capsem-rootfs-artifacts/test_rootfs_artifacts.py -k 'python_venv or uv_cache'`
  passed for overlay-backed Python venv and uv cache contracts.
- Functional: `uv run pytest -q tests/capsem-rootfs-artifacts/test_rootfs_artifacts.py -k 'uv_cache or git_workspaces'`
  passed for uv cache and guest Git workspace contracts.
- E2E/VM: `just exec "nproc && grep -E 'processor|^siblings|^cpu cores' /proc/cpuinfo | head -20"`
  passed with `4` visible processors in the Linux KVM guest.
- E2E/VM: Live probes verified DNS/MITM proxy listeners, external DNS
  resolution, HTTPS MITM reachability, and `/var/lib/capsem/venv/bin/python3`
  execution in the guest.
- E2E/VM: Live `claude mcp list` now preserves `/root/.claude.json` and keeps
  the Capsem MCP server configured after Claude rewrites its state file.
- E2E/VM: Live relative symlink probe in `/root` now supports `ls`, `readlink`,
  and `test -e`.
- E2E/VM: `capsem-doctor -x -v -k s21_symlink_revert` passed.
- E2E/VM: `capsem-doctor -x -v -k 'tmp_symlink_support or s21_symlink_revert'`
  passed for `/tmp` symlink usability plus workspace snapshot symlink restore.
- E2E/VM: `capsem-doctor -x -v -k 'uv_pip_install_works or uv_add_package_works'`
  passed.
- E2E/VM: `capsem-doctor -x -v -k git_workflow` passed.
- E2E/VM: `capsem-doctor -x -v` remains open on
  the next full-suite blocker; do not call the live doctor gate green until a
  complete run passes.
- Telemetry: Pending successful full doctor/session inspection.
- Performance: Pending live boot.
- Missing/deferred: Full doctor gate, telemetry inspection, and performance
  timing remain pending.
