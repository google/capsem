---
name: admin-security
description: Capsem admin policy and detection pack validation. Use this whenever the user edits policy packs, detection packs, CEL enforcement rules, Sigma imports, pySigma validation, Detection IR, capsem-admin policy/detection commands, or asks how enforcement and detection artifacts flow through profiles.
---

# Admin Security

Use this skill for profile-owned enforcement and detection content. Policy packs
drive blocking/asking/rewrite decisions; detection packs generate findings.
Sigma is an import format for detection, not a blocking policy language.

## Ground Rules

- Keep enforcement and detection separate in schemas, docs, and runtime paths.
- Use real CEL for policy expressions and real pySigma for Sigma parsing. Do
  not hand-roll validators for either language.
- Compile supported detection content to typed `capsem.detection.ir.v1` before
  runtime consumption.
- Unsupported Sigma constructs must fail closed with a clear admin error.
- Detection results must be available before audit logging, telemetry, and other
  sinks persist the resolved security event.

## First Files To Read

- `src/capsem/builder/security_packs.py`
- `crates/capsem-core/src/security_packs.rs`
- `schemas/capsem.policy-pack.v1.schema.json`
- `schemas/capsem.detection-pack.v1.schema.json`
- `schemas/capsem.detection.ir.v1.schema.json`
- `docs/src/content/docs/security/enforcement.md`
- `docs/src/content/docs/security/detection.md`

## Admin CLI Surface

Use these commands when working on policy or detection packs:

```bash
uv run capsem-admin policy schema
uv run capsem-admin policy validate security/policy-pack.toml
uv run capsem-admin detection schema
uv run capsem-admin detection validate security/detection-pack.yml
uv run capsem-admin detection compile security/detection-pack.yml --out /tmp/detection-ir.json
uv run capsem-admin detection check /tmp/detection-ir.json --events tests/fixtures/security-events.jsonl
```

## Testing Checklist

- Prove invalid policy/detection pack payloads fail at Pydantic boundaries.
- Prove pySigma rejects malformed or unsupported Sigma before IR emission.
- Prove Python-generated Detection IR matches Rust serde/evaluator fixtures.
- Prove detection checks run against normalized SecurityEvent JSONL fixtures.
- Prove docs keep Sigma as detection-only and CEL as enforcement policy.

Useful focused gates:

```bash
uv run python -m pytest tests/test_security_packs.py tests/test_admin_cli.py -q
cargo test -p capsem-core --test security_packs
cargo clippy -p capsem-core --test security_packs -- -D warnings
```
