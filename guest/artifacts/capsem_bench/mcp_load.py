"""mcp-load: concurrency-driven load test against the guest MCP path.

Drives `local__echo` (the diagnostic builtin tool that returns input
verbatim with zero I/O) at multiple concurrency levels so we can
characterize the MCP transport overhead end-to-end:

    Python fastmcp.Client (in guest)
      -> stdio -> /run/capsem-mcp-server (guest agent's MCP server)
      -> framed MCP over vsock:5002 -> capsem-process MITM MCP endpoint
      -> capsem-mcp-aggregator (host)
      -> stdio -> capsem-mcp-builtin (host subprocess)
      -> echo handler returns the text
      -> back up the chain

Pure protocol cost. If `mcp-load` does NOT scale linearly with
concurrency, we have a serialization bug in the guest relay / MITM
endpoint / aggregator / server / vsock path and the transport needs
attention.
Sister bench to `mitm-load` (which isolates the proxy hot path).
"""

import asyncio
import os
import resource
import time

from fastmcp import Client
from fastmcp.client.transports import StdioTransport
from rich.table import Table

from .helpers import console, percentile

MCP_SERVER = "/run/capsem-mcp-server"
DEFAULT_CONCURRENCY = (1, 10, 50, 200)
DEFAULT_DURATION_S = 10.0
DEFAULT_PAYLOAD = "ping"


async def _drive_at_concurrency(client, concurrency, duration_s, payload):
    """Hold `concurrency` in-flight echo calls for `duration_s`.

    A pool of `concurrency` worker coroutines, each looping
    `client.call_tool(...)` until the deadline. Returns latencies in ms
    (one entry per completed call) plus the error count.
    """
    deadline = time.monotonic() + duration_s
    latencies = []
    errors = 0
    lat_lock = asyncio.Lock()

    async def worker():
        nonlocal errors
        while time.monotonic() < deadline:
            t0 = time.monotonic()
            try:
                await client.call_tool("local__echo", {"text": payload})
                ms = (time.monotonic() - t0) * 1000
                async with lat_lock:
                    latencies.append(ms)
            except Exception:
                errors += 1

    await asyncio.gather(*(worker() for _ in range(concurrency)))
    return latencies, errors


def _summarize(latencies, errors, concurrency, duration_s):
    if not latencies:
        return {
            "concurrency": concurrency,
            "duration_s": duration_s,
            "total_requests": 0,
            "errors": errors,
            "rps": 0.0,
            "p50_ms": 0.0,
            "p95_ms": 0.0,
            "p99_ms": 0.0,
            "p999_ms": 0.0,
        }
    sorted_latencies = sorted(latencies)
    return {
        "concurrency": concurrency,
        "duration_s": duration_s,
        "total_requests": len(latencies),
        "errors": errors,
        "rps": len(latencies) / duration_s,
        "p50_ms": percentile(sorted_latencies, 50),
        "p95_ms": percentile(sorted_latencies, 95),
        "p99_ms": percentile(sorted_latencies, 99),
        "p999_ms": percentile(sorted_latencies, 99.9),
    }


def _peak_rss_mb():
    ru = resource.getrusage(resource.RUSAGE_SELF)
    return ru.ru_maxrss / 1024.0


async def _run_async(concurrency_levels, duration_s, payload):
    rows = []
    # FastMCP's stdio transport treats `env` as the subprocess
    # environment. Pass the current env through explicitly so benchmark
    # gates can select duration/payload knobs without losing the guest
    # default framed transport.
    transport = StdioTransport(command=MCP_SERVER, args=[], env=dict(os.environ))
    async with Client(transport) as client:
        # Warm-up call so subprocess/handshake cost doesn't pollute the
        # first concurrency level.
        await client.call_tool("local__echo", {"text": "warmup"})

        for c in concurrency_levels:
            console.print(f"  concurrency={c} ...")
            latencies, errors = await _drive_at_concurrency(
                client, c, duration_s, payload
            )
            row = _summarize(latencies, errors, c, duration_s)
            row["rss_peak_mb"] = _peak_rss_mb()
            rows.append(row)
    return rows


def mcp_load_bench(concurrency_levels=None, duration_s=None, payload=None):
    """Drive local__echo at each concurrency level; return the result dict."""
    concurrency_levels = concurrency_levels or DEFAULT_CONCURRENCY
    duration_s = duration_s or float(
        os.environ.get("CAPSEM_BENCH_MCP_DURATION", DEFAULT_DURATION_S)
    )
    payload = payload or os.environ.get("CAPSEM_BENCH_MCP_PAYLOAD", DEFAULT_PAYLOAD)

    console.print(
        f"[bold]mcp-load[/bold] tool=local__echo "
        f"payload_bytes={len(payload)} duration={duration_s}s"
    )

    rows = asyncio.run(_run_async(concurrency_levels, duration_s, payload))

    out = {
        "version": "1.0",
        "tool": "local__echo",
        "payload_bytes": len(payload),
        "concurrency_levels": rows,
    }

    table = Table(title=f"mcp-load (tool=local__echo, {duration_s}s per level)")
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
