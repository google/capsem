# Settings Debug Report Plan

## Goal

Add Settings and CLI surfaces that collect redacted, pasteable Capsem debug information for GitHub bug reports. The report must make release/runtime attribution obvious, especially the binary version, asset version, initrd hash, resolved initrd path, whether installed asset files match the signed manifest, setup-state signals, and the most relevant host log tails.

## Decisions

- Reuse the existing gateway proxy path by adding a service endpoint that returns both a human text report and canonical structured JSON.
- Keep the existing `capsem support-bundle` tarball intact; this sprint adds a lightweight pasteable report for bugs.
- Add `capsem debug` as the terminal-first collection path. It prints the structured JSON object so issue templates, scripts, and release triage can consume the same data.
- Report paths with home directories collapsed to `~/` so users can paste publicly with less identifying noise.
- Include manifest-derived and file-derived asset evidence so a report can be tied back to a specific initrd asset.
- Include redacted log tails, not full logs, so the report is useful in GitHub without becoming a support bundle or leaking bearer/API tokens.

## Files

- `crates/capsem-service/src/debug_report.rs`
- `crates/capsem-service/src/debug_report/tests.rs`
- `crates/capsem-service/src/lib.rs`
- `crates/capsem-service/src/main.rs`
- `frontend/src/lib/api.ts`
- `frontend/src/lib/components/shell/SettingsPage.svelte`
- `frontend/src/lib/__tests__/api.test.ts`
- `frontend/src/lib/__tests__/settings-debug-report.test.ts`
- `crates/capsem/src/main.rs`
- `crates/capsem/src/client.rs`
- `docs/src/content/docs/debugging/debug-report.md`
- `CHANGELOG.md`
- `sprints/debug-report-settings/{plan.md,tracker.md,MASTER.md}`

## Done

- A user can open Settings -> About and copy a redacted debug report.
- A user can run `capsem debug` and paste the structured JSON into a bug.
- The frontend calls the service through the gateway, not a separate local command.
- The report includes version/build, platform, Capsem paths, service runtime files, VM counts, setup state, manifest versions, resolved kernel/initrd/rootfs paths, expected hashes, actual hashes, sizes, match status, and redacted host log tails.
- Tests prove the report includes initrd attribution and redacts home paths before implementation.
- Docs and changelog explain how to collect and interpret the report.

## Testing Proof Matrix

- Unit/contract: Rust tests for report formatting, JSON schema, asset attribution, log tail redaction, setup-state capture, and path redaction.
- Functional: Frontend API test for `GET /debug/report`; Settings component test for copy-to-clipboard state; CLI parse test for `capsem debug`.
- Adversarial: Tests ensure home paths and token-like log fields are redacted and missing assets are represented instead of panicking.
- E2E/VM: Deferred for this slice; Settings visual verification, `capsem debug` against a running service, and a real gateway call are the follow-up gate.
- Telemetry: Not applicable; this is a support/debug read path.
- Performance: Report hashes only three VM asset files; acceptable on click, no startup cost.
