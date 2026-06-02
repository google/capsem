import asyncio
import sys
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).parent.parent
sys.path.insert(0, str(PROJECT_ROOT / "guest" / "artifacts"))

from capsem_bench import mcp_load  # noqa: E402


def test_selected_lanes_defaults_to_all(monkeypatch):
    monkeypatch.delenv("CAPSEM_BENCH_MCP_LANES", raising=False)

    assert mcp_load._selected_lanes() == (
        "fastmcp",
        "raw-single",
        "raw-multiprocess",
    )


def test_selected_lanes_rejects_unknown(monkeypatch):
    monkeypatch.setenv("CAPSEM_BENCH_MCP_LANES", "fastmcp,nope")

    with pytest.raises(ValueError, match="unknown CAPSEM_BENCH_MCP_LANES"):
        mcp_load._selected_lanes()


def test_raw_mcp_client_matches_out_of_order_responses(tmp_path):
    fake = tmp_path / "fake-mcp-server"
    fake.write_text(
        """#!/usr/bin/env python3
import json
import sys

for line in sys.stdin:
    req = json.loads(line)
    text = req["params"]["arguments"]["text"]
    resp = {
        "jsonrpc": "2.0",
        "id": req["id"],
        "result": {
            "content": [{"type": "text", "text": text}],
            "isError": False,
        },
    }
    print(json.dumps(resp), flush=True)
"""
    )
    fake.chmod(0o755)

    async def run():
        client = mcp_load.RawMcpClient(command=str(fake))
        await client.start()
        try:
            results = await asyncio.gather(
                *(client.call_echo(f"payload-{i}") for i in range(8))
            )
        finally:
            await client.close()
        return results

    results = asyncio.run(run())

    assert [result["content"][0]["text"] for result in results] == [
        f"payload-{i}" for i in range(8)
    ]


def test_mcp_load_bench_preserves_legacy_fastmcp_key(monkeypatch):
    monkeypatch.setenv("CAPSEM_BENCH_MCP_LANES", "raw-single")

    async def fake_run(_levels, _duration, _payload, lanes):
        assert lanes == ("raw-single",)
        return {
            "raw-single": [
                {
                    "lane": "raw-single",
                    "concurrency": 1,
                    "duration_s": 0.01,
                    "total_requests": 1,
                    "errors": 0,
                    "rps": 100.0,
                    "p50_ms": 1.0,
                    "p95_ms": 1.0,
                    "p99_ms": 1.0,
                    "p999_ms": 1.0,
                }
            ]
        }

    monkeypatch.setattr(mcp_load, "_run_async", fake_run)

    result = mcp_load.mcp_load_bench(
        concurrency_levels=(1,),
        duration_s=0.01,
        payload="x",
    )

    assert result["version"] == "1.1"
    assert result["lanes"]["raw-single"][0]["rps"] == 100.0
    assert result["concurrency_levels"] == []
