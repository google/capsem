# Linux migration handoff

Exact migration head: `15723845316e23107d4110210b04fb319057567e`.

## Remaining suspected bug

Persistent VM suspend/resume is the behavior still needing end-to-end proof on
Linux/KVM. Resume must wait for the old suspended process to exit, and the KVM
checkpoint must continue proving recovery with an open VirtioFS handle. The
relevant fixes are `318d8b3d` and `5b765eb9`.

The macOS full gate run `20260830-121456-292a1c-candidate` completed Citadel,
source/release contracts, native Rust tests, and doctests without a test
failure. It was manually interrupted during `assets.preflight` for the machine
move; the recorded SIGTERM is not a product failure, and gate teardown reported
no leaked Capsem process.

```bash
git pull --ff-only origin main
just test 15723845316e23107d4110210b04fb319057567e
```

If the Linux gate exposes the suspend/resume defect, fix the behavior and keep
Citadel, Seatbelt, Ironbank, and the existing contracts intact. SDK work remains
on hold until the exact full gate is green.
