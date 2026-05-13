# Sprint: Settings Debug Report

## Tasks

- [x] Plan sprint and scope
- [x] RED: Rust report tests fail for missing debug report module
- [x] GREEN: Add service debug report collector and endpoint
- [x] RED: Frontend API/UI tests fail for missing copy-debug flow
- [x] GREEN: Add Settings About debug copy button
- [ ] Testing gate
- [x] Changelog

## Notes

- Existing `capsem support-bundle` is tarball-oriented. This sprint adds a pasteable text report so GitHub issues can carry the first-pass forensic tuple without unpacking files.
- The report must include initrd attribution because the incident hinged on `initrd-151e52b1ac1ff0a4.img` vs the local repacked initrd.
- RED proof: `cargo test -p capsem-service debug_report -- --nocapture` failed with missing `capsem_service::debug_report`, proving the new contract is not already implemented.
- Service endpoint RED proof: `cargo test -p capsem-service handle_debug_report_returns_pasteable_text -- --nocapture` failed with missing `handle_debug_report`.
- Frontend API RED proof: `pnpm exec vitest run src/lib/__tests__/api.test.ts -t getDebugReport` failed with `api.getDebugReport is not a function`.
- Settings UI RED proof: `pnpm exec vitest run src/lib/__tests__/settings-debug-report.test.ts` failed because no `Copy debug info` button existed.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-service debug_report -- --nocapture` covers report formatting, path redaction, missing assets, and the service handler.
- Functional: `pnpm exec vitest run src/lib/__tests__/api.test.ts -t getDebugReport`; `pnpm exec vitest run src/lib/__tests__/settings-debug-report.test.ts`.
- Adversarial: Rust tests cover home path redaction and missing assets without panics.
- E2E/VM: Missing until after implementation; needs real Settings click or gateway call.
- Telemetry: Not applicable.
- Performance: Missing explicit benchmark; report is user-triggered and hashes three local asset files.
- Missing/deferred: Real visual verification and gateway smoke after green.
