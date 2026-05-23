# S22 - Rate Limits, Budgets, And Quotas

## Status

Proposed standalone sprint. Not part of the S08/S13 ship path and not part of
the bedrock release.

## Goal

Design and implement cross-surface throttling, rate limits, and cost budgets
without rewriting the Security Engine.

Capsem needs to govern request rates and spend across HTTP, MCP, model calls,
tokens, estimated cost, tools, profiles, users, VMs, and sessions. This is a
serious product/security design sprint, not a small plugin task and not an S08
requirement.

The bedrock release must reserve the attachment points, but S22 owns quota
semantics. S22 may add a local engine, plugin-backed provider, or hybrid
coordinator behind the existing Security Engine action/result model. It must
not rename event identities, policy roots, profile pinning, route families, or
resolved-event journal semantics.

## Product Contract

- S08b must preserve the attachment points: normalized events carry enough
  quota dimensions, `SecurityAction` can represent a future throttle decision,
  and resolved events can record why an action was delayed or denied.
- S12 must expose the live counters/cost summaries needed by a future budget
  engine, but S12 does not enforce budgets.
- S13 remains remote enforcement/observer plumbing. It does not become the
  budget system.
- S22 decides whether the main implementation is:
  - a local first-class Rate Limit/Budget Engine;
  - a plugin-backed quota provider behind a local contract;
  - a hybrid local-fast-path plus centralized coordinator.
- The design must work when offline/local-only and when centrally managed.
- Enforcement must be auditable: every throttle, allow-after-delay, deny, or
  budget-exhausted decision is attached to the resolved event and visible in
  status/debug/timeline/telemetry.

## Surfaces To Govern

- HTTP request count, bytes, host/domain/path class, methods, and burst rates.
- DNS query count and domain class.
- MCP server/tool calls, arguments class, call duration, and burst rates.
- Model requests, provider/model, input tokens, output tokens, estimated cost,
  and exact cost corrections when available.
- File/process activity where future profiles need quotas, such as write rates,
  snapshot creation, process spawn bursts, or long-running execs.
- Per-user, per-profile, per-VM, per-session, per-team/corp, and per-provider
  scopes.

## Design Questions

- Should the primary abstraction be a local Rate Limit/Budget Engine, a plugin,
  or a hybrid? Bias is allowed, but the sprint must decide with tests and
  failure-mode analysis.
- Which algorithms are used for each quota class: token bucket, leaky bucket,
  sliding window, fixed window, cost ledger, or explicit reservation?
- Which decisions exist: allow, delay/throttle, ask, deny, degrade, or require
  external approval?
- How are model costs handled before exact usage is known: preflight estimate,
  reservation, post-hoc correction, or both?
- What is the fail-open/fail-closed story for centralized quota providers?
- How are concurrent requests charged safely without creating hot locks?
- Which quota state is persisted across VM restarts and which is live-only?
- How do corp profiles define immutable quota policy while user profiles define
  local preferences?
- How does the UI explain "why was this delayed/blocked?" without exposing raw
  prompts, secrets, URLs, or arguments as labels?

## Required S08/S12 Compatibility

S08b should reserve:

- `SecurityAction::Throttle(ThrottlePlan)` or equivalent typed action;
- resolved-event step kind for `rate_limit_check`;
- quota dimensions on `SecurityEvent`:
  `profile_id`, `profile_revision`, `vm_id`, `session_id`, `user_id`,
  event family/type, provider/model, MCP server/tool, HTTP host/method/path
  class, DNS domain class, estimated tokens/cost, request/byte counts, and
  correlation ids;
- final-action evidence fields for throttle delay, deny reason, quota id,
  budget scope, and provider/plugin source.

S12 should preserve counters and summaries for:

- request counts and denials by family;
- MCP call counts and errors;
- model calls, tokens, provider/model summaries, and estimated cost;
- enforcement and detection match stats;
- future budget/throttle counters without requiring a schema rewrite.

## Later Public Surfaces

Exact API names are decided in this sprint, but expected families include:

- `capsem budget ...` and/or `capsem quota ...` CLI commands;
- UDS/HTTP endpoints for quota status, validation, dry-run, overrides, and
  history;
- profile TOML schema for quota policy;
- admin validation/schema support in `capsem-admin`;
- UI status panels showing live usage, remaining budget, throttled requests,
  and recent reasons;
- OTel/status fields with bounded labels only.

## Tasks

- Write ADR for local engine vs plugin vs hybrid centralized quota provider.
- Define quota schema in profile/service settings and `capsem-admin` Pydantic
  models.
- Implement local fast-path rate-limit/budget evaluation or plugin-backed
  provider, depending on ADR.
- Wire evaluations through the Security Engine using the S08b `Throttle`
  compatibility point.
- Add status/debug/timeline/telemetry explanations.
- Add CLI, UDS, HTTP, and UI surfaces after the core contract is stable.
- Add centralized-provider integration only after the local contract works.

## Coverage Ledger

- Unit/contract: quota schema, algorithm behavior, clock handling, persistence,
  and concurrent reservation/correction.
- Functional: HTTP/MCP/model requests are allowed, throttled, delayed, denied,
  and recorded according to policy.
- Adversarial: clock skew, service restart, concurrent bursts, centralized
  provider timeout, malicious profile values, missing cost data, and retry loops.
- E2E/VM: VM-originated HTTP/MCP/model workloads hit quota limits and expose
  correct user-facing behavior.
- Telemetry: bounded OTel/status counters for usage, remaining budgets,
  throttles, denies, errors, and provider health.
- Performance: hot-path budget checks stay below the S08d engine overhead budget
  and do not add unbounded locks or remote calls to every event.
