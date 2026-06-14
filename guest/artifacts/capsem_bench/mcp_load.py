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
import time

from fastmcp import Client
from fastmcp.client.transports import StdioTransport

from .helpers import console
from .load_harness import (
    DurationLoadConfig,
    render_load_table,
    summarize_load_level,
)

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
    return summarize_load_level(latencies, errors, concurrency, duration_s)


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
            rows.append(_summarize(latencies, errors, c, duration_s))
    return rows


def mcp_load_bench(concurrency_levels=None, duration_s=None, payload=None):
    """Drive local__echo at each concurrency level; return the result dict."""
    config = DurationLoadConfig.from_inputs(
        "mcp-load",
        default_concurrency=DEFAULT_CONCURRENCY,
        default_duration_s=DEFAULT_DURATION_S,
        concurrency_levels=concurrency_levels,
        duration_s=duration_s,
    )
    payload = payload or os.environ.get("CAPSEM_BENCH_MCP_PAYLOAD", DEFAULT_PAYLOAD)

    console.print(
        f"[bold]mcp-load[/bold] tool=local__echo "
        f"payload_bytes={len(payload)} duration={config.duration_s}s "
        f"concurrency={','.join(str(c) for c in config.concurrency_levels)}"
    )

    rows = asyncio.run(_run_async(config.concurrency_levels, config.duration_s, payload))

    out = {
        "version": "1.0",
        "tool": "local__echo",
        "payload_bytes": len(payload),
        "concurrency_levels": rows,
    }

    render_load_table(
        f"mcp-load (tool=local__echo, {config.duration_s}s per level)",
        rows,
    )

    return out
