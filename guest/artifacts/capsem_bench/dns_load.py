"""DNS proxy load through the async Rust benchmark driver."""

import json
import os
import subprocess

from .helpers import console
from .load_harness import (
    DurationLoadConfig,
    capacity_concurrency_levels,
    render_load_table,
)

DEFAULT_QNAME = "load-test.capsem-bogus"
DEFAULT_QTYPE = 1
DEFAULT_DURATION_S = 10.0
DEFAULT_TIMEOUT_S = 5.0
RUST_BENCH = "/usr/local/bin/capsem-bench-rs"


def _drive_at_concurrency(qname, qtype, concurrency, duration_s, timeout_s):
    """Run fixed async UDP clients and validate that every client ran."""
    completed = subprocess.run(
        [
            RUST_BENCH,
            "dns-load",
            "--target",
            "127.0.0.1:53",
            "--qname",
            qname,
            "--qtype",
            str(qtype),
            "--concurrency",
            str(concurrency),
            "--duration-s",
            str(duration_s),
            "--timeout-ms",
            str(round(timeout_s * 1000)),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode:
        raise RuntimeError(
            f"dns-load: Rust load engine exited {completed.returncode}: "
            f"{completed.stderr[-2000:].strip()}"
        )
    try:
        row = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError("dns-load: Rust load engine returned invalid JSON") from exc
    if row.get("concurrency") != concurrency:
        raise RuntimeError("dns-load: Rust load engine reported the wrong concurrency")
    if row.get("total_requests", 0) < concurrency:
        raise RuntimeError(
            f"dns-load: only {row.get('total_requests', 0)} of {concurrency} "
            "client loops made a request; the load generator was starved"
        )
    return row


def dns_load_bench(qname=None, qtype=None, concurrency_levels=None, duration_s=None):
    """Drive the DNS proxy at each concurrency level; return the result dict."""
    qname = qname or os.environ.get("CAPSEM_BENCH_DNS_QNAME", DEFAULT_QNAME)
    qtype = qtype or int(os.environ.get("CAPSEM_BENCH_DNS_QTYPE", DEFAULT_QTYPE))
    timeout_s = float(os.environ.get("CAPSEM_BENCH_DNS_TIMEOUT", DEFAULT_TIMEOUT_S))
    default_concurrency = capacity_concurrency_levels() if concurrency_levels is None else ()
    config = DurationLoadConfig.from_inputs(
        "dns-load",
        default_concurrency=default_concurrency,
        default_duration_s=DEFAULT_DURATION_S,
        concurrency_levels=concurrency_levels,
        duration_s=duration_s,
    )

    console.print(
        f"[bold]dns-load[/bold] qname={qname} qtype={qtype} "
        f"duration={config.duration_s}s "
        f"concurrency={','.join(str(c) for c in config.concurrency_levels)}"
    )

    rows = []
    for concurrency in config.concurrency_levels:
        console.print(f"  concurrency={concurrency} ...")
        row = _drive_at_concurrency(
            qname,
            qtype,
            concurrency,
            config.duration_s,
            timeout_s,
        )
        rows.append(row)
        if row["errors"]:
            # Preserve the failed row and every lower successful level in the
            # JSON artifact. The host collector rejects non-zero errors after
            # recording the structured RPS, latency and pressure evidence.
            break

    out = {
        "version": "1.3",
        "qname": qname,
        "qtype": qtype,
        "concurrency_levels": rows,
    }
    render_load_table(
        f"dns-load (qname={qname}, qtype={qtype}, {config.duration_s}s per level)",
        rows,
        extra_columns=[
            (
                "decisions",
                lambda row: ",".join(
                    f"{key}={value}"
                    for key, value in row["decision_distribution"].items()
                ),
            ),
        ],
    )
    return out
