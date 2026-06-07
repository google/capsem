# The Ledger of the Realm: Testing, Architecture, And Resilience

This ledger is the shared vocabulary for Capsem engineering quality. It is not
release theater. It is how we name the proof expected before a sprint claims
that a system boundary, state machine, telemetry ledger, or security control is
ready.

When a sprint says a slice must be "Lannister grade", "Winterfell grade", or
"Iron Bank clean", it references the responsibilities below.

## Part I: The Great Houses

### House Stark of Winterfell

- Words: "Winter is Coming."
- Realm: deep core logic and immutable truth.
- Discipline: formal verification, proof systems, and cold invariants.
- Capsem meaning: use this standard for cryptographic trust chains, hashes,
  signatures, schema proofs, deterministic replay, and state that must not lie.

### House Lannister of Casterly Rock

- Words: "Hear Me Roar"; "A Lannister always pays his debts."
- Realm: state machines, ledgers, FinOps, and durable accounting.
- Discipline: property-based testing, invariant checks, and cost accounting.
- Capsem meaning: use this standard for session DB ledgers, profile state,
  VM/package/asset accounting, cost/token counters, quota dimensions, and any
  enum-backed persistence. The ledger must be queryable and balanced; opaque
  JSON blobs are debt unless explicitly justified as a bounded payload field.

### House Baratheon of Storm's End

- Words: "Ours is the Fury."
- Realm: API gateways and network endpoints.
- Discipline: security testing, fuzzing, penetration testing, and boundary
  hardening.
- Capsem meaning: use this standard for UDS/HTTP routes, MCP framing, policy
  routes, validation endpoints, auth boundaries, rate limits, malformed input,
  injection attempts, and fail-closed behavior.

### House Tyrell of Highgarden

- Words: "Growing Strong."
- Realm: capacity, performance, load, and stress.
- Discipline: performance testing, load testing, stress testing, and
  bottleneck analysis.
- Capsem meaning: use this standard for boot time, exec latency, rule matching
  latency, detection/backtest throughput, event emission, profile asset
  download behavior, and VM fleet scale claims.

### House Greyjoy of Pyke

- Words: "We Do Not Sow"; "What is dead may never die."
- Realm: resilience and self-healing systems.
- Discipline: chaos engineering, restart proof, network partition tolerance,
  and crash recovery.
- Capsem meaning: use this standard for process supervision, background
  downloads, reconnect loops, service/process restarts, VM lifecycle recovery,
  WAL/session DB durability, and cleanup races.

### House Martell of Dorne

- Words: "Unbowed, Unbent, Unbroken."
- Realm: edge cases and defensive execution.
- Discipline: mutation testing, fault injection, and adversarial test design.
- Capsem meaning: use this standard for parser hardening, malformed payloads,
  partial streams, invalid signatures, ambiguous linkage, locked profile
  mutation attempts, missing assets, and deliberately poisoned test fixtures.

### House Targaryen of Dragonstone

- Words: "Fire and Blood."
- Realm: infrastructure as code and disaster recovery.
- Discipline: ephemeral environment provisioning, rebuild proof, and automated
  failover.
- Capsem meaning: use this standard for image builds, bootstrap, release
  bundles, signed asset rebuilds, clean-room setup, disaster recovery, and
  "burn it down and rebuild it" verification.

## Part II: The Small Council And The Court

### The Citadel And The Maesters

- Domain: documentation, schemas, and ADRs.
- Capsem meaning: every durable contract needs a schema or typed model, docs
  for operators/developers, and sprint notes that preserve why the decision was
  made.

### The Master Of Whisperers

- Domain: distributed tracing and log aggregation.
- Capsem meaning: every resolved event must carry enough trace, activity,
  process, profile, VM, user, and accounting ids for timeline, debugging,
  telemetry, and forensic reconstruction.

## Part III: Across The Narrow Sea

### Essos And The Free Cities

- Domain: third-party APIs, webhooks, and event buses.
- Discipline: contract testing and circuit breakers.
- Capsem meaning: provider APIs, model APIs, MCP servers, catalog endpoints,
  update endpoints, and enterprise sinks need typed contracts, timeout paths,
  retries where appropriate, and explicit unsupported-shape diagnostics.

### The Iron Bank Of Braavos

- Domain: CI/CD, static gates, coverage gates, and compliance.
- Discipline: linting, static analysis, policy enforcement, and release gates.
- Capsem meaning: no sprint is complete until focused tests, formatting, diff
  hygiene, changelog, tracker updates, and release holds match reality.

### The Many-Faced God

- Domain: mocks, stubs, and service virtualization.
- Discipline: deterministic test isolation.
- Capsem meaning: use fakes and fixtures for provider APIs, MCP servers,
  profile catalogs, image inventories, and service/process IPC when the goal is
  to isolate a contract without waiting on an external system.

## Part IV: Beyond The Wall

### The Free Folk

- Domain: open-source ecosystems and third-party packages.
- Discipline: supply-chain security and dependency scanning.
- Capsem meaning: Cargo, PyPI, npm, uv, builder packages, guest packages, and
  MCP dependencies require SBOMs, pinned contracts, vulnerability scanning, and
  release review.

### The Night's Watch

- Domain: production monitoring, APM, alerting, and incident response.
- Discipline: real-time telemetry and operational readiness.
- Capsem meaning: status, metrics, debug reports, logs, OpenTelemetry,
  Prometheus, and alerts must explain the live system before users need support.

### The White Walkers And The Army Of The Dead

- Domain: legacy code, technical debt, and unpatched vulnerabilities.
- Capsem meaning: old settings paths, old policy runtimes, untyped JSON
  manipulation, unowned session tables, stale compatibility lanes, and hidden
  CVEs must be burned out, not wrapped in shims.

## Bannerman's Quick Reference

| Faction / House | Software discipline | Core responsibility |
| :--- | :--- | :--- |
| House Stark | Formal verification | Cryptographic proofs, state immutability, absolute truth |
| House Lannister | Property testing and FinOps | Ledger balance, invariant states, cost accounting |
| House Baratheon | Penetration testing | Boundary defense, fuzzing, malicious payload handling |
| House Tyrell | Load and performance testing | Autoscaling, bottlenecks, throughput, latency |
| House Greyjoy | Chaos engineering | Auto-recovery, restart resilience, partition handling |
| House Martell | Mutation testing | Blind-spot detection through deliberate sabotage |
| House Targaryen | Infrastructure as code | Clean rebuilds, ephemeral environments, disaster recovery |
| The Citadel | Documentation and schemas | API definitions, runbooks, ADRs, typed contracts |
| Master of Whisperers | Distributed tracing | End-to-end traceability and log aggregation |
| Essos and the Free Cities | API contract testing | External API pacts and circuit breakers |
| The Iron Bank | CI/CD gates | Static gates, linting, compliance, release blocking |
| Many-Faced God | Mocking and virtualization | Deterministic isolation with fakes and fixtures |
| The Free Folk | Supply-chain security | Dependency scanning and SBOM proof |
| The Night's Watch | Telemetry and alerting | Monitoring, APM, incident response |
| White Walkers | Legacy and tech debt | Remove stale, unsafe, unmaintained paths |

## Current Capsem Translation

- Lannister-grade persistence means typed Rust/Pydantic contracts, explicit
  enum conversion, relational query surfaces for state and accounting, property
  or invariant tests where state transitions matter, and no hidden JSON bed.
- Winterfell-grade trust means hashes, signatures, schemas, strict parsers,
  deterministic fixtures, and replayable proof.
- Baratheon-grade APIs fail closed on malformed input, injection attempts,
  locked mutations, unsupported rules, and ambiguous authority.
- Tyrell-grade performance requires measured VM-originated latency, throughput,
  and rule-count scaling before public claims.
- Greyjoy-grade resilience requires restart/retry/race tests for every
  background worker and lifecycle edge.
- Iron-Bank clean means the tracker, changelog, focused verification, and final
  release holds all agree with git history.
