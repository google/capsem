# Sprint: Settings Debug Report

## Tasks

- [x] Plan sprint and scope
- [x] RED: Rust report tests fail for missing debug report module
- [x] GREEN: Add service debug report collector and endpoint
- [x] RED: Frontend API/UI tests fail for missing copy-debug flow
- [x] GREEN: Add Settings About debug copy button
- [x] RED: Rust report JSON test fails for missing structured report/log tails
- [x] RED: CLI parse test fails for missing `capsem debug`
- [x] GREEN: Add structured JSON, status/readiness issues, log tails, setup/runtime evidence, and CLI output
- [x] GREEN: Add host install/runtime attribution (disk, install layout, binary hashes, process liveness)
- [x] Docs: explain collection and interpretation workflow
- [ ] Testing gate
- [x] Changelog

## Notes

- Existing `capsem support-bundle` is tarball-oriented. This sprint adds a pasteable text report so GitHub issues can carry the first-pass forensic tuple without unpacking files.
- The report must include initrd attribution because the incident hinged on `initrd-151e52b1ac1ff0a4.img` vs the local repacked initrd.
- RED proof: `cargo test -p capsem-service debug_report -- --nocapture` failed with missing `capsem_service::debug_report`, proving the new contract is not already implemented.
- Service endpoint RED proof: `cargo test -p capsem-service handle_debug_report_returns_pasteable_text -- --nocapture` failed with missing `handle_debug_report`.
- Frontend API RED proof: `pnpm exec vitest run src/lib/__tests__/api.test.ts -t getDebugReport` failed with `api.getDebugReport is not a function`.
- Settings UI RED proof: `pnpm exec vitest run src/lib/__tests__/settings-debug-report.test.ts` failed because no `Copy debug info` button existed.
- Follow-up scope: the report needs to be machine-readable and CLI-accessible, because release triage cannot depend on users opening Settings or copying a text block.
- Format decision: canonical bug-report payload is JSON printed by `capsem debug`.
- Updated format decision: both `capsem debug` and Settings -> About -> Copy debug report copy the same canonical JSON from `/debug/report`; the text field remains only as a human fallback in the service response.
- RED proof: `cargo test -p capsem-service json_report_captures_setup_runtime_assets_and_redacted_logs -- --nocapture` failed because `DebugReport` had no `json` field.
- RED proof: `cargo test -p capsem parse_debug -- --nocapture` failed because `MiscCommands::Debug` did not exist.
- GREEN proof: `cargo test -p capsem-service debug_report -- --nocapture` passed after adding `capsem.debug.v1` JSON, setup/runtime evidence, asset hash/size details, and redacted log tails.
- Refactor follow-up: after reviewing `crates/capsem/src/status.rs`, debug reports now include a `status` section for status/doctor-style readiness issues and defunct session summaries.
- Refactor follow-up: `capsem debug` now lives in `crates/capsem/src/status.rs` as `debug_report()`, with `main.rs` reduced to dispatch. Service readiness issue rendering moved into `capsem_service::debug_report::status_issues`.
- GREEN proof: `cargo test -p capsem status::tests -- --nocapture` passed after moving debug payload selection into `status.rs`.
- Host attribution proof: `cargo test -p capsem-service json_report_captures_setup_runtime_assets_and_redacted_logs -- --nocapture` now asserts `host`, `disk`, `install`, `host_binaries`, and `processes` fields.
- GREEN proof: `cargo test -p capsem parse_debug -- --nocapture` passed after adding `capsem debug`.
- GREEN proof: `pnpm exec vitest run src/lib/__tests__/settings-debug-report.test.ts` passed after the UI copied JSON and used the Debug report label.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-service debug_report -- --nocapture` covers report formatting, JSON schema, asset attribution, path redaction, missing assets, log redaction, and the service handler.
- Functional: `pnpm exec vitest run src/lib/__tests__/api.test.ts -t getDebugReport`; `pnpm exec vitest run src/lib/__tests__/settings-debug-report.test.ts`; CLI parse test for `capsem debug`.
- Adversarial: Rust tests cover home path redaction, token-like log redaction, and missing assets without panics.
- E2E/VM: Missing until after implementation; needs real Settings click, `capsem debug` against the service, or gateway call.
- Telemetry: Not applicable.
- Performance: Missing explicit benchmark; report is user-triggered and hashes three local asset files.
- Missing/deferred: Real visual verification and gateway smoke after green.
