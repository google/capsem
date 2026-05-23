# Security Engine Benchmarks

This directory stores committed Security Engine benchmark artifacts.

The first artifact is a host-side Rust Criterion microbenchmark for canonical
CEL paths in `capsem-security-engine`. It is useful for comparing evaluator,
policy-context materialization, rule-count, and native lookup costs across
commits.

It is not a VM-originated benchmark. Release and marketing claims must wait for
the S08d VM-originated harness that measures the full guest -> service ->
Security Engine -> journal path.

## Run

```bash
cargo bench -p capsem-security-engine --bench security_engine_cel
```

