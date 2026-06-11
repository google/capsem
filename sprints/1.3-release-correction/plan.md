# Plan: 1.3 Release Correction

## Goal

Make 1.3 release-ready by fixing the product contract, not patching individual
symptoms. We need one coherent profile-owned configuration path, one hermetic
test/doctor/benchmark substrate, complete route contracts, and UI/TUI surfaces
that reflect those contracts exactly.

## Non-Negotiables

- No compatibility rail for retired config or policy paths.
- No `user.toml`; remove reads, writes, env overrides, tests, benchmarks, and
  helper code that depend on it.
- No manual credential/client run as debugging strategy. Real AGY/Claude/Codex
  auth is final compatibility confirmation only.
- No hidden fallback route, default-only profile route, benchmark-only server,
  or release gate that skips the real VM/security/logging path.
- No synthetic UI vocabulary for profile/security/plugin states. If the UI
  displays it, the route contract owns it.
- No asset blobs in `.pkg` or `.deb`.

## Key Decisions

1. Profiles are real objects, not default settings.
2. A session is an execution of one profile.
3. Profile files are materialized by `capsem-admin`; CI, local install, doctor,
   and tests all use that same path.
4. Profile-owned files include profile TOML, root bootstrap files, MCP config,
   enforcement TOML, detection YAML, plugin config, manifest entries, OBOM pins,
   tips, and surface availability.
5. Corp can lock or add constraints but does not live in UI settings.
6. Rule predicates operate on parsed facts: `http`, `dns`, `mcp`, `model`,
   `file`, `process`, `ip`, `tcp`, `udp`.
7. Rule outputs/security ledger are outputs, not predicate inputs.
8. Plugins can mutate events and decisions through pre/post stages, record
   detection evidence, and expose status/counters; profile/corp config controls
   plugin mode.
9. Credential broker owns credential capture/broker/inject behavior and exposes
   opaque references/status only.
10. Doctor is the canonical in-VM truth probe and must exercise real rails.

## Execution Order

### S0. Sprint Ledger and Release Hold

- Create this sprint and link older hotlists as evidence.
- Add guardrail notes to older trackers so work resumes here first.
- Snapshot dirty tree and branch before implementation begins.

### S1. Profile/Config Authority

- Burn `user.toml` support completely.
- Add always-on profile/config linter through `capsem-admin` rails.
- Validate corp, settings, profile catalog, profile files, rules, detection
  YAML, MCP, plugins, assets, manifests, OBOM pins, bootstrap root files.
- Add adversarial tests for malformed profiles, invalid rule roots, missing
  profile files, stale hashes, and forbidden legacy config paths.

### S2. Materialization, Assets, VM Resources

- Make `just _materialize-config` materialize every checked-in profile.
- Ensure `code` and `co-work` do not clobber one another.
- Verify file:// and remote manifest paths use the same downloader/status path.
- Prove profile VM resources apply to new sessions: CPU, RAM, scratch disk.
- Add doctor/status/debug evidence for guest disk, host sparse image, inode and
  host filesystem pressure.
- Add bounded write/package-manager probes for `/usr/local`, `/var/cache/apt`,
  `/tmp`, `/var/tmp`, `/root`.

### S3. Route Contract and API Coverage

- Define route inventory before UI changes.
- Add contract tests for every UI/TUI route for each materialized profile.
- Remove 404/501 surfaces.
- Session routes use session state enum and expose only valid actions.
- Profile routes expose info/status/edit/reload where meaningful, not magic
  global routes.

### S4. Hermetic Protocol Lab and Recorder

- Build one local protocol lab shared by doctor, tests, recorder, and bench.
- Cover HTTP, HTTPS/MITM, gzip, chunked, SSE, WebSocket, DNS, MCP, model
  protocols, OAuth/OIDC, and broker flows.
- Add recorder/replay corpus for Claude/Anthropic, OpenAI/Codex-compatible,
  Gemini/AGY-compatible, MCP JSON-RPC, and credential flows.
- Local Ollama is a host/lab backend, not a guest install requirement. The
  current developer baseline is `gemma4:latest` on `127.0.0.1:11434`; tests
  must route to it through Capsem-owned host aliasing so the ledger sees normal
  network/MITM/model traffic.

### S5. Doctor, Just, E2E, Benchmark

- Fold benchmark-only local server modes into the standard benchmark tool.
- Remove release `--fast` paths.
- `just smoke` and `just test` run doctor, integration, package, install, and
  benchmark gates appropriate for release.
- Benchmarks use scaled concurrency/request counts and emit report artifacts
  Linux can reproduce.

### S6. CEL and Security Event Contract

- Add first-party `ip`, `tcp`, and `udp` CEL facts.
- Add `valid` booleans consistently at family and subobject levels.
- Remove `security.*` as rule predicate input.
- Add `disable` rule action if the rule contract needs route-backed disabled
  rules.
- Add default local/private network guard rules and explicit Ollama/local
  backend allow/ask/block/disable profile rules.
- Re-audit existing Ollama/default-provider rules so localhost/private network
  access is not broadly allowed by accident. Ollama approval must be an
  explicit profile rule that can be toggled `allow`, `ask`, `block`, or
  `disable`.

### S7. Runtime Protocol Fixes

- Fix AGY/Gemini SSE and Google internal endpoints.
- Fix Claude/Anthropic streaming, headers, and EOF/hyper errors.
- Separate tool declarations from executed tool calls.
- Detect unknown AI-compatible protocol shapes on unknown hosts.
- Detect unknown remote MCP and promote it to route-visible profile evidence.
- Prove broker capture/broker/inject across OAuth, headers, query params,
  cookies, body tokens, config files, env-style files, and MCP/tool configs.

### S8. UI/TUI Contract Repair

- Rename user-facing VMs to sessions.
- Profile cards show profile icon/name/description/readiness from profile
  routes, with `New` and `Customize`.
- Incompatible/defunct sessions are greyed and expose only valid actions.
- Profile settings use select boxes for profile lists/enums.
- Enforcement/detection/plugins/MCP/assets routes render complete contracts.
- Detail panels render one canonical view; raw JSON is debug-only.
- Payload rendering uses content type/mimetype/parser state and syntax
  highlighting.

### S9. Agent Bootstrap Repair

- Profile root contains non-secret bootstrap for AGY, Claude, Codex, MCP,
  aliases/wrappers, tips, and approved local configuration.
- Do not bake OAuth tokens, logs, conversations, history, lock files, or caches.
- Claude MCP approval and dangerous-mode acknowledgement are profile-owned.
- AGY alias/wrapper and config are profile-owned.
- Codex config/MCP compatibility is profile-owned.

### S10. Packaging, Install, Docs, Release Gate

- `.pkg` and `.deb` payload tests enforce closed contract.
- Package accepts local or remote manifest override and records origin/hash.
- `just install` builds CI-like package and installs through the package path.
- Status/debug report manifest origin/hash, service version, profile status,
  plugin status, route status, doctor evidence, OBOM/SBOM references.
- Changelog, docs, skills, and release benchmark page are updated.

## Testing Proof Matrix

| Area | Unit/Contract | Functional | Adversarial | E2E/VM | Observability | Performance |
| --- | --- | --- | --- | --- | --- | --- |
| Profile config | schema/linter | materialize profiles | malformed/stale hashes | install + service status | structured lint errors | lint fast |
| Routes | route inventory tests | UI/TUI route calls | missing profile/bad ids | installed app smoke | gateway logs | route latency |
| Security/CEL | compiler/evaluator | allow/ask/block/rewrite | invalid roots/self-decision | doctor requests | ledger rows | CEL timing |
| Protocols | parsers/fixtures | lab request/replay | malformed/truncated streams | VM doctor | DB/log rows | concurrent bench |
| Broker | plugin unit tests | capture/inject/replay | raw secret leak attempts | OAuth lab | broker events/counters | per-plugin latency |
| Package | payload tests | install package | asset blob included | `just install` | install log/hash | install timing |
| UI/TUI | component/route tests | app smoke | 404/501/disabled states | installed UI manual | visible debug/status | render not primary |

## Done

- Every slice in `tracker.md` is checked with proof commands/evidence.
- No release holds remain.
- `just test`, `just smoke`, `just install`, doctor, and benchmark gates pass.
- Changelog/docs/skills reflect the final contract.
- Branch is committed and pushed with clean sprint docs.
