# Sprint tracker: hot-swappable vsock bridges

See [plan.md](./plan.md) for design, [ISSUE.md](./ISSUE.md) for history.

## Tasks

### M1 — Scaffolding (handshake helpers + EPIPE retry)

- [x] Extract `perform_handshake`, `collect_terminal_control_pair`, `is_retryable_handshake_error`
- [x] Switch handshake error wrapping from `.map_err(anyhow!)` to `.context()` so io::Error stays in chain
- [x] Wrap prologue in `handshake_with_retry` (`HANDSHAKE_RETRY_MAX = 3`)
- [x] 7 new unit tests green (15 total in `vsock::tests`)
- [x] `cargo check -p capsem-process` clean
- [x] `cargo clippy -p capsem-process` clean
- [x] Migrate inline `mod tests { ... }` into sibling `tests.rs` (CLAUDE.md convention)
- [x] CHANGELOG entry under `Unreleased / Fixed`
- [x] Commit: `fix(process): extract handshake helpers with narrow EPIPE retry`

### M2 — Continuous accept + rekey channels

- [x] Design `control_rekey_tx` / `terminal_rekey_tx` channel types (`std::sync::mpsc::Sender<RawFd>`? `tokio::sync::mpsc::UnboundedSender<VsockConnection>`? — pick one and justify)
- [x] Replace `handshake_with_retry` call with long-lived accept task
- [x] First successful handshake still gates `.ready` sentinel (unchanged semantics)
- [x] Subsequent successful handshakes push new fds through rekey channels
- [x] `deferred_conns` logic retained for auxiliary ports
- [x] Unit test: accept task emits rekey event after simulated reset + fresh handshake
- [x] CHANGELOG + commit: `feat(process): continuous vsock accept with rekey channels`

### M3 — Rekeyable control writer

- [x] Writer thread holds `Option<File>`; on write error, drop and wait on rekey recv
- [x] Heartbeat, command handler, serialize-single-writer invariant unchanged
- [x] Unit test: writer survives simulated EPIPE, picks up new fd, emits next message
- [x] Verify invariant #1 (heartbeat) and #2 (TerminalResize) still fire after rekey
- [x] CHANGELOG + commit: `feat(process): rekeyable control writer`

### M4 — Rekeyable control reader with buffer reset

- [x] Reader loop resets `len_buf` + payload `Vec` on rekey
- [x] Reader only fires `JobStore::fail_all` when rekey channel closes, not on transient fd error
- [x] Unit test: reader handles partial-frame-then-reset followed by fresh full frame without decode error
- [x] Regression test for `control frame too large (0x81A08329)` — simulate partial frame bytes, rekey, verify no misdecode
- [x] Verify invariant #11 (reader-break poisoning) still fires on genuine guest death
- [x] CHANGELOG + commit: `feat(process): rekeyable control reader with buffer reset`

### M5 — Rekeyable terminal bridge

- [x] Terminal reader (→ `TerminalOutputQueue`) rekeys
- [x] Terminal writer (TerminalInput handler) rekeys
- [x] `CoalesceBuffer::reset()` or equivalent called on rekey; verify no byte carryover
- [x] Unit test: coalesce buffer reset semantics
- [x] CHANGELOG + commit: `feat(process): rekeyable terminal bridge`

### M6 — Stress validation + closeout

- [x] `cargo test -p capsem-process` passes
- [x] `just test` passes (full suite)
- [x] `CAPSEM_STRESS=1 uv run pytest tests/capsem-mcp/test_stress_suspend_resume.py -n 8 --tb=line -q` → 50/50, or remaining failures are loop-device I/O only
- [x] Spot-check all 11 features via `just run "capsem-doctor"` and targeted MCP traces
- [x] `stash@{0}` disposition: dropped (with `git stash drop stash@{0}`) after extraction confirmed complete, OR kept with note in ISSUE.md
- [ ] Update ISSUE.md footer: sprint closed, link to final commit range
- [ ] Open follow-up issue for loop-device I/O errors (out of scope)
