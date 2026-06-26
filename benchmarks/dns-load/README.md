# dns-load baseline

Locked output of `capsem-bench dns-load` captured during T3
closure (mitm-redesign sprint, T3.4). This historical baseline used
`api.openai.com` with active profile rules allowing it, so the first
query went through upstream forwarding and subsequent queries hit the
answer cache.

The current default benchmark target is `load-test.capsem-bogus`.
That name is a Capsem-local bogus TLD fixture: the host DNS handler
returns local NXDOMAIN after security evaluation and before resolver
forwarding. Use this target for route/proxy latency gates because it is
self-contained and cannot be polluted by external resolver behavior.
Set `CAPSEM_BENCH_DNS_QNAME=api.openai.com` only when intentionally
benchmarking the upstream/cache path.

| concurrency | rps   | p50 ms | p99 ms | errors |
|-------------|-------|--------|--------|--------|
| 1           |  3556 |   0.3  |   0.5  |  0     |
| 10          | 12928 |   0.7  |   1.1  |  0     |
| 50          | 12425 |   4.0  |   4.9  |  0     |
| 200         | 11482 |  16.5  |  26.7  |  0     |

The historical baseline below was captured with **debug-build** host binaries on
an Apple M-series host. To re-baseline with release builds,
sign the `assets/manifest.json` with the release minisign key
(release builds hard-fail without the signature) and re-run
`capsem-bench dns-load` in a persistent VM:

```sh
just install                 # produces signed pkg + installs
capsem create -n bench-vm
capsem exec bench-vm -- 'capsem-bench dns-load'
capsem exec bench-vm -- 'cat /tmp/capsem-benchmark.json' > new-baseline.json
```

## Capture environment

* Host: macOS, Apple M-series
* Build profile: debug
* Hypervisor: Apple Virtualization.framework
* VM: 4 vCPU, 4 GB RAM (capsem create defaults)
* Bench duration: 10s per concurrency level (CAPSEM_BENCH_DNS_DURATION default)

## Regression policy

Per the mitm-redesign sprint discipline:
* >2x p99 regression at any concurrency level fails the closure gate
* >50% rps drop at any concurrency level fails the closure gate
* Any non-zero `transport_error` count outside the upstream-failure
  shape is a real bug (see T3 closure commit `c7f9898` for the
  cache qid bug that caused 100% errors before the fix)

The decision distribution must match what the target says. For the
default `load-test.capsem-bogus` fixture, expect
`decision_distribution = {"denied": N}` and `errors = 0`. For an
allowed upstream/cache target such as `api.openai.com`, expect
`{"allowed": N}` once the active profile allows it. Any
`transport_error` > 0 outside that shape is a real proxy bug, not bench
noise.
