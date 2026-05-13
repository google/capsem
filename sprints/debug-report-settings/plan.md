# Settings Debug Report Plan

## Goal

Add a Settings surface that collects redacted, pasteable Capsem debug information for GitHub bug reports. The report must make release/runtime attribution obvious, especially the binary version, asset version, initrd hash, resolved initrd path, and whether installed asset files match the signed manifest.

## Decisions

- Reuse the existing gateway proxy path by adding a service endpoint that returns a plain text report.
- Keep the existing `capsem support-bundle` tarball intact; this sprint adds a lightweight pasteable report for bugs.
- Report paths with home directories collapsed to `~/` so users can paste publicly with less identifying noise.
- Include manifest-derived and file-derived asset evidence so a report can be tied back to a specific initrd asset.

## Files

- `crates/capsem-service/src/debug_report.rs`
- `crates/capsem-service/src/debug_report/tests.rs`
- `crates/capsem-service/src/lib.rs`
- `crates/capsem-service/src/main.rs`
- `frontend/src/lib/api.ts`
- `frontend/src/lib/components/shell/SettingsPage.svelte`
- `frontend/src/lib/__tests__/api.test.ts`
- `frontend/src/lib/__tests__/settings-debug-report.test.ts`
- `sprints/debug-report-settings/{plan.md,tracker.md,MASTER.md}`

## Done

- A user can open Settings -> About and copy a redacted debug report.
- The frontend calls the service through the gateway, not a separate local command.
- The report includes version/build, platform, Capsem paths, service runtime files, VM counts, manifest versions, resolved kernel/initrd/rootfs paths, expected hashes, actual hashes, and match status.
- Tests prove the report includes initrd attribution and redacts home paths before implementation.

## Testing Proof Matrix

- Unit/contract: Rust tests for report formatting, asset attribution, and path redaction.
- Functional: Frontend API test for `GET /debug/report`; Settings component test for copy-to-clipboard state.
- Adversarial: Tests ensure home paths are redacted and missing assets are represented instead of panicking.
- E2E/VM: Deferred for this slice; Settings visual verification and a real gateway call are the follow-up gate.
- Telemetry: Not applicable; this is a support/debug read path.
- Performance: Report hashes only three VM asset files; acceptable on click, no startup cost.
