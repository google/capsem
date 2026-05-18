# S09 - CLI Integration

## Goal

Expose settings/profile/MCP/skills control and profile-backed VM creation
through CLI command families.

## Tasks

- Add `capsem profile list/create/fork/update/delete/show/resolve`.
- Add `capsem profile catalog`, `capsem profile revisions <id>`,
  `capsem profile install/update/remove <id> [--revision ...]`, and status
  output using the canonical `ProfileRevisionStatus` enum values:
  `active`, `deprecated`, `removed`, and `revoked`.
- Extend `capsem profile show/resolve` to print package/tool contracts, resolved
  VM asset identity, asset readiness, and revoke/deprecation warnings.
- Extend VM create/start commands to accept `--profile <id>` and optional
  `--profile-revision <rev>`. The command must show first-use asset download
  progress and print the resolved profile id/revision and asset hashes.
- Add `capsem mcp list/add/delete/show`.
- Add `capsem skills list/add/delete/show`.
- Add `capsem rules list/show/add/remove/evaluate` mirroring the
  [S07 Rules API](S07-uds-service-api.md#rules-api). `capsem rules
  evaluate` takes a subject (file or stdin JSON), a callback type,
  and an optional profile, and prints the would-be decision
  (matched rule, action, reason) without enforcing -- the CLI
  primitive for scripted policy debugging and CI.
- Add `capsem confirm list/accept/deny/promote-allow/promote-deny`
  for the [S15 ask resolve path](S15-confirm-ux.md). Two operators
  on the same session must see the same pending queue; the CLI
  shares the gateway state with the UI.
- Keep command shapes consistent.
- Add parser, integration, error, and smoke tests.

## Coverage Ledger

- Unit/contract: parser tests; `capsem rules evaluate` parser tests
  (subject from `--subject path.json` vs stdin; rejects missing
  callback); profile catalog/revision output golden tests for all
  `ProfileRevisionStatus` enum values; VM create parser tests for `--profile`,
  `--profile-revision`, download progress, and revoked/incompatible profile
  errors.
- Functional: CLI to service integration tests; `capsem rules add`
  then `capsem rules evaluate` roundtrip; `capsem confirm list` +
  `capsem confirm accept <id>` resolves a real pending ask end to
  end. CLI-created VM with `--profile` pins the selected profile revision and
  assets.
- Adversarial: bad ids, bad files, locked actions, service
  unavailable, attempt to `capsem rules remove` a built-in rule
  surfaces the typed `rule_is_builtin` error verbatim, accept on a
  non-existent ask id returns a typed error rather than hanging, revoked
  profile cannot be used for new VM create, incompatible profile revision
  explains the binary compatibility failure, interrupted first-use download
  resumes or fails with a typed cleanup hint, and stale catalog/rollback
  rejection is rendered without suggesting a destructive fix.
- E2E/VM: CLI-created/profile-selected VM can launch a session after verified
  first-use asset download; CLI status shows package/tool contract proof after
  the VM probe runs.
- Telemetry: CLI status/debug exposes active profile revision, package contract,
  asset readiness, and VM pin state.
- Performance: not primary.
