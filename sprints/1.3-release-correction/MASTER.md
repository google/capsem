# 1.3 Release Correction Sprint

Status: Active planning. No product-code fixes start until this sprint is the
execution ledger.

## Why This Sprint Exists

The 1.3 branch has the right direction, but the release loop exposed a pattern
we must correct before asking for another manual credential/client run: profile
routes are incomplete, some bootstrap/config paths still drift from the profile
contract, protocol tests are too thin, UI surfaces render guesses, and doctor /
bench / smoke do not yet prove the real VM path. This sprint replaces the messy
hotlist with a controlled correction plan and gates.

Manual AGY/Claude/Codex/OAuth runs are forbidden until the local hermetic gates
prove the same rails without user credentials.

## Absolute Contracts

- Profile is the unit of product truth. A session runs a profile.
- Settings are UI/application settings only. They do not decide profile
  behavior.
- Corp owns locked constraints and reporting endpoints.
- Profile owns assets, VM resources, bootstrap root files, enforcement rules,
  detection files, MCP config, plugin config, and surface availability.
- No `user.toml`, no fallback config, no global profile behavior.
- UI/TUI render route contracts. They do not rename profile data or invent
  states.
- The security rail is one CEL/security-event path with typed events and typed
  rule actions.
- Plugins are configured by profile/corp and report structured status/counters.
- Snapshot is a hermetic subsystem surfaced by routes, not a generic activity
  table.
- Doctor, tests, benchmark, and install all use the same manifest/profile/admin
  path.
- Installer packages contain the app/runtime config/manifest provenance, not VM
  asset blobs.

## Status Table

| Slice | Name | Status | Exit Gate |
| --- | --- | --- | --- |
| S0 | Sprint ledger and release hold | Complete | `MASTER.md`, `plan.md`, and `tracker.md` are coherent and linked from old trackers. |
| S1 | Profile/config authority | Planned | `user.toml` rail burned; profile linter always runs; invalid profiles cannot be materialized. |
| S2 | Materialization/assets/resources | Planned | `code` and `co-work` materialize from `capsem-admin`; assets and VM resources verified end to end. |
| S3 | Route contract and API coverage | Planned | Every UI/TUI-used profile/session/stats route has contract tests for both profiles; no 404/501. |
| S4 | Hermetic protocol lab and recorder | Planned | Local lab covers HTTP/HTTPS/SSE/WS/DNS/MCP/model/OAuth/broker without public services. |
| S5 | Doctor/just/benchmark unification | Planned | `just test` and `just smoke` run doctor/E2E/bench through the hermetic lab, no `--fast` release escape. |
| S6 | CEL/security event correction | Planned | IP/TCP/UDP facts and `valid` booleans are first-party CEL objects; no `security.*` predicates. |
| S7 | Runtime protocol fixes | Planned | AGY/Claude/Codex model, MCP, broker, SSE, and tool-call paths pass replay and DB-ledger assertions. |
| S8 | UI/TUI contract repair | Planned | Sessions/profiles/settings/stats/plugin/MCP/security/file/process views reflect routes and enums only. |
| S9 | Agent bootstrap repair | Planned | AGY, Claude, Codex, MCP, aliases, and profile root files are packaged from profile-owned bootstrap. |
| S10 | Packaging/install/release gate | Planned | Package payload closed contract, `just install`, status/debug, changelog/docs, and benchmark report pass. |

## Release Holds

- Hold: no more real OAuth/client manual testing until S1-S7 local gates pass.
- Hold: do not purge or kill user evidence sessions without explicit approval.
- Hold: no old policy/domain/MCP fallback rails may be reintroduced.
- Hold: no package may include rootfs/initrd/kernel asset blobs.
- Hold: no profile route may return 404/501 from installed UI/TUI surfaces.

## Source Evidence

- Active hotlist: `sprints/1.3-debug-loop/current-hotlist.md`
- Historical debug tracker: `sprints/1.3-debug-loop/tracker.md`
- Existing narrow Claude note: `sprints/1.3-claude-mcp-bootstrap/`
- Local baseline confirmed on 2026-06-11: host Ollama is reachable at
  `127.0.0.1:11434`; `/api/tags` reports `gemma4:latest` with completion,
  tools, and thinking capabilities. Use this as the local live backend for
  recorder/smoke tests, routed through Capsem, not as a guest install target.

Those files remain evidence. This sprint is the execution authority.
