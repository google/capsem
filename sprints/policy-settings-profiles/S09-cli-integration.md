# S09 - CLI Integration

## Goal

Expose settings/profile/MCP/skills control through CLI command families.

## Tasks

- Add `capsem profile list/create/fork/update/delete/show/resolve`.
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
  callback).
- Functional: CLI to service integration tests; `capsem rules add`
  then `capsem rules evaluate` roundtrip; `capsem confirm list` +
  `capsem confirm accept <id>` resolves a real pending ask end to
  end.
- Adversarial: bad ids, bad files, locked actions, service
  unavailable, attempt to `capsem rules remove` a built-in rule
  surfaces the typed `rule_is_builtin` error verbatim, accept on a
  non-existent ask id returns a typed error rather than hanging.
- E2E/VM: CLI-created profile can launch a session.
- Telemetry: CLI status/debug exposes active profile state.
- Performance: not primary.
