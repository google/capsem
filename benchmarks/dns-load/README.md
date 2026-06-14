# dns-load baseline

Locked output of `capsem-bench dns-load` captured during T3
closure (mitm-redesign sprint, T3.4). The baseline represents the
expected steady-state of the capsem DNS proxy serving the default
qname (`api.openai.com`) with the active profile rules allowing it
(so every query goes through the upstream-forward path -> answer
cache hot loop, which is the dominant in-agent workload).

| concurrency | rps   | p50 ms | p99 ms | errors |
|-------------|-------|--------|--------|--------|
| 1           |  3556 |   0.3  |   0.5  |  0     |
| 10          | 12928 |   0.7  |   1.1  |  0     |
| 50          | 12425 |   4.0  |   4.9  |  0     |
| 200         | 11482 |  16.5  |  26.7  |  0     |

The baseline was captured with **debug-build** host binaries on
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

The decision distribution must match what the policy says: if
the active profile allows `api.openai.com`, every row should be
`decision_distribution = {"allowed": N}`. If the profile or corp
rules block it, expect `{"denied": N}`. Any `transport_error` > 0
outside that shape is a real proxy bug, not bench noise.
