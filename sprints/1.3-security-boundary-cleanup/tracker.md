# Sprint: 1.3 Security Boundary Cleanup

## Status

In progress. No implementation is accepted until RED tests prove the boundary
failure first.

## Tasks

- [x] Capture sprint boundary and end posture.
- [ ] RED: security-engine contract proves broker pre-plugin plus sanitizer
  final-plugin are required to keep runtime bytes working and ledger bytes safe.
- [ ] RED: network header formatter cannot create credential refs, hashes, or
  provider-sensitive redaction.
- [ ] RED: logger write path fails closed when final log sanitizer is absent or
  disabled for security-event materialization.
- [ ] Implement explicit broker pre-plugin / sanitizer final-plugin split.
- [ ] Split runtime materialization from ledger materialization.
- [ ] Burn credential-sensitive logic from network formatter/intercept helpers.
- [ ] Rename/docs cleanup for touched boundaries: network engine, security
  engine, credential broker, log sanitizer.
- [ ] Update architecture docs with the explicit runtime-vs-ledger
  materialization contract.
- [ ] Update developer skills with the no-drift rule: no credential handling in
  network formatters, DB readers, frontend transforms, or one-off harnesses.
- [ ] Ironbank: HTTP credential header request reaches upstream while DB/log/UI
  route payloads contain no raw secret.
- [ ] Ironbank: query, JSON body, form body, response token body, and model SDK
  replay get the same no-raw-ledger proof.
- [ ] Add plugin latency/counter evidence for broker and sanitizer.
- [ ] Update CHANGELOG.md.
- [ ] Focused test gate.
- [ ] Commit and push this slice before returning to broader bug hotlist.

## Invariants

- Network engine parses and routes; it does not decide, broker, redact, or
  credential-classify.
- Security engine is the only rule/plugin/decision rail.
- Credential broker pre-plugin owns capture/store/inject metadata.
- Log sanitizer final-plugin owns durable projection.
- Upstream/runtime bytes and ledger bytes are separate materializations.
- Raw credential material must never reach session DB, structured logs, route
  JSON, or frontend stats.
- Missing sanitizer is a failure, not a fallback to raw logging.
- No compatibility rail, no fallback logger, no formatter side-channel.

## Coverage Ledger

- Unit/contract: pending.
- Functional: pending.
- Adversarial: pending.
- E2E/VM: pending in `tests/ironbank/`.
- Telemetry: pending.
- Performance: pending plugin counters/latency evidence.
- Docs/skills: pending architecture docs and developer skill updates.
- Missing/deferred: none accepted for release blocker scope.
