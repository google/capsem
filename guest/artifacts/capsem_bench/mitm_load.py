"""mitm-load: concurrency-driven load test against the MITM proxy.

Measures rps + tail latency (p50/p95/p99/p99.9) at multiple concurrency
levels so the T0 -> T5 redesign has a concrete regression baseline for
"pipeline overhead under load," "lock contention on cert mint," and
"telemetry path saturation." Only the host MITM is exercised --
upstream is a single non-routable domain so every request fails fast
at the upstream-dial stage with the proxy doing all the work
(SNI parse + policy check + cert mint + connect attempt + telemetry
emission). That isolates the proxy's cost from upstream variance.

Output schema:

  {
    "version": "1.0",
    "target": "https://thisdomaindoesnotexistforsur3.ai/",
    "concurrency_levels": [
      {
        "concurrency": 1,
        "duration_s": 30.0,
        "total_requests": 1234,
        "errors": 1234,         # all requests "fail" -- non-routable upstream
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
import resource
import time
from concurrent.futures import ThreadPoolExecutor, as_completed

from rich.table import Table

from .helpers import console, percentile

# Non-routable domain so every request resolves to the upstream-dial
# failure path -- isolates the proxy's per-request cost from real
# upstream variance.
DEFAULT_TARGET = "https://thisdomaindoesnotexistforsur3.ai/"
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
    if not results:
        return {
            "concurrency": concurrency,
            "duration_s": duration_s,
            "total_requests": 0,
            "errors": 0,
            "rps": 0.0,
            "p50_ms": 0.0,
            "p95_ms": 0.0,
            "p99_ms": 0.0,
            "p999_ms": 0.0,
        }
    latencies = sorted(r[0] for r in results)
    errors = sum(1 for r in results if r[2] is not None)
    return {
        "concurrency": concurrency,
        "duration_s": duration_s,
        "total_requests": len(results),
        "errors": errors,
        "rps": len(results) / duration_s,
        "p50_ms": percentile(latencies, 50),
        "p95_ms": percentile(latencies, 95),
        "p99_ms": percentile(latencies, 99),
        "p999_ms": percentile(latencies, 99.9),
    }


def _peak_rss_mb():
    """Peak RSS of this process in MB."""
    ru = resource.getrusage(resource.RUSAGE_SELF)
    # Linux: ru_maxrss is in KB. macOS: bytes. We're in-VM (Linux),
    # so KB is right.
    return ru.ru_maxrss / 1024.0


def mitm_load_bench(target=None, concurrency_levels=None, duration_s=None):
    """Drive the MITM proxy at each concurrency level; return the result dict."""
    target = target or os.environ.get("CAPSEM_BENCH_MITM_TARGET", DEFAULT_TARGET)
    concurrency_levels = concurrency_levels or DEFAULT_CONCURRENCY
    duration_s = duration_s or float(
        os.environ.get("CAPSEM_BENCH_MITM_DURATION", DEFAULT_DURATION_S)
    )

    console.print(f"[bold]mitm-load[/bold] target={target} duration={duration_s}s")

    rows = []
    for c in concurrency_levels:
        console.print(f"  concurrency={c} ...")
        results = _drive_at_concurrency(target, c, duration_s)
        row = _summarize(results, c, duration_s)
        row["rss_peak_mb"] = _peak_rss_mb()
        rows.append(row)

    out = {
        "version": "1.0",
        "target": target,
        "concurrency_levels": rows,
    }

    # Human-readable table to stderr.
    table = Table(title=f"mitm-load (target={target}, {duration_s}s per level)")
    table.add_column("concurrency", justify="right")
    table.add_column("rps", justify="right")
    table.add_column("p50_ms", justify="right")
    table.add_column("p95_ms", justify="right")
    table.add_column("p99_ms", justify="right")
    table.add_column("p999_ms", justify="right")
    table.add_column("errors", justify="right")
    for row in rows:
        table.add_row(
            str(row["concurrency"]),
            f"{row['rps']:.1f}",
            f"{row['p50_ms']:.1f}",
            f"{row['p95_ms']:.1f}",
            f"{row['p99_ms']:.1f}",
            f"{row['p999_ms']:.1f}",
            str(row["errors"]),
        )
    console.print(table)

    return out
