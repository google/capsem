# Sprint: close the Linux-clippy gap (Rust 1.95 toolchain + KVM bitrot)

## TL;DR

`just test` runs `cargo clippy --workspace --all-targets -- -D warnings`
on the host only. The host rustup channel is `stable` (currently 1.93.1
on dev macOS laptops); the Docker builder image
`capsem-host-builder:latest` pins `1.95.0`, which is also what CI's
`test-linux` job and the release pipeline use. Two problems fall out of
that skew:

1. **Clippy can't see Linux code.** Anything inside
   `#[cfg(not(target_os = "macos"))]` or `#[cfg(target_os = "linux")]`
   is never type-checked on macOS dev boxes. A genuine bug in
   `crates/capsem-process/src/vsock.rs` (unused `Result` from KVM's
   `vm.stop()`) sat undetected until Docker/CI failed the release
   pipeline. The fix was a one-liner; the detection was the problem.

2. **The KVM backend has bitrotted against rustc 1.95.** Spiking Linux
   clippy exposed ~15 hard compile errors (not lints) from the rustc
   1.93 → 1.95 jump: `E0277` (missing `Debug` derives on KVM
   `KernelLoadInfo` / `InitrdLoadInfo`), `E0596` (~14 `cannot borrow as
   mutable` sites in `kvm/virtio_blk.rs` — likely tighter binding-mode
   rules), `E0433` (`super::memory` missing), `E0599` (no method `mode`
   on `Permissions`). Plus a pile of new-in-1.95 clippy lints:
   `while_let_loop`, `needless_as_bytes`, `get_first`, `map_or`
   simplification, `is_multiple_of`, `manual_c_str_literals`,
   `collapsible_if`, `repeat_take`, `unneeded_return`,
   `mutable_borrow_from_immutable_input`.

Net effect: the KVM backend does not compile on the toolchain the
release + CI pipeline uses today, but the local `just test` never runs
against that toolchain. This sprint brings dev boxes, CI, and the
cross-compile Docker image onto the same toolchain, fixes the bitrot,
and extends `just test` Stage 1 to include a Linux clippy pass so this
class of drift fails fast next time.

## Fingerprint

Surfaced during the `explicit-shutdown-cleanup` sprint when the user
ran the release build pipeline (see `target/release` cross-compile
logs on 2026-04-23):

```
error: unused `Result` that must be used
   --> crates/capsem-process/src/vsock.rs:363:29
    |
363 | ...                   v_m.blocking_lock().stop();
    |                       ^^^^^^^^^^^^^^^^^^^^^^^^^^
    = note: `-D unused-must-use` implied by `-D warnings`
```

That single warning is the only symptom currently visible through the
release build. Running `cargo clippy --target x86_64-unknown-linux-gnu`
inside `capsem-host-builder:latest` from a 1.93 host reveals the rest:

```
error: could not compile `capsem-core` (lib) due to 12 previous errors
error: could not compile `capsem-core` (lib test) due to 20 previous errors
```

See `Appendix A` below for the full list.

## Current state

- `rust-toolchain.toml` — `channel = "stable"`, so `rustup` picks
  whatever stable happens to be installed. macOS devs drift behind.
- `capsem-host-builder:latest` Docker image — pinned to `1.95.0`. Used
  by `just cross-compile`, `just test-install`, and
  `.github/workflows/release.yaml build-app-linux`.
- `.github/workflows/ci.yaml test-linux` — `dtolnay/rust-toolchain@stable`
  on `ubuntu-24.04-arm`, so it tracks the same channel as the Docker
  image. That's the job that would catch the bitrot first.
- `just test` Stage 1 — single host-target clippy pass. Misses Linux
  cfg branches entirely.

## What the fix should look like

### Phase 1: pin the toolchain so dev + CI + Docker agree

Amend `rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.95.0"   # was "stable"
components = ["rustfmt", "clippy", "llvm-tools"]
targets = [
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
]
profile = "minimal"
```

Rationale: "stable" silently rolls dev boxes ahead of CI. Once we pin
to 1.95, the KVM bitrot surfaces locally on every `just test` instead
of quarterly during release. If 1.95 was the right target for
Dockerfile `FROM rust:1.95-*`, match it here too. Bump the two
together whenever we upgrade.

Also bump `Cargo.toml`'s `rust-version = "1.91"` to `"1.95"` so any
non-rustup consumer of the workspace (rust-analyzer, dependabot, crate
publishers) sees the real MSRV.

### Phase 2: unblock the KVM backend on 1.95

Real compile errors, not lints — these are the things we must fix
before Linux `cargo check` goes green again.

- `crates/capsem-core/src/hypervisor/kvm/boot.rs:370,431` —
  `KernelLoadInfo` and `InitrdLoadInfo` are used as `Err` values in
  `Result::unwrap`-style chains that now require `Debug`. Add
  `#[derive(Debug)]` to both (they're internal; no stability cost).
- `crates/capsem-core/src/hypervisor/kvm/virtio_mmio.rs:397:86` —
  `super::memory` path no longer resolves. Either a module was renamed
  or the `pub(crate)` hierarchy changed; trace the `fuse`/`memory`
  split in `hypervisor/mod.rs` and re-expose.
- `crates/capsem-core/src/hypervisor/kvm/virtio_blk.rs:636..908` —
  fourteen `E0596: cannot borrow h.dev as mutable` sites. Match-binding
  modes tightened in 1.95. Typically the fix is `let mut h = ...` or
  `let Ref { dev } = ...` → `let Ref { ref mut dev } = ...`. Spot
  fixes; the pattern repeats.
- `crates/capsem-core/src/hypervisor/kvm/virtio_fs/tests.rs:203` —
  `Permissions::mode()` needs `use std::os::unix::fs::PermissionsExt`
  in scope.

### Phase 3: sweep new-in-1.95 clippy lints

Entirely mechanical; clippy will autofix most of them. Track in one
commit per crate so the fix-by-fix blame is clean:

- `manual_c_str_literals` — `CString::new(b"...")` → `c"..."`.
  `kvm/sys.rs:324`, `virtio_vsock.rs:318`.
- `is_multiple_of` — `x % n == 0` → `x.is_multiple_of(n)`.
  `kvm/memory.rs:209`.
- `collapsible_if` — `virtio_mmio.rs:270`.
- `map_or` simplification — `virtio_blk.rs:106,152`.
- `repeat_take` — `virtio_fs/ops_dir.rs:65`.
- `needless_as_bytes` — `virtio_fs/mod.rs:217`.
- `get_first` — `queues.get(0)` → `.first()`. `virtio_fs/mod.rs:271`.
- `mutable_borrow_from_immutable_input` — `kvm/sys.rs:760`.
- `unneeded_return` x2 — `auto_snapshot.rs:706,709`.

`while_let_loop` in `crates/capsem-logger/src/writer.rs` already got
fixed in `dc143d5` during the spike — no further action.

### Phase 4: make `just test` Stage 1 fail on Linux regressions

Add a parallel Docker-based clippy step to the Stage 1 fast-fail block
in `justfile:test`:

```bash
(
    set -euo pipefail
    if ! docker image inspect capsem-host-builder:latest &>/dev/null; then
        just build-host-image
    fi
    HOST_ARCH=$(uname -m | sed 's/aarch64/arm64/;s/x86_64/x86_64/')
    case "$HOST_ARCH" in
        x86_64) RUST_TARGET="x86_64-unknown-linux-gnu" ;;
        arm64)  RUST_TARGET="aarch64-unknown-linux-gnu" ;;
    esac
    docker run --rm \
        -e "RUST_TARGET=$RUST_TARGET" \
        -v "$ROOT:/src" \
        -v "capsem-cargo-registry:/usr/local/cargo/registry" \
        -v "capsem-cargo-git:/usr/local/cargo/git" \
        -v "capsem-host-target-$HOST_ARCH:/cargo-target" \
        -v "capsem-rustup:/usr/local/rustup" \
        -w /src \
        capsem-host-builder:latest \
        cargo clippy --workspace --all-targets --target "$RUST_TARGET" \
            -- -D warnings
) & PID_CLIPPY_LINUX=$!
# ...
wait $PID_CLIPPY_LINUX || { echo "linux clippy failed"; FAIL=1; }
```

This must come AFTER Phase 2/3 land — otherwise Stage 1 goes red on
every run. On a warm cargo cache the Docker clippy finishes in ~45s
(roughly in line with the host clippy), so it fits Stage 1's
"cheap-parallel" budget.

Consider: also replace the Stage 2 "cross-arch agent cross-compile"
call (`uv run capsem-builder agent`) with a clippy-only invocation if
it turns out most of its value is type-checking. Out of scope for this
sprint -- note it as follow-up only.

### Phase 5: verify

- `just test` clean on macOS with cold cache.
- `just test` clean on macOS with the warm Docker clippy cache.
- `just cross-compile` still green (Docker image unchanged in content;
  only toolchain file changes).
- `.github/workflows/ci.yaml` `test-linux` still green.

## Where to start in a new session

1. **Pin `rust-toolchain.toml` to 1.95.0** and `Cargo.toml`
   `rust-version`. Commit on its own so everyone's next `rustup show`
   fetches the pin.
2. **Fix the compile errors in Phase 2 first** — until KVM compiles,
   there's no way to know whether later clippy fixes are real or
   dependent.
3. **Clippy sweep per Phase 3**, ideally one commit per file or per
   lint family so blame is legible.
4. **Wire the Docker clippy step into `just test`** only after 2 + 3
   are green locally. Anything sooner makes `just test` permanently
   red for every developer on the team.
5. **Update `skills/dev-testing/SKILL.md`** with a note that Stage 1
   now includes Linux clippy, and that bumping the Docker builder
   image's Rust version MUST be accompanied by a `rust-toolchain.toml`
   bump in the same commit.

## Scope

**In scope:**
- Pin `rust-toolchain.toml` to match `capsem-host-builder:latest`.
- Fix the ~15 Rust 1.95 compile errors in the KVM backend.
- Apply the Rust 1.95 clippy suggestions listed in Phase 3.
- Add Linux-target clippy to `just test` Stage 1.
- Document the toolchain-coupling rule.

**Out of scope:**
- Upgrading beyond 1.95 (separate cadence -- do that when the Docker
  image moves again).
- Rewriting `kvm/virtio_blk.rs` ownership beyond the minimum needed to
  satisfy 1.95's borrow rules.
- Adding a musl or an aarch64-gnu clippy pass (the host arch match is
  enough to catch the Linux-cfg regression class).
- Touching `test-linux` in `.github/workflows/ci.yaml`; that job's
  coverage rules are orthogonal.

## Non-goals

- Do **not** add `#[allow(clippy::...)]` silencers to work around lints
  without first considering the underlying fix. The whole point is
  that silencers hide the next regression.
- Do **not** demote the Stage 1 Docker clippy to Stage 2 "because it's
  a bit slower." The fast-fail property is why it's there. If warm
  Docker clippy becomes slow, profile before deferring.

## Risks

- **Docker image drift.** If someone rebuilds `capsem-host-builder`
  with a different toolchain without updating `rust-toolchain.toml`,
  we're back to the skew problem. Mitigation: note in
  `skills/build-initrd` (or wherever `build-host-image` is documented)
  that the image's Rust version must match the repo pin, and
  reference this sprint.
- **Dev machines without Docker.** The Stage 1 Docker clippy step
  requires Docker/Colima. `just test` already requires Docker for
  Stages 5 / 7 / 8, so this is not a regression. Add an early check
  that fails with a helpful message if Docker is missing before
  clippy kicks off.
- **Cold-cache runtime.** First `just test` after the sprint will
  download a fresh clippy dependency tree into the Docker volume.
  One-time cost, ~2-3 min. Document in the CHANGELOG under
  "Developer workflow".

## Appendix A: full Linux clippy dump (2026-04-23)

```
error[E0433]: cannot find `memory` in `super`
   --> crates/capsem-core/src/hypervisor/kvm/virtio_mmio.rs:397:86
error: unneeded `return` statement
   --> crates/capsem-core/src/auto_snapshot.rs:706:25
error: unneeded `return` statement
   --> crates/capsem-core/src/auto_snapshot.rs:709:17
error: manually constructing a nul-terminated string
   --> crates/capsem-core/src/hypervisor/kvm/sys.rs:324:24
error: mutable borrow from immutable input(s)
   --> crates/capsem-core/src/hypervisor/kvm/sys.rs:760:36
   --> crates/capsem-core/src/hypervisor/kvm/sys.rs:760:26
error: manual implementation of `.is_multiple_of()`
   --> crates/capsem-core/src/hypervisor/kvm/memory.rs:209:25
error: this `if` can be collapsed into the outer `match`
   --> crates/capsem-core/src/hypervisor/kvm/virtio_mmio.rs:270:17
error: this `map_or` can be simplified
   --> crates/capsem-core/src/hypervisor/kvm/virtio_blk.rs:106:12
error: this `map_or` can be simplified
   --> crates/capsem-core/src/hypervisor/kvm/virtio_blk.rs:152:12
error: manually constructing a nul-terminated string
   --> crates/capsem-core/src/hypervisor/kvm/virtio_vsock.rs:318:13
error: this `repeat().take()` can be written more concisely
  --> crates/capsem-core/src/hypervisor/kvm/virtio_fs/ops_dir.rs:65:24
error: needless call to `as_bytes`
   --> crates/capsem-core/src/hypervisor/kvm/virtio_fs/mod.rs:217:19
error: accessing first element with `queues.get(0)`
   --> crates/capsem-core/src/hypervisor/kvm/virtio_fs/mod.rs:271:34
error[E0277]: `hypervisor::kvm::boot::KernelLoadInfo` doesn't implement `std::fmt::Debug`
   --> crates/capsem-core/src/hypervisor/kvm/boot.rs:370:44
error[E0277]: `hypervisor::kvm::boot::InitrdLoadInfo` doesn't implement `std::fmt::Debug`
   --> crates/capsem-core/src/hypervisor/kvm/boot.rs:431:24
error[E0599]: no method named `mode` found for struct `std::fs::Permissions` in the current scope
   --> crates/capsem-core/src/hypervisor/kvm/virtio_fs/tests.rs:203:22
error: could not compile `capsem-core` (lib) due to 12 previous errors
error[E0596]: cannot borrow `h.dev` as mutable (x14)
   --> crates/capsem-core/src/hypervisor/kvm/virtio_blk.rs:636..908
error[E0596]: cannot borrow `dev` as mutable
   --> crates/capsem-core/src/hypervisor/kvm/virtio_blk.rs:894:9
error: could not compile `capsem-core` (lib test) due to 20 previous errors
```

Host: `rustc 1.93.1 (01f6ddf75 2026-02-11)`.
Docker: `rustc 1.95.0 (59807616e 2026-04-14)` via
`capsem-host-builder:latest` on target `aarch64-unknown-linux-gnu`.
