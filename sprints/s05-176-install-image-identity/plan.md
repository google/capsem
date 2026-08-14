# S05-176 — frozen install-image identity

## Incident

Exact qualification `ba9e862659612d8133addca3408b37381604ab76`
completed every source, asset, VM, package, timing, and functional phase. The
install source image was built while Rust coverage transiently changed source
metadata, so it received tag `efc569…`; smoke later measured the restored tree
and required `3baa217…`. The final source digest still matched `source.record`.

## Design

- Capture one immutable, Git-visible source snapshot in `source.record` and
  prove its digest equals the recorded source state.
- Require source-derived Docker builds to consume that typed snapshot; never
  default to hashing the mutable gate workspace.
- Persist a strict exact-image receipt only after the helper, source snapshot,
  tag, platform child ID, and runnable reference have all been validated.
- Make smoke and resume load and revalidate that receipt. A missing, malformed,
  stale, or moved product fails before Docker execution.
- Keep the install branch parallel with Rust coverage; isolation replaces an
  expensive ordering workaround.

## Proof matrix

- Unit: snapshot/receipt parsing, tag derivation, and exact-image validation.
- Adversarial: live-tree churn, snapshot drift during build, malformed receipt,
  moved helper/image, and missing retained product all fail closed.
- Graph: `source.record` precedes image build; the receipt is produced and is a
  carry check for same-commit resume.
- Release: a new exact committed SHA completes qualification and local glow-up.
