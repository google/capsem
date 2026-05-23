---
title: Rule Corpus Workflow
description: How enforcement and detection fixtures stay aligned across admin tooling and Rust runtime tests.
sidebar:
  order: 28
---

The rule corpus is the shared test ledger for Capsem enforcement and
detection. It prevents `capsem-admin`, Detection IR, Rust CEL evaluation, and
expected backtest output from drifting apart.

## Layout

| Path | Purpose |
|---|---|
| `data/policy-context/canonical-policy-contexts.jsonl` | Typed policy-context event fixtures. |
| `data/policy-context/session-*.jsonl` | Stable session-export fixtures captured from the installed-service policy-context export shape. |
| `data/enforcement/cel/` | CEL conditions consumed by Rust runtime tests. |
| `data/enforcement/policy/` | Policy pack fixtures consumed by `capsem-admin`. |
| `data/enforcement/backtest-expected/` | Expected enforcement backtest reports without timing fields. |
| `data/detection/sigma/` | Sigma-backed detection pack fixtures. |
| `data/detection/ir/` | Compiled `capsem.detection.ir.v1` fixtures. |
| `data/detection/backtest-expected/` | Expected detection backtest reports without timing fields. |
| `data/detection/hunt-expected/` | Expected session-backed detection hunt reports and projection-path summaries. |

Policy-context fixtures must use canonical roots such as
`http.request.host`, `http.request.header("authorization").exists()`, and
`http.request.body.text`. Internal `event.*` and legacy `subject.*` paths are
test failures. Unknown canonical-looking paths and cross-family roots are also
test failures: the admin policy compiler has an explicit family-scoped
allowlist, so a typo like `http.request.raw` must fail before replay.

## Update Order

1. Add or edit policy-context rows in
   `data/policy-context/canonical-policy-contexts.jsonl`.
2. Update enforcement CEL and policy packs together:

   ```bash
   uv run capsem-admin policy compile data/enforcement/policy/http-google-secret-policy.toml --json
   uv run capsem-admin policy backtest data/enforcement/policy/http-google-secret-policy.toml --events data/policy-context/canonical-policy-contexts.jsonl --json
   ```

3. Update detection Sigma and Detection IR together:

   ```bash
   uv run capsem-admin detection compile data/detection/sigma/google-secret-egress.yml
   uv run capsem-admin detection backtest data/detection/sigma/google-secret-egress.yml --events data/policy-context/canonical-policy-contexts.jsonl --json
   ```

4. Refresh the matching expected artifacts under
   `data/enforcement/backtest-expected/` and
   `data/detection/backtest-expected/`. If the change affects session-backed
   forensic search, refresh `data/detection/hunt-expected/` as well.
5. When a real VM/session behavior should graduate into the corpus, export the
   installed service's typed policy contexts:

   ```bash
   capsem export-policy-contexts <session-id> > data/policy-context/<name>.jsonl
   capsem export-policy-contexts <session-id> --json
   ```

   The JSONL form is for committed fixture rows. The `--json` form keeps the
   export envelope with `fixture_count` for local inspection.
6. Run both language gates:

   ```bash
   uv run pytest tests/test_admin_cli.py tests/test_security_packs.py tests/test_admin_docs.py tests/test_admin_hygiene.py -q
   cargo test -p capsem-core --test security_packs
   cargo test -p capsem-security-engine
   ```

## Rules

`capsem-admin` works offline. It validates public pack schemas, compiles the
admin-supported policy subset, compiles Sigma with pySigma into Detection IR,
and replays fixtures. It is not a substitute for the installed service's
runtime rule registry.

Rust runtime tests remain the authority for CEL semantics. When a new CEL
construct is added, add the fixture first, then add the Rust parity assertion,
then decide whether the offline admin subset should support it or reject it
with a clear diagnostic.

Expected artifacts omit timing so they stay deterministic. Keep event ids,
session ids, rule ids, pack ids, decisions, findings, and matched fields exact.
If the expected row changes, both the Python and Rust tests must explain why.
