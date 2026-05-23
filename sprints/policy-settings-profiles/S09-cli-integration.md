# S09 - CLI Integration

## Goal

Expose settings/profile/MCP/skills control and profile-backed VM creation
through CLI command families.

This is release-blocking for the Profile V2 bedrock release. The engine is not
usable if operators need raw HTTP, raw UDS payloads, or direct database access
to create profile-backed VMs, inspect profile state, install runtime
enforcement/detection overlays, backtest/hunt rules, or debug why a request was
blocked.

## Tasks

- Initial S07a bridge landed: `capsem profile reconcile-catalog --manifest
  <path> --pubkey <path> [--json]` and `--manifest-url <https-url>` post a
  signed catalog source to `POST /profiles/catalog/reconcile` and print either
  a compact lifecycle summary or raw JSON. Richer catalog/revision subcommands
  remain in this sprint.
- Initial catalog inspection landed: `capsem profile catalog [--json]` calls
  `GET /profiles/catalog` and prints configured source state, persisted
  manifest presence, profile ids, current/installed revisions, and canonical
  revision status values.
- Initial revision inspection landed: `capsem profile revisions <id> [--json]`
  calls `GET /profiles/{id}/revisions` and prints current/installed revision
  markers plus canonical lifecycle status values.
- Initial revision lifecycle actions landed: `capsem profile install <id>
  [--revision <rev>] [--json]`, `capsem profile update <id> [--revision <rev>]
  [--json]`, and `capsem profile remove <id> [--revision <rev>] [--json]`
  call the matching service revision actions. `install` only accepts active
  revisions, `update` reconciles lifecycle state, and `remove` clears local
  launchable state while preserving archived payloads.
- Add `capsem profile list/create/fork/update/delete/show/resolve`.
- Latest read-only profile CLI slice landed `capsem profile list`,
  `capsem profile show <id>`, and `capsem profile resolve <id>` over the
  typed service `/profiles`, `/profiles/{id}`, and `/profiles/{id}/effective`
  routes. Human output surfaces profile source, lock state, inheritance, UI
  mode, and effective counts for rules/MCP/skills/tools; `--json` preserves
  the raw service payload for scripted callers. Remaining mutating profile
  verbs are `create`, `fork`, body `update`, and `delete`; the existing
  revision lifecycle `profile update` command needs an explicit naming
  decision before body update lands.
- Latest mutating profile CLI slice landed `capsem profile fork <source>
  --id <new-id> --name <name>` and `capsem profile delete <id>` over the typed
  service routes. `fork` uses the service's schema-aware profile fork path
  instead of asking operators to hand-author a full profile document. Remaining
  mutating verbs are full-profile `create` and body `update`; both need the
  formal profile schema/admin tooling path rather than raw JSON shortcuts.
- Latest typed document slice landed `capsem profile create --file <profile>`
  and `capsem profile update <id> --file <profile>`. Files are parsed as the
  Rust `Profile` model from TOML or JSON and validated before calling the
  service. The existing revision lifecycle `profile update <id> --revision`
  remains available, but full-profile update now requires `--file` so operators
  cannot accidentally confuse revision reconciliation with profile body writes.
- Add richer status output using the canonical `ProfileRevisionStatus` enum values:
  `active`, `deprecated`, and `revoked`. A missing revision is rendered as
  absent/unknown, not as `removed`.
- Extend `capsem profile show/resolve` to print package/tool contracts, resolved
  VM asset identity, asset readiness, and revoke/deprecation warnings.
- Extend VM create/start commands to accept `--profile <id>` and optional
  `--profile-revision <rev>`. Initial `capsem create --profile
  --profile-revision` parsing and request forwarding have landed; remaining
  CLI work must show first-use asset download progress and print the resolved
  profile id/revision and asset hashes.
- Add `capsem mcp list/add/delete/show`. Initial Profile V2 connector CLI
  replacement has landed as `capsem mcp connectors`, `capsem mcp add`, and
  `capsem mcp delete`; the old `servers/tools/policy/refresh/call` verbs are
  removed instead of bridged. Remaining S09 work can refine naming/output if
  the product wants `list/show` aliases.
- Latest MCP polish slice landed `capsem mcp list` and `capsem mcp show <id>`
  as operator-friendly aliases over the Profile V2 connector route. `connectors`
  remains available as the explicit low-level spelling.
- Add `capsem skills list/add/delete/show`. Latest CLI slice landed
  `capsem skills list`, `show`, `add`, and `delete` over the service
  Profile V2 `/skills` routes, including profile selection, skill kind
  selection (`group`, `enabled`, `disabled`), ownership/editability summary
  output, and JSON output for scripted callers.
- Do not extend `capsem rules` for post-S08b behavior. If the legacy S07
  command family still exists when this sprint starts, either retire it or keep
  it explicitly documented as a compatibility shim for the closed S07 surface.
  New scripted debugging and CI use `capsem enforcement ...` and
  `capsem detection ...`.
- Add `capsem enforcement validate|compile|backtest|list|add|update|delete|stats`
  mirroring the S08b service routes. This is the runtime-installed surface for
  blocking-capable CEL enforcement rules. `backtest` prints summary counts plus
  event-level rows; default output includes up to 100 matched events, deduped
  by simple evidence signature for diversity.
- Latest CLI breadth slice landed `compile`, `update`, and `backtest` for
  enforcement rules. `install` keeps a visible `add` alias. Backtest accepts a
  JSON array, `{ "events": [...] }` envelope, single event object, or JSONL
  file of runtime backtest events, then calls the service `/enforcement/backtest`
  route and renders event/evidence rows in human output.
- Add `capsem detection validate|compile|backtest|list|add|update|delete|stats|hunt`
  mirroring the S08b service routes. Detection `hunt` is forensic over
  historical resolved-event journals; enforcement does not use the word hunt.
- Latest CLI breadth slice landed `compile`, `update`, `backtest`, and
  file-backed `hunt` for detection rules. `install` keeps a visible `add`
  alias, while `hunt-session` remains the session-db forensic shortcut.
- CLI backtest/hunt output includes event refs (`session_id`, `event_id`,
  `sequence`) and full matched field evidence by default. Redacted output is an
  explicit export/support-bundle mode, not the local debugging default.
- Add `capsem confirm list/accept/deny/promote-allow/promote-deny`
  for the [S15 ask resolve path](S15-confirm-ux.md). Two operators
  on the same session must see the same pending queue; the CLI
  shares the gateway state with the UI.
  If the bedrock release disables user-facing `ask`, the CLI must still render
  the disabled/unavailable state clearly and tests must prove ask-enabled rules
  cannot silently behave as allow.
- Bedrock release slice landed `capsem confirm list` only. It calls
  `/confirm/pending` and renders the disabled resolver state with
  `resolve_owner=S15-confirm-ux`; accept/deny/promote verbs stay in S15 and
  must not be exposed until real ask resolution exists.
- Keep command shapes consistent.
- Add parser, integration, error, and smoke tests.

## Coverage Ledger

- Unit/contract: parser tests; `capsem mcp connectors/add/delete` parser tests;
  legacy `capsem rules` retirement or compatibility-shim parser tests if the
  command still ships; `capsem enforcement backtest` and
  `capsem detection backtest` parser/output tests for default 100 matched-row
  evidence; profile
  catalog/revision output golden tests for all
  `ProfileRevisionStatus` enum values; VM create parser tests for `--profile`,
  `--profile-revision` have landed. Remaining parser coverage covers download
  progress and revoked/incompatible profile errors.
- Functional: CLI to service integration tests; enforcement add/backtest/list
  and detection add/backtest/list/hunt roundtrips; `capsem confirm list` +
  `capsem confirm accept <id>` resolves a real pending ask end to
  end. Enforcement/detection CLI commands validate, compile, backtest, install,
  list stats, and detection-hunt through the service. CLI-created VM with
  `--profile` pins the selected profile revision and assets.
- Adversarial: bad ids, bad files, locked actions, service unavailable,
  attempt to mutate locked/generated enforcement through the new CLI surfaces a
  typed ownership error verbatim, accept on a
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
