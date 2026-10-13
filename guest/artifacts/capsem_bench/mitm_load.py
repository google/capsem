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
numbers were DNS-failure numbers. Any failed request now rejects the run.

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
        "error_ratio": 0.0,
        "rps": 41.1,
        "p50_ms": 22.0,
        "p95_ms": 35.0,
        "p99_ms": 41.0,
        "p999_ms": 70.0,
        "response_body_bytes": 29616
      },
      ...
    ]
  }

CI gate (T5): >2x p99 regression vs. baseline at any concurrency
level fails the build.
"""

import json
import os
import subprocess

from .helpers import console
from .load_harness import (
    DurationLoadConfig,
    render_load_table,
)

# Served by capsem-mock-server through the benchmark collector's corp config
# (DNS fixture + upstream override), configured by the shared guest collector.
DEFAULT_TARGET = "https://fixture.capsem.test/tiny"
DEFAULT_CONCURRENCY = (1, 10, 50, 200)
DEFAULT_DURATION_S = 10.0
RUST_BENCH = "/usr/local/bin/capsem-bench-rs"


def _drive_at_concurrency(url, concurrency, duration_s):
    """Run the async Rust load engine and validate that every client ran."""
    completed = subprocess.run(
        [
            RUST_BENCH,
            "http-load",
            "--target",
            url,
            "--concurrency",
            str(concurrency),
            "--duration-s",
            str(duration_s),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode:
        raise RuntimeError(
            f"mitm-load: Rust load engine exited {completed.returncode}: "
            f"{completed.stderr[-2000:].strip()}"
        )
    try:
        row = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError("mitm-load: Rust load engine returned invalid JSON") from exc
    if row.get("concurrency") != concurrency:
        raise RuntimeError("mitm-load: Rust load engine reported the wrong concurrency")
    if row.get("total_requests", 0) < concurrency:
        raise RuntimeError(
            f"mitm-load: only {row.get('total_requests', 0)} of {concurrency} "
            "client loops made a request; the load generator was starved"
        )
    return row


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
        row = _drive_at_concurrency(target, c, config.duration_s)
        if row["errors"]:
            raise RuntimeError(
                f"mitm-load: {row['errors']} of {row['total_requests']} requests failed "
                f"at concurrency {c}: {row['error_distribution']}; "
                "the rail did not sustain the requested load"
            )
        rows.append(row)

    out = {
        "version": "1.1",
        "target": target,
        "concurrency_levels": rows,
    }

    render_load_table(
        f"mitm-load (target={target}, {config.duration_s}s per level)",
        rows,
    )

    return out
