"""mcp-load: concurrency-driven load test against the guest MCP path.

Drives `local__echo` (the diagnostic builtin tool that returns input
verbatim with zero I/O) at multiple concurrency levels so we can
characterize the MCP transport overhead end-to-end:

    Python client (in guest)
      -> stdio -> /run/capsem-mcp-server (guest agent's MCP server)
      -> framed MCP over vsock:5002 -> capsem-process MITM MCP endpoint
      -> endpoint-local echo handler returns the text
      -> back up the chain

Pure protocol cost. If `mcp-load` does NOT scale linearly with
concurrency, we have a serialization bug in the guest relay / MITM
endpoint / vsock path, policy/telemetry handling, or the guest stdio
client path and the transport needs attention.
Sister bench to `mitm-load` (which isolates the proxy hot path).
"""

import asyncio
import contextlib
import json
import os
import resource
import time

from rich.table import Table

from .helpers import console, percentile

MCP_SERVER = "/run/capsem-mcp-server"
DEFAULT_CONCURRENCY = (1, 10, 50, 200)
DEFAULT_DURATION_S = 10.0
DEFAULT_PAYLOAD = "ping"
DEFAULT_LANES = ("fastmcp", "raw-single", "raw-multiprocess")
RAW_MULTIPROCESS_RELAYS = 4


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


async def _drive_raw_at_concurrency(client, concurrency, duration_s, payload):
    deadline = time.monotonic() + duration_s
    latencies = []
    errors = 0
    lat_lock = asyncio.Lock()

    async def worker():
        nonlocal errors
        while time.monotonic() < deadline:
            t0 = time.monotonic()
            try:
                await client.call_echo(payload)
                ms = (time.monotonic() - t0) * 1000
                async with lat_lock:
                    latencies.append(ms)
            except Exception:
                errors += 1

    await asyncio.gather(*(worker() for _ in range(concurrency)))
    return latencies, errors


def _summarize(latencies, errors, concurrency, duration_s, lane):
    if not latencies:
        return {
            "lane": lane,
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
        "lane": lane,
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


class RawMcpClient:
    def __init__(self, command=MCP_SERVER):
        self.command = command
        self.proc = None
        self.next_id = 1
        self.pending = {}
        self.write_lock = asyncio.Lock()
        self.reader_task = None

    async def start(self):
        self.proc = await asyncio.create_subprocess_exec(
            self.command,
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.DEVNULL,
            env=dict(os.environ),
        )
        self.reader_task = asyncio.create_task(self._read_responses())
        await self.call_echo("warmup")

    async def close(self):
        if self.proc is not None:
            if self.proc.stdin is not None:
                self.proc.stdin.close()
                with contextlib.suppress(Exception):
                    await self.proc.stdin.wait_closed()
            with contextlib.suppress(asyncio.TimeoutError):
                await asyncio.wait_for(self.proc.wait(), timeout=2)
            if self.proc.returncode is None:
                self.proc.kill()
                with contextlib.suppress(Exception):
                    await self.proc.wait()
        if self.reader_task is not None:
            self.reader_task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await self.reader_task

    async def call_echo(self, payload):
        request_id = await self._send_request(
            "tools/call",
            {"name": "local__echo", "arguments": {"text": payload}},
        )
        response = await self.pending[request_id]
        if "error" in response:
            raise RuntimeError(response["error"])
        return response.get("result")

    async def _send_request(self, method, params):
        async with self.write_lock:
            request_id = self.next_id
            self.next_id += 1
            loop = asyncio.get_running_loop()
            self.pending[request_id] = loop.create_future()
            line = (
                json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "method": method,
                        "params": params,
                    },
                    separators=(",", ":"),
                ).encode("utf-8")
                + b"\n"
            )
            self.proc.stdin.write(line)
            await self.proc.stdin.drain()
            return request_id

    async def _read_responses(self):
        while True:
            line = await self.proc.stdout.readline()
            if not line:
                break
            try:
                response = json.loads(line)
            except json.JSONDecodeError:
                continue
            request_id = response.get("id")
            future = self.pending.pop(request_id, None)
            if future is not None and not future.done():
                future.set_result(response)
        for future in self.pending.values():
            if not future.done():
                future.set_exception(RuntimeError("MCP relay exited"))
        self.pending.clear()


class RawMcpPool:
    def __init__(self, size):
        self.clients = [RawMcpClient() for _ in range(size)]
        self.next_client = 0
        self.lock = asyncio.Lock()

    async def start(self):
        await asyncio.gather(*(client.start() for client in self.clients))

    async def close(self):
        await asyncio.gather(*(client.close() for client in self.clients))

    async def call_echo(self, payload):
        async with self.lock:
            client = self.clients[self.next_client]
            self.next_client = (self.next_client + 1) % len(self.clients)
        return await client.call_echo(payload)


async def _run_fastmcp(concurrency_levels, duration_s, payload):
    from fastmcp import Client
    from fastmcp.client.transports import StdioTransport

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
            console.print(f"  lane=fastmcp concurrency={c} ...")
            latencies, errors = await _drive_at_concurrency(
                client, c, duration_s, payload
            )
            row = _summarize(latencies, errors, c, duration_s, "fastmcp")
            row["rss_peak_mb"] = _peak_rss_mb()
            rows.append(row)
    return rows


async def _run_raw_lane(lane, client, concurrency_levels, duration_s, payload):
    rows = []
    await client.start()
    try:
        for c in concurrency_levels:
            console.print(f"  lane={lane} concurrency={c} ...")
            latencies, errors = await _drive_raw_at_concurrency(
                client, c, duration_s, payload
            )
            row = _summarize(latencies, errors, c, duration_s, lane)
            row["rss_peak_mb"] = _peak_rss_mb()
            rows.append(row)
    finally:
        await client.close()
    return rows


async def _run_async(concurrency_levels, duration_s, payload, lanes):
    rows_by_lane = {}
    if "fastmcp" in lanes:
        rows_by_lane["fastmcp"] = await _run_fastmcp(
            concurrency_levels, duration_s, payload
        )
    if "raw-single" in lanes:
        rows_by_lane["raw-single"] = await _run_raw_lane(
            "raw-single", RawMcpClient(), concurrency_levels, duration_s, payload
        )
    if "raw-multiprocess" in lanes:
        rows_by_lane["raw-multiprocess"] = await _run_raw_lane(
            "raw-multiprocess",
            RawMcpPool(RAW_MULTIPROCESS_RELAYS),
            concurrency_levels,
            duration_s,
            payload,
        )
    return rows_by_lane


def _selected_lanes():
    raw = os.environ.get("CAPSEM_BENCH_MCP_LANES")
    if not raw:
        return DEFAULT_LANES
    lanes = tuple(lane.strip() for lane in raw.split(",") if lane.strip())
    unknown = sorted(set(lanes) - set(DEFAULT_LANES))
    if unknown:
        raise ValueError(f"unknown CAPSEM_BENCH_MCP_LANES: {', '.join(unknown)}")
    return lanes


def mcp_load_bench(concurrency_levels=None, duration_s=None, payload=None):
    """Drive local__echo at each concurrency level; return the result dict."""
    concurrency_levels = concurrency_levels or DEFAULT_CONCURRENCY
    duration_s = duration_s or float(
        os.environ.get("CAPSEM_BENCH_MCP_DURATION", DEFAULT_DURATION_S)
    )
    payload = payload or os.environ.get("CAPSEM_BENCH_MCP_PAYLOAD", DEFAULT_PAYLOAD)
    lanes = _selected_lanes()

    console.print(
        f"[bold]mcp-load[/bold] tool=local__echo "
        f"payload_bytes={len(payload)} duration={duration_s}s lanes={','.join(lanes)}"
    )

    rows_by_lane = asyncio.run(_run_async(concurrency_levels, duration_s, payload, lanes))

    out = {
        "version": "1.1",
        "tool": "local__echo",
        "payload_bytes": len(payload),
        "lanes": rows_by_lane,
        "concurrency_levels": rows_by_lane.get("fastmcp", []),
    }

    table = Table(title=f"mcp-load (tool=local__echo, {duration_s}s per level)")
    table.add_column("lane", justify="left")
    table.add_column("concurrency", justify="right")
    table.add_column("rps", justify="right")
    table.add_column("p50_ms", justify="right")
    table.add_column("p95_ms", justify="right")
    table.add_column("p99_ms", justify="right")
    table.add_column("p999_ms", justify="right")
    table.add_column("errors", justify="right")
    for lane in lanes:
        for row in rows_by_lane[lane]:
            table.add_row(
                lane,
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
