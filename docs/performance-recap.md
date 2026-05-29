# Plugin Runtime Benchmark

Recorded on 2026-05-21 in the isolated Rust plugin-engine prototype.

## Question

Does adding the plugin execution safety wall materially slow the Rust WASM hot
path?

Compiler numbers are a separate experiment because they measure install-time
source compilation, not callback throughput. They live in
[`compiler-benchmark.md`](compiler-benchmark.md).

## Benchmark

Same benchmark shape for every row:

- Rust host.
- No HTTP.
- One installed plugin.
- `25,000` callback executions.
- Same tiny object/context ABI fixture.
- Release build.

All time values are milliseconds. `ms/call` is normalized from total wall time.

| Runtime | Calls | Compile ms | Load ms | Run wall ms | ms/call | p50 ms | p95 ms | QPS | QPS delta |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Baseline | `25,000` | `3.766792` | `2.975375` | `412.549208` | `0.016502` | `0.014041` | `0.022958` | `60,599` | baseline |
| Baseline + CPU rate limit | `25,000` | `3.652708` | `3.003666` | `424.932625` | `0.016997` | `0.014583` | `0.023583` | `58,833` | `-2.9%` |
| Baseline + CPU rate limit + memory/table limits | `25,000` | `3.857583` | `2.817000` | `426.852750` | `0.017074` | `0.014666` | `0.023291` | `58,568` | `-3.4%` |

## Conclusion

Keep the safety wall. The fully limited Rust runtime still runs about `58.6k`
callbacks/sec, with p50 at `0.014666ms` and only about `3.4%` QPS overhead
versus the unlimited baseline.

## Command

```bash
CAPSEM_BENCH_ENGINE=wasmtime-wat-fuel \
CAPSEM_BENCH_REPEATS=1000 CAPSEM_BENCH_BATCH_RUNS=25 \
  cargo run -p capsem-plugin-engine --release --example benchmark
```

For the unlimited baseline, omit `CAPSEM_BENCH_ENGINE`.
