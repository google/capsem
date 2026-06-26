# Meta Sprint: security-event-rule-spine

## Purpose

Replace the callback-shaped Policy V2 authoring surface with one rule system
over the canonical `SecurityEvent` object.

The current implementation has the right plugin shape
(`SecurityEvent -> SecurityEvent`) but rule matching is still demultiplexed
through per-callback subjects such as HTTP request, DNS query, MCP request, and
model request. This sprint burns that drift.

## Target Shape

Rule storage has two first-principle homes:

```toml
[corp.rules.block_openai]
name = "openai_api_block"
action = "block"
detection_level = "high"
corp_locked = true
match = 'http.host.matches("(^|.*\.)(openai\.com|chatgpt\.com|oaistatic\.com|oaiusercontent\.com)$")'
```

```toml
[profiles.rules.redact_pii]
name = "openai_prompt_pii_redact"
action = "preprocess"
match = 'has(model.request.body)'
```

Provider-scoped rules are convenience/default authoring only. They normalize
into profile rules before runtime:

```toml
[plugins.credential_broker]
mode = "rewrite"
detection_level = "informational"
match = 'http.host.matches("(^|.*\.)(openai\.com|chatgpt\.com|oaistatic\.com|oaiusercontent\.com)$")'
```

## Non-Negotiables

- CEL evaluates one authored rule against one canonical `SecurityEvent`.
- No `on`.
- No `if`; use `match`.
- No separate credential blocks.
- No `aliases`.
- No user-facing top-level `policy.*` provider authoring.
- No HTTP/DNS/MCP/model verb buckets as first-principle storage.
- No rule fan-out or callback reconciliation.
- Missing roots on an event evaluate as non-matches, not hard failures.
- All runtime security event families must be first-party CEL roots.
- Plugins run on `SecurityEvent` and return `SecurityEvent`.
- Detection and enforcement share the same rule rail.
- `ask` includes the full resolution path, not just matching.
- Sigma-derived detections compile into the same enum-backed rule contract.
- Security events remain the only truth for logging, detection, enforcement,
  plugin actions, and OTEL rule labels.

## Rule Contract

Every rule has:

- `name`: mandatory stable observability name.
- `action`: mandatory rule action.
- `match`: mandatory CEL expression over `SecurityEvent`.
- `priority`: optional authoring field with source-based defaults.

`name` constraints:

- max 64 characters.
- lowercase only.
- allowed chars: `a-z`, `0-9`, `_`, `-`.
- no spaces.

`action` values:

- `allow`
- `ask`
- `block`
- `preprocess`
- `postprocess`

Detection metadata:

- `detection_level` is optional and orthogonal to action.
- Canonical levels: `informational`, `low`, `medium`, `high`, `critical`.
- `info` is accepted as authoring sugar for `informational`.
- `action = "detect"` and old `level =` authoring are invalid.

Plugin rules:

- `plugin` is required for plugin-owned actions.
- Credential broker, PII, VirusTotal, and future scanners are plugins.
- `credential_broker` runs as `postprocess`: after policy decision, before
  boundary materialization/logging, and logs only BLAKE3 substitution refs.
- PII runs as `preprocess`: after parse/normalize and before risk evaluation.
- Raw authorization headers and raw credential file contents are plugin-private;
  they are not first-party CEL fields.

Priority defaults:

- corp locked rules default to `-10`.
- built-in/default rules default to `0`.
- user/plugin rules default to `10`.

Priority validation:

- all explicit priorities must be in `[-1000, 1000]`.
- corp locked rules may use priorities `<= -10`.
- built-in defaults may only use `0`.
- user/plugin rules may use priorities `>= 10`.
- non-corp rules may not use negative priority.

## Required CEL Roots

This sprint requires first-party CEL roots for:

- `http`
- `dns`
- `mcp`
- `model`
- `file`
- `process`
- `credential`
- `snapshot`

Minimum runtime coverage:

- HTTP request and response.
- DNS query and response if emitted.
- MCP tool call, tool list, resource/list/read, prompt/list/get, initialize,
  notifications, and unknown/future MCP events.
- Model request, model response, model tool call, model tool response.
- File import/export/read/create/write/delete events, each with
  `path`, `name`, `ext`, `mime_type`, and `content`.
- Process exec request, exec complete, and audit events.
- Credential observed/substitution/broker events.
- Snapshot events.

## Status

| Sprint | Status | Purpose |
| --- | --- | --- |
| T0: Contract Freeze | Done | Freeze the TOML schema, rule naming, action vocabulary, priority defaults, and CEL missing-root semantics. |
| T1: SecurityEvent Subject | Done | Make canonical `SecurityEvent` the CEL subject with all first-party roots. |
| T2: Rule Compiler | Done | Parse corp/profile rules and normalize convenience provider rules into one internal rule registry without callback fan-out. |
| T3: Plugin Actions | Done | Run typed preprocess/postprocess plugins against matched `SecurityRule` entries and emit the mutated event. |
| T4: Detection, Enforcement, Ask | Done | Rule-match ledger, DB-backed latest/info, enum-backed action/level, 12-hex event ids, typed enforcement materialization guard, append-only ask pending/resolution records, and rule trace labels are implemented. |
| T5: Runtime Hook Burn-In | Done | Shared primary event id handoff, DB-facing matched-rule bridge, reloadable runtime ruleset, MITM HTTP/model telemetry, DNS telemetry, MCP telemetry, file monitor/tool telemetry, explicit service/process file boundaries, process exec/audit/complete, credential substitution, and snapshots are implemented and proven. |
| T6: Sigma And Refactor | Done | Sigma import/support now compiles onto `SecurityRule` without callback/string drift. |
| T7: Burn Old API | Done | Provider `on`/`if`/`decision`/`actions` authoring is removed; stale credential-block guidance is gone; first-party root drift has a CEL coverage guard. |
| T8: Verification | Done | Focused schema, CEL/security-event, provider-default, `policy_config`, `security_engine`, formatting, VM/E2E, benchmark, package, install, and docs gates pass. |
| T9: Credential Broker Catalog | Open | Reconcile `credential-broker-rule-memo.md` against the current rule contract. The rule/plugin rail is done, but the full Agent Vault-derived provider catalog and non-header credential rendering tests are not done. |

## Current Proof

- `cargo test -p capsem-core --lib security_rule_profile -- --nocapture`
  passes 18 focused contract/compiler tests, including Sigma import,
  evaluation, and stale field rejection.
- `cargo test -p capsem-core --lib provider_profile -- --nocapture`
  passes 3 adapter tests proving provider defaults compile as security rules
  and do not emit generated old `PolicyConfig` callbacks.
- `cargo test -p capsem-core --lib policy_config -- --nocapture`
  passes 450 tests, including referenced Sigma rule-file loading into
  `MergedPolicies.security_rules`.
- `cargo test -p capsem-core --lib 'net::policy_config::loader' -- --nocapture`
  passes 24 loader tests.
- `uv run pytest tests/capsem-security/test_detection_yaml.py -q`
  passes the Python parser compatibility gate for `detection.yaml`.
- `cargo test -p capsem-core --lib security_engine -- --nocapture`
  passes 40 tests, including typed `SecurityRuleSet` plugin execution,
  preprocess/postprocess stage re-evaluation, missing-plugin fail-closed
  semantics, credential-broker postprocess execution from rule metadata,
  enforcement decisions derived from the same evaluation as the ledgered
  matches, default allow for non-enforcement matches, HTTP materialization
  refusal for unresolved ask/block, ask pending row emission, and ask
  approval/denial resolution semantics, stable OTEL-style rule labels,
  12-hex `SecurityEventId`, forensic rule ledger emission,
  primary-event/rule-ledger `event_id` join, all matched rule emission,
  DB-regenerated detection/enforcement/plugin labels plus ask lifecycle rows,
  non-match zero-row behavior, file helpers, explicit file import/export/read
  roots, process exec/complete shared ids, credential substitution, and
  snapshot joins.
- `cargo test -p capsem-core --lib telemetry_hook -- --nocapture`
  passes 13 tests, including MITM HTTP and model telemetry writing
  DB-joined `security_rule_events` rows with the primary logger event id.
- `cargo test -p capsem-core --lib fs_monitor -- --nocapture` passes 17 tests,
  including file monitor rule-ledger joins and `.env` credential broker
  persistence.
- `cargo test -p capsem-core mcp::file_tools -- --nocapture` passes 45 tests,
  including the regression proof that snapshot revert returns a first-party
  file event and emits it through the async security engine from inside Tokio.
- `cargo test -p capsem-core --lib emit_explicit_file_security_events_map_import_export_and_read_roots -- --nocapture`
  passes, proving explicit `file.import`, `file.export`, and `file.read` roots
  write primary `fs_events` rows plus matched `security_rule_events` payloads.
- `cargo test -p capsem-proto log_file_boundary -- --nocapture` passes,
  proving the typed `FileBoundaryAction` IPC command/result roundtrip.
- `cargo test -p capsem-process classify_log_file_boundary -- --nocapture`
  passes, proving process IPC treats file-boundary logging as a job.
- `cargo check -p capsem-service -p capsem-process -p capsem-proto` passes
  after wiring service upload/download import/export through the process-owned
  security-event emitter.
- `cargo test -p capsem-core --lib dns::telemetry -- --nocapture`
  passes 8 tests, including canonical DNS security-event conversion.
- `cargo test -p capsem-process vsock -- --nocapture` passes 17 tests,
  including DNS primary-row/rule-ledger event id joins through the process-side
  emitter.
- `cargo test -p capsem-core --lib mcp_frame -- --nocapture` passes 50 tests,
  including MCP tool-call and notification rule-ledger joins and built-in
  provider MCP defaults logged through `security_rule_events`.
- `cargo test -p capsem-logger -- --nocapture` passes 116 unit tests plus 126
  roundtrip tests, including strict `security_rule_events` and
  `security_ask_events` checks, generated primary event ids, ask lifecycle
  roundtrip, and supplied event id preservation.
- `cargo check -p capsem-logger -p capsem-core -p capsem-process -p capsem-service -p capsem-mcp-builtin`
  passes with shared event ids on logger structs and runtime producers.
- `cargo check -p capsem-mcp-builtin` passes with the builtin MCP subprocess
  loading `MergedPolicies.security_rules` for file-tool producers.
- `cargo test -p capsem-mcp-builtin -- --nocapture` passes.
- `cargo build -p capsem-mcp-builtin -p capsem-mcp-aggregator` passes,
  rebuilding the actual host MCP binaries used by VM E2E.
- `CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest tests/capsem-e2e/test_e2e_lifecycle.py::TestDoctor::test_doctor_passes -v --tb=short -s`
  passes in 37.32s. This proves the VM doctor path after fixing the
  `snapshots_revert` async runtime violation where the host
  `capsem-mcp-builtin` used `DbWriter::write_blocking` inside Tokio.
- `CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest tests/capsem-e2e/test_framed_mcp_mitm.py::test_framed_guest_mcp_tools_call_and_session_db_rows tests/capsem-e2e/test_framed_mcp_mitm.py::test_framed_guest_mcp_invalid_json_notifications_and_string_ids -v --tb=short`
  passes after aligning MCP session DB assertions with notification rows whose
  `request_id` is intentionally `NULL`.
- `CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_fork_benchmark -v --tb=short -s`
  passes with EROFS/LZ4HC numbers: fork min/mean/max 26/28/29 ms, image
  min/mean/max 12.6/12.7/12.7 MB under the 13 MB gate, boot provision
  922/925/929 ms, and boot ready 10/12/12 ms.
- `just test` passes end-to-end. Highlights: Python main suite
  `1329 passed, 69 skipped` with 91.15% coverage; build-chain suite
  `27 passed`; injection suite `5 passed, 0 failed`; integration ledger
  `40 passed, 0 failed` with guest doctor subset `94 passed, 2 skipped`;
  install E2E container suite `30 passed, 26 skipped`; Linux arm64 `.deb`
  built and validated. The local package boot test is intentionally skipped
  when KVM/cross-arch boot is unavailable.
- Fresh `just test` benchmark artifacts: lifecycle total min/mean/max
  1052.7/1075.7/1113.8 ms, provision 971.9/993.2/1030.9 ms, exec-ready
  10.9/11.8/12.9 ms, exec 9.8/10.2/10.6 ms, delete 59.3/60.4/61.1 ms in
  `benchmarks/lifecycle/data_1.0.1780610732.json`; capsem-bench disk
  sequential write/read 1777.7/4326.0 MB/s, random write/read
  7407.0/52983.3 IOPS, rootfs sequential read 3198.6 MB/s, rootfs random read
  32775.1 IOPS, HTTP 50/50 success at 65.7 req/s with p50/p95/p99
  59.0/203.0/207.5 ms, and throughput 22.54 MB/s in
  `benchmarks/capsem-bench/data_1.0.1780610732_arm64.json`.
- `cargo test -p capsem-core --lib security_rule_profile -- --nocapture`
  passes 18/18 after the docs pass, proving the documented TOML/Sigma fixtures
  and old syntax rejection still compile through the Rust parser.
- `pnpm -C docs install --frozen-lockfile && pnpm -C docs run build` passes;
  Astro/Starlight builds 44 pages, including the rebuilt policy reference and
  updated architecture/session telemetry docs.
- Public docs stale-syntax scan for old `policy.*` / `on` / `if` / `decision`
  examples returns only the intentional warning in the new policy page that
  tells admins not to use callback-local roots.
- `cargo fmt --check -p capsem-logger -p capsem-core -p capsem-process -p capsem-service -p capsem-mcp-builtin`
  passes.
- `cargo check -p capsem-core`, `cargo fmt --check -p capsem-core`, and
  `git diff --check` pass after the Sigma adapter refactor.

## Release Hold

Cleared for branch handoff by the current proof above:

- A single cross-root authored rule evaluates against HTTP, model, and
  credential events without fan-out.
- Missing-root CEL behavior is proven safe and boring.
- Detection metadata requires valid `name` and `detection_level`.
- All rules require valid `name`.
- Priority defaults and validation are proven for corp/default/user/plugin.
- `ask` rules block materialization until resolved and log both the ask and the
  resolution.
- Sigma rules carry typed action/detection_level/name/priority metadata through
  the same engine. Done for the current parser-compatible detection YAML rail.
- Every runtime event type is either enforceable through this rail or fails a
  drift test.
- Every runtime producer passes the same 12-hex primary event id into the
  security rule ledger row for matched rules.
- Old provider authoring shape is removed and cannot silently re-enter as a
  second engine.
