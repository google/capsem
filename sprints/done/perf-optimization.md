# Performance Optimization Sprint

Started 2026-04-09. Goal: all VM lifecycle operations under 1.2s.

## Benchmark (measured 2026-04-09)

```
provision:   24ms mean   PASS
exec_ready: 2349ms mean  FAIL  (cold=6286ms, warm=380ms)
exec:         22ms mean  PASS
delete:     5030ms mean  FAIL  (5s every run)
```

Run: `uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py -xvs`

## Done

- [x] Benchmark infra: `test_lifecycle_benchmark` + `test_fork_benchmark` in `tests/capsem-serial/test_lifecycle_benchmark.py`
- [x] Updated `skills/dev-benchmark/SKILL.md` and `site/src/content/docs/benchmarks/results.md`
- [x] `capsem-init`: replaced `sleep 1` + `sleep 0.2` + `sleep 0.1` with poll loops
- [x] `capsem-init`: moved `uv venv` to background
- [x] `capsem-process`: consolidated 4+ redundant `load_settings_files()` into one call
- [x] `capsem-process`: deferred initial snapshot to background task
- [x] `capsem-process`: added `quiet random.trust_cpu=1` to kernel cmdline
- [x] `capsem-core/vm/boot.rs`: `create_net_state_with_policy()` + `preloaded_guest_config` param
- [x] `apple_vz/machine.rs`: 5ms poll with `returnAfterSourceHandled=1`, removed `thread::sleep`
- [x] `Cargo.toml`: `[profile.release]` lto=thin, codegen-units=1, strip=symbols

## Blocking

- [ ] **delete is 5 seconds** -- #1 problem. Investigate delete path in `capsem-service`. 5s on every run, warm or cold.
- [ ] **exec_ready cold boot is 6s** -- run 1 on fresh service. Warm runs are 380ms. capsem-init sleep replacements done but cold boot still slow. Investigate what's actually slow (kernel cache? rootfs decompression? something else?).

## Resume prompt

```
We're optimizing capsem VM lifecycle performance. The benchmark at
tests/capsem-serial/test_lifecycle_benchmark.py gates every operation
under 1200ms mean. Provision (24ms) and exec (22ms) pass. Two failures:

1. delete takes 5 seconds every run -- investigate the delete path in
   capsem-service/src/main.rs and capsem-process. Find what blocks.
2. exec_ready cold boot is 6s (warm is 380ms) -- the capsem-init sleep
   replacements are done but didn't help cold boot. Profile what's slow.

Run the benchmark: uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_lifecycle_benchmark -xvs

Sprint status: sprints/next-gen/perf-optimization.md
```
