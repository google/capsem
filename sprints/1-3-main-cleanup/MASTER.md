# 1.3 Main Cleanup Sprint

## Status

| Slice | Status | Release Hold | Notes |
| --- | --- | --- | --- |
| T0 sprint + changelog audit | Complete | Yes | Changelog currently overclaims unified runtime enforcement. |
| T1 compression/install/setup cleanup | Complete | Yes | lz4hc level 12, setup cleanup, default plugin policy examples, and plugin UI controls are done. |
| T2 single security-engine runtime rail | Complete | Yes | Old PolicyHook/Policy V2/MCP decision/provider rails removed; HTTP/model/MCP/DNS enforcement now goes through `SecurityEvent` + CEL. |
| T3 docs + default templates + benchmarks | In Progress | Yes | Benchmark page and release skill updated; changelog final pass remains. |
| T4 smoke/tests/CI readiness | In Progress | Yes | Focused Rust/frontend gates passed; `just smoke`, `just test`, and fresh benchmark artifacts remain. Linux-only failures can be triaged Monday. |

## Release Contract

- Main is the 1.3 truth branch.
- EROFS rootfs compression default is `lz4hc` level `12`.
- Zstd remains supported for experiments, but it is not the 1.3 default because macOS and Linux benchmark evidence showed it was not worth the trade-off for Capsem's speed-first release target.
- Runtime enforcement/detection uses one path: normalized `SecurityEvent` -> one CEL-backed `SecurityRuleSet::evaluate` -> plugin/action materialization -> one DB writer ledger.
- No setup wizard or `capsem-setup` authority path remains in product docs, defaults, install flow, or endpoints.
- Plugin policy is visible in default templates, with built-in defaults documented.
- Plugin policy is visible in the UI with enum-backed `mode` and
  `detection_level` selects.
- Changelog claims must be backed by code and tests.

## Final Gates

- Focused unit/contract tests for each changed slice.
- `uv run pytest tests/capsem-security/test_detection_yaml.py -q`
- Service endpoint tests for assets/plugins/enforcement.
- Security-engine tests proving single evaluator behavior and detection vector behavior.
- `just smoke`
- `just test`
- Release docs/changelog pass.
- Benchmark artifacts and `docs/src/content/docs/benchmarks/results.md` are updated with current numbers and notes on the EROFS compression decision.

Linux-only failures are release notes for the Linux team only if macOS/main stays clean and the failure is clearly platform-specific.
