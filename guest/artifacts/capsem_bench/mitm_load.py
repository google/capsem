"""mitm-load: concurrency-driven load test against the MITM proxy.

Measures rps + tail latency (p50/p95/p99/p99.9) at multiple concurrency
levels so the T0 -> T5 redesign has a concrete regression baseline for
"pipeline overhead under load," "lock contention on cert mint," and
"telemetry path saturation." The target is a fixture host the benchmark
collector resolves and serves through capsem-mock-server on the host, so
every request traverses guest DNS, capsem-net-proxy, the host MITM (TLS
termination, SNI, policy, cert mint, telemetry) and a real, fast upstream.

It used to target a non-existent public domain "so every request fails at
the upstream dial". The guest's resolver answered NXDOMAIN and the client
failed before opening a socket, so the proxy never ran and the recorded
numbers were DNS-failure numbers. A level where every request fails is now
an error, not a measurement.

Output schema:

  {
    "version": "1.0",
    "target": "https://fixture.capsem.test/tiny",
    "concurrency_levels": [
      {
        "concurrency": 1,
        "duration_s": 30.0,
        "total_requests": 1234,
        "errors": 0,
        "rps": 41.1,
        "p50_ms": 22.0,
        "p95_ms": 35.0,
        "p99_ms": 41.0,
        "p999_ms": 70.0,
        "rss_peak_mb": 132.0
      },
      ...
    ]
  }

CI gate (T5): >2x p99 regression vs. baseline at any concurrency
level fails the build.
"""

import os
import time
from concurrent.futures import ThreadPoolExecutor, as_completed

from .helpers import console
from .load_harness import (
    DurationLoadConfig,
    render_load_table,
    summarize_load_level,
)

# Served by capsem-mock-server through the benchmark collector's corp config
# (DNS fixture + upstream override), configured by the shared guest collector.
DEFAULT_TARGET = "https://fixture.capsem.test/tiny"
DEFAULT_CONCURRENCY = (1, 10, 50, 200)
DEFAULT_DURATION_S = 10.0


def _do_request(url, session):
    """Single HTTP GET; latency in ms, no body assertions."""
    start = time.monotonic()
    try:
        resp = session.get(url, timeout=30)
        elapsed_ms = (time.monotonic() - start) * 1000
        return (elapsed_ms, resp.status_code, None)
    except Exception as exc:
        elapsed_ms = (time.monotonic() - start) * 1000
        return (elapsed_ms, 0, str(exc))


def _drive_at_concurrency(url, concurrency, duration_s):
    """Spawn `concurrency` workers, each looping `duration_s`.

    Each worker holds its own requests Session so connection-pool
    behavior matches a real client. Returns a list of (latency_ms,
    status, error) tuples.
    """
    import requests as req

    deadline = time.monotonic() + duration_s

    def worker():
        session = req.Session()
        out = []
        while time.monotonic() < deadline:
            out.append(_do_request(url, session))
        session.close()
        return out

    all_results = []
    with ThreadPoolExecutor(max_workers=concurrency) as pool:
        futures = [pool.submit(worker) for _ in range(concurrency)]
        for fut in as_completed(futures):
            all_results.extend(fut.result())
    return all_results


def _summarize(results, concurrency, duration_s):
    """Build the JSON-shaped row for this concurrency level."""
    latencies = sorted(r[0] for r in results)
    errors = sum(1 for r in results if r[2] is not None)
    return summarize_load_level(latencies, errors, concurrency, duration_s)


def mitm_load_bench(target=None, concurrency_levels=None, duration_s=None):
    """Drive the MITM proxy at each concurrency level; return the result dict."""
    target = target or os.environ.get("CAPSEM_BENCH_MITM_TARGET", DEFAULT_TARGET)
    config = DurationLoadConfig.from_inputs(
        "mitm-load",
        default_concurrency=DEFAULT_CONCURRENCY,
        default_duration_s=DEFAULT_DURATION_S,
        concurrency_levels=concurrency_levels,
        duration_s=duration_s,
    )

    console.print(
        f"[bold]mitm-load[/bold] target={target} duration={config.duration_s}s "
        f"concurrency={','.join(str(c) for c in config.concurrency_levels)}"
    )

    rows = []
    for c in config.concurrency_levels:
        console.print(f"  concurrency={c} ...")
        results = _drive_at_concurrency(target, c, config.duration_s)
        row = _summarize(results, c, config.duration_s)
        if row["total_requests"] and row["errors"] >= row["total_requests"]:
            first_error = next((r[2] for r in results if r[2] is not None), "unknown")
            raise RuntimeError(
                f"mitm-load: every request at concurrency {c} failed ({first_error}); "
                "the proxy was not measured"
            )
        rows.append(row)

    out = {
        "version": "1.0",
        "target": target,
        "concurrency_levels": rows,
    }

    render_load_table(
        f"mitm-load (target={target}, {config.duration_s}s per level)",
        rows,
    )

    return out
