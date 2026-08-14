# S05-176 tracker

- [x] Diagnose the exact failed run and prove the final source stayed clean.
- [x] Record the systemic design and release evidence.
- [x] Capture RED regression tests.
- [x] Implement frozen source capture and strict image receipt.
- [x] Run focused, Ruff, Ty, source-contract, and resume checks.
- [ ] Commit and push the exact fix SHA.
- [ ] Qualify that SHA and prove glow-up/install.

## Evidence

- Retained prefix: `/home/elieb_google_com/.cg/ba9e862659612d8133addca3408b37381604ab76`.
- Run: `20260814-205354-00157f-candidate`.
- Recorded/final digest:
  `1a3c0e2669982e81a1cf501cb4596d8701c46c97547c5c039d134667ad0c3546`.
- Built source tag: `capsem-install-test:efc5690f7cb247b34b6319feda3f613f`.
- Smoke-required tag: `capsem-install-test:3baa21700c22aa62f31de9739862b944`.
- RED: five focused contracts failed on the absent snapshot API, raw digest
  acceptance, absent receipt, missing carry check, and missing graph edge.
- GREEN: 79 install/resume/composition tests; 222 source/config/candidate tests;
  545 artifact/boundary/type/plan/release contracts with 2 platform skips.
- Ruff is green, and strict all-platform Ty was rerun after formatting.
- First exact `ab61e39c` attempt stopped intentionally during fast Citadel
  after the observer reported the new snapshot's expected duplicate symlink
  bytes. RED observer contracts then established a separate exact source-
  replica vocabulary. GREEN: 43 full observation/propagation/identity tests
  and 72 config/source/Citadel checks; sibling duplicates and source hardlinks
  remain faults. A new committed SHA is required for qualification.
