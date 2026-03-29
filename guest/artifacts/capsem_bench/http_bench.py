"""HTTP throughput benchmarks (ab-style concurrent GETs through MITM proxy)."""

import time
from concurrent.futures import ThreadPoolExecutor, as_completed

from rich.table import Table
from rich.text import Text

from .helpers import (
    DEFAULT_HTTP_C, DEFAULT_HTTP_N, DEFAULT_HTTP_URL,
    console, fmt_bytes, percentile,
)


def do_request(url, session):
    """Make a single HTTP GET and return timing + status."""
    start = time.monotonic()
    try:
        resp = session.get(url, timeout=30)
        elapsed = time.monotonic() - start
        return {
            "status": resp.status_code,
            "size": len(resp.content),
            "latency_ms": elapsed * 1000,
            "error": None,
        }
    except Exception as e:
        elapsed = time.monotonic() - start
        return {
            "status": 0,
            "size": 0,
            "latency_ms": elapsed * 1000,
            "error": str(e),
        }


def http_bench(url=None, total_requests=None, concurrency=None):
    """Run HTTP benchmarks (ab-style concurrent GETs)."""
    import requests as req

    url = url or DEFAULT_HTTP_URL
    total_requests = total_requests or DEFAULT_HTTP_N
    concurrency = concurrency or DEFAULT_HTTP_C

    all_results = []

    def worker(n_requests):
        session = req.Session()
        worker_results = []
        for _ in range(n_requests):
            worker_results.append(do_request(url, session))
        session.close()
        return worker_results

    per_worker = total_requests // concurrency
    remainder = total_requests % concurrency

    wall_start = time.monotonic()

    with ThreadPoolExecutor(max_workers=concurrency) as pool:
        futures = []
        for i in range(concurrency):
            n = per_worker + (1 if i < remainder else 0)
            if n > 0:
                futures.append(pool.submit(worker, n))

        for f in as_completed(futures):
            all_results.extend(f.result())

    wall_time = time.monotonic() - wall_start

    latencies = sorted(r["latency_ms"] for r in all_results)
    successful = sum(
        1 for r in all_results
        if r["error"] is None and 200 <= r["status"] < 400
    )
    failed = total_requests - successful
    total_bytes = sum(r["size"] for r in all_results)

    stats = {
        "url": url,
        "total_requests": total_requests,
        "concurrency": concurrency,
        "successful": successful,
        "failed": failed,
        "total_duration_ms": round(wall_time * 1000, 1),
        "requests_per_sec": round(total_requests / wall_time, 1) if wall_time > 0 else 0,
        "transfer_bytes": total_bytes,
        "latency_ms": {
            "min": round(latencies[0], 1) if latencies else 0,
            "max": round(latencies[-1], 1) if latencies else 0,
            "mean": round(sum(latencies) / len(latencies), 1) if latencies else 0,
            "p50": round(percentile(latencies, 50), 1),
            "p95": round(percentile(latencies, 95), 1),
            "p99": round(percentile(latencies, 99), 1),
        },
    }

    table = Table(title=Text(f"HTTP Benchmark  [{url}]"))
    table.add_column("Metric", style="bold")
    table.add_column("Value", justify="right")

    table.add_row("Requests", f"{successful}/{total_requests}")
    table.add_row("Concurrency", str(concurrency))
    table.add_row("Requests/sec", f"{stats['requests_per_sec']}")
    table.add_row("Transfer", fmt_bytes(total_bytes))
    table.add_row("Duration", f"{stats['total_duration_ms']} ms")

    lat = stats["latency_ms"]
    table.add_section()
    table.add_row("Latency min", f"{lat['min']} ms")
    table.add_row("Latency mean", f"{lat['mean']} ms")
    table.add_row("Latency p50", f"{lat['p50']} ms")
    table.add_row("Latency p95", f"{lat['p95']} ms")
    table.add_row("Latency p99", f"{lat['p99']} ms")
    table.add_row("Latency max", f"{lat['max']} ms")

    if failed:
        table.add_section()
        errors = {}
        for r in all_results:
            if r["error"]:
                errors[r["error"]] = errors.get(r["error"], 0) + 1
        for err, count in errors.items():
            table.add_row("Error", f"{err} (x{count})")

    console.print(table)
    return stats
