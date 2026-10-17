# Source guards and testable design

## Platform gating tests

`cargo test --test platform_gating` scans all `.rs` files under `crates/` for macOS-only and Linux-only symbols (`libc::clonefile`, `AppleVzHypervisor`, `KvmHypervisor`, `FICLONE`, etc.) and verifies they appear inside `#[cfg(target_os = "...")]` blocks. This catches ungated platform APIs before they reach CI. Run this test when adding any platform-specific code.

## Excluding something from a guard

Real trees contain things a guard should not fail on. One legal shape, in
`capsem_builder.gate.exclusions`: **exact** (the thing, not a category), **hashed**
where the subject is content, **with a stated reason** whose length the schema
checks, and **reconciled both ways** so a stale entry fails too. Never a count
-- it fails on a harmless addition and passes on a dangerous change to
something already listed. `/dev-gate` has why each wrong shape was tried.

## Testable design

Extract logic from presentation and process entrypoints into the
lowest-dependency crate that owns its domain. If logic cannot be tested without
booting a VM or launching the GUI, first separate the pure decision from its
runtime adapter; do not default unrelated shared code into `capsem-core`.
