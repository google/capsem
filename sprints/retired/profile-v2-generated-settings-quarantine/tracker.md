# Sprint: profile-v2-generated-settings-quarantine

## Tasks

- [x] Add generated-settings authority guard tests.
- [x] Prove the guard fails on stale authority language.
- [x] Update builder/frontend/MITM comments.
- [x] Regenerate generated frontend mock if needed.
- [x] Run focused verification.
- [x] Update Profile V2 rescue tracker and changelog.
- [ ] Commit.

## Notes

- Starting from clean `profile-v2` at `0705fd60`.
- `config/defaults.json` is still used by Python tests and frontend mock
  generation, but current `rg` found no Rust runtime reads.

## Coverage Ledger

- Unit/contract: RED proof
  `uv run python -m pytest tests/test_config.py::TestGeneratedSettingsAuthorityQuarantine -q`
  failed on stale `defaults.json`/`policy_config` authority wording.
- Functional:
  `uv run python scripts/generate_schema.py` regenerated
  `frontend/src/lib/mock-settings.generated.ts`; conformance tests confirmed the
  generated fixture is current.
- Adversarial:
  Guard tests reject Rust runtime reads/embeds of `defaults.json` and
  `settings-schema.json`.
- E2E/VM: not required; no runtime behavior change.
- Telemetry: not touched.
- Performance: not touched.
- Missing/deferred: full `just test`/VM smoke remains a later Profile V2 rescue
  gate.
