# Security Engine Benchmarks

This directory stores committed Security Engine benchmark artifacts.

Artifacts currently cover two lanes:

- host-side Rust Criterion microbenchmarks for canonical CEL paths in
  `capsem-security-engine`;
- host-side serial pytest runs that exercise VM-originated Security Engine
  events through the real service/process IPC path and verify session DB,
  runtime counters, and log projection.

The Criterion numbers explain evaluator, policy-context materialization,
rule-count, and native lookup costs across commits. The serial pytest numbers
are the first product-path latency artifacts and are appropriate for
engineering regression tracking when quoted with their workload and host.

## Run

```bash
cargo bench -p capsem-security-engine --bench security_engine_cel
uv run pytest tests/capsem-serial/test_security_engine_benchmark.py -xvs
```
