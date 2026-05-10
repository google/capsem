# Sprint: audit-bugs

Status: Queue created from audit findings. No fixes started here yet.

## Priority Queue

- [ ] **AB-001 [P1] Prefix CORS can leak gateway token**
  - Files: `crates/capsem-gateway/src/main.rs`, `crates/capsem-gateway/src/auth.rs`
  - Finding: `AllowOrigin::predicate` accepts origins by string prefix, so
    `http://localhostevil.com` and `https://127.0.0.1.evil.example` can receive
    CORS permission. `GET /token` is unauthenticated except for loopback peer IP,
    so a malicious browser page can read the local gateway token.
  - First verification: flip/add the negative localhost-like origin test and
    confirm it fails on current code.
  - Expected fix: parse `Origin` as a URL and require exact loopback hosts or
    `tauri://localhost` as appropriate.
  - Suggested tests: gateway CORS tests for `localhostevil.com`,
    `127.0.0.1.evil.example`, valid `localhost:<port>`, valid `127.0.0.1:<port>`,
    and valid Tauri origin.

- [ ] **AB-002 [P1] User MCP servers shadow corp definitions**
  - Files: `crates/capsem-core/src/mcp/mod.rs`, `crates/capsem-core/src/mcp/tests.rs`
  - Finding: user manual MCP servers are inserted into `seen` before
    corp-injected servers, so a user can define the same server name and cause
    the corp definition to be skipped. This contradicts the documented
    `corp > user > defaults` policy and the settings docs that say corp MCP
    servers cannot be removed.
  - First verification: add a same-name user/corp MCP server test and confirm
    the effective server comes from user config today.
  - Expected fix: process corp definitions before user/manual/autodetected
    definitions, or replace same-name user entries with corp values.
  - Suggested tests: same-name corp/user collision, corp enabled override on a
    user-defined server, and no regression for unique manual servers.

- [ ] **AB-003 [P2] Deep link values are interpolated into eval**
  - Files: `crates/capsem-app/src/main.rs`
  - Finding: `dispatch_deep_link` builds JavaScript with CLI/deep-link values
    and only escapes single quotes. Backslashes, newlines, and malformed string
    literals can break out of the JS object passed to `window.eval`.
  - First verification: add unit coverage for a value containing backslash,
    quote, newline, and a JS suffix; inspect generated script or move to an
    event API with direct payload tests.
  - Expected fix: emit a Tauri event or serialize the payload with
    `serde_json::to_string` before passing data into JavaScript.
  - Suggested tests: malformed `--connect` and `--action` arguments remain data,
    not executable source.

- [ ] **AB-004 [P2] WebSocket auth token travels in URLs**
  - Files: `crates/capsem-gateway/src/auth.rs`, `frontend/src/lib/api.ts`,
    `frontend/src/lib/components/terminal/TerminalFrame.svelte`
  - Finding: gateway auth accepts `?token=` on `/terminal/{id}` and `/events`,
    while the frontend comment says the token is never placed in URLs. These
    URLs are visible to browser/network tooling and can leak through request
    tracing unless every URI is redacted.
  - First verification: inspect TraceLayer output for a WebSocket request with
    `?token=` and decide whether the supported contract is "no tokens in URLs"
    or "queries are allowed but redacted everywhere."
  - Expected fix: use a WebSocket subprotocol, one-shot WebSocket ticket, or
    explicit URI-query redaction plus documentation update.
  - Suggested tests: auth accepts the replacement path and logging never emits
    the raw token.

- [ ] **AB-005 [P2] Builtin MCP pool can be defeated by singleton lock**
  - Files: `crates/capsem-core/src/mcp/server_manager.rs`,
    `crates/capsem-mcp-builtin/src/main.rs`
  - Finding: pooled stdio builtin peers need distinct singleton lock files when
    `CAPSEM_SESSION_DIR` is set. Otherwise peer 0 owns `mcp-builtin.lock` and
    later peers exit, making `CAPSEM_MCP_BUILTIN_POOL>1` behave like pool size 1.
  - First verification: confirm whether the current branch already carries the
    `CAPSEM_BUILTIN_PEER_INDEX` fix and add a regression test or integration
    smoke that proves multiple builtin peers stay alive.
  - Expected fix: pass a peer index from the manager and use per-peer lock names
    in the builtin process.
  - Suggested tests: pool size greater than 1 produces more than one live
    builtin peer when `CAPSEM_SESSION_DIR` is present.

- [ ] **AB-006 [P2] Stdio MCP weakens aggregator isolation contract**
  - Files: `crates/capsem-core/src/mcp/mod.rs`,
    `crates/capsem-core/src/mcp/server_manager.rs`,
    `crates/capsem-mcp-aggregator/src/main.rs`,
    `docs/src/content/docs/architecture/mcp-aggregator.md`
  - Finding: docs and module comments describe the aggregator as a network-only
    subprocess with no filesystem access, but stdio MCP definitions spawn
    configured host commands from inside the aggregator process.
  - First verification: decide whether stdio support is intentional product
    behavior or a regression against the privilege-separation design.
  - Expected fix: either reject/sandbox stdio definitions in the aggregator, or
    update the architecture docs and threat model to describe host command
    execution accurately.
  - Suggested tests: if rejected, stdio definitions are skipped with a clear
    status; if supported, add tests for command allowlisting/sandbox behavior.

- [ ] **AB-007 [P2] Unicode paths can panic snapshot rendering**
  - Files: `crates/capsem-core/src/mcp/file_tools.rs`
  - Finding: `truncate_path` slices UTF-8 by byte offset. Long non-ASCII paths or
    snapshot names can put the offset inside a code point and panic while
    rendering snapshot lists or changes.
  - First verification: add a unit test with a long Unicode path whose truncation
    boundary falls inside a multibyte character.
  - Expected fix: truncate by `char_indices`, grapheme clusters, or display width.
  - Suggested tests: ASCII and Unicode paths both truncate without panic and
    preserve the max display contract.

- [ ] **AB-008 [P3] Failed-session preservation is not idempotent**
  - Files: `crates/capsem-service/src/main.rs`, `crates/capsem-service/src/tests.rs`
  - Finding: a duplicate failed-session preservation path treats `NotFound` as
    log loss and then warns that the original directory is orphaned, even when a
    previous call already renamed it successfully.
  - First verification: add a service unit test that calls preservation twice on
    the same failed session directory.
  - Expected fix: treat `NotFound` from rename/remove as already preserved or
    already cleaned.
  - Suggested tests: first call preserves, second call is quiet/idempotent, and
    real permission errors still warn.

- [ ] **AB-009 [P3] Status colors bypass design tokens**
  - Files: `frontend/src/lib/components/shell/Toolbar.svelte`,
    `frontend/src/lib/components/views/LogsView.svelte`,
    `frontend/src/lib/components/views/ServiceLogsView.svelte`,
    `site/src/components/InstallCommand.svelte`
  - Finding: the UI pass found a small number of raw status colors
    (`green`, `amber`, `red`) instead of semantic status tokens or shared status
    classes.
  - First verification: decide the semantic token/class names for connected,
    warning, error, and success status.
  - Expected fix: move raw status color classes to shared semantic styles or
    existing design tokens.
  - Suggested tests: frontend check/build; visual smoke if the toolbar or log
    views change materially.

- [ ] **AB-010 [P1] Capsem MCP is unusable from Codex when the service is not launchctl-loaded**
  - Files: `crates/capsem-mcp/src/main.rs`, `crates/capsem-service/src/main.rs`,
    `crates/capsem/src/main.rs`, installation/LaunchAgent setup code
  - Finding: basic Codex MCP calls currently fail with `Transport closed`
    (`capsem_panics`, `capsem_version`, `capsem_list`). `capsem-mcp` starts and
    registers 26 tools, but tool calls close from the client side. Host evidence:
    `capsem status` reports `Installed: true` and `Running: false`, while
    `launchctl print gui/501/com.capsem.service` says the service is not loaded
    even though `/Users/elie/Library/LaunchAgents/com.capsem.service.plist`
    exists. `capsem-mcp` then logs `Service not responding, attempting to
    relaunch...`, spawns `target/debug/capsem-service` ad hoc, and continues
    sending UDS requests, but Codex still receives `Transport closed`.
  - First verification: from a clean shell where launchctl does not have
    `com.capsem.service`, call `capsem_version`, `capsem_list`, and
    `capsem_service_logs` via MCP and assert whether they return structured
    errors or close the transport.
  - Expected fix: MCP should either ensure the service is properly bootstrapped
    through the supported service manager path, or return a structured
    actionable error. A missing/unloaded service must not make the MCP transport
    disappear.
  - Suggested tests: integration test for host MCP startup when the service sock
    is absent/stale and launchctl reports the service unloaded; test that
    service relaunch failures are surfaced as MCP tool errors instead of
    transport closure.

## Validation Already Run During Audit

- [x] `cargo check --workspace`
- [x] `cargo check --workspace --all-targets`
- [x] `cargo clippy --workspace -- -D warnings`
- [x] `cargo test -p capsem-core dns_parser --lib`
- [x] `cargo test -p capsem-gateway cors`
- [x] `cargo test -p capsem-core build_server_list --lib`
- [x] `frontend/pnpm run check`
- [x] `frontend/pnpm test`
- [x] `frontend/pnpm run build`
- [x] `cargo audit`
- [x] `frontend/pnpm audit`

## Known Gaps From Audit Pass

- [ ] Full `just test` was not run for this audit queue.
- [ ] VM smoke was not run.
- [ ] `site` and `docs` builds were not verified because their dependencies were
      not installed in this workspace.
- [ ] Capsem MCP triage tools were unavailable during the audit with
      `Transport closed`.

## Notes

- 2026-05-07: Created from review findings. Duplicate review comments were
  deduplicated into stable bug IDs AB-001 through AB-009.
- 2026-05-07: Dependency audit note: `cargo audit` reported no vulnerabilities,
  but did report allowed unmaintained warnings in the Linux/Tauri GTK chain plus
  `bincode`, `fxhash`, `instant`, and `proc-macro-error`. `frontend/pnpm audit`
  reported no known vulnerabilities.
- 2026-05-07: MCP smoke from Codex failed. `capsem_panics`, `capsem_version`,
  and `capsem_list` all returned `Transport closed`; CLI status showed the
  LaunchAgent plist exists but launchctl does not have the service loaded.
