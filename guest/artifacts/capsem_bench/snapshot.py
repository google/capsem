"""Snapshot operation benchmarks (end-to-end via the guest MCP endpoint)."""

import asyncio
import json
import os
import shutil
import time

from rich.table import Table
from rich.text import Text

from .helpers import console
from .mcp_transport import Client, StdioTransport

SNAPSHOT_WORKSPACE = "/root"
SNAPSHOT_FILE_COUNTS = [10, 100, 500]
SNAPSHOT_FILE_SIZE = 4096  # 4K per file
MCP_SERVER = "/run/capsem-mcp-server"


async def snapshot_run(client, name, arguments=None):
    """Call one snapshot MCP tool and return (text, duration_ms, ok, error)."""
    start = time.monotonic()
    try:
        result = await client.call_tool(f"local__snapshots_{name}", arguments or {})
        elapsed_ms = round((time.monotonic() - start) * 1000, 1)
        text = next(
            (block.text for block in result.content if hasattr(block, "text")),
            "",
        )
        return text, elapsed_ms, not result.is_error, ""
    except Exception as error:
        elapsed_ms = round((time.monotonic() - start) * 1000, 1)
        return "", elapsed_ms, False, str(error)


def snapshot_populate_workspace(n_files, file_size=SNAPSHOT_FILE_SIZE):
    """Create n_files in the workspace, each file_size bytes."""
    os.makedirs(SNAPSHOT_WORKSPACE, exist_ok=True)
    content = b"x" * file_size
    for i in range(n_files):
        subdir = os.path.join(SNAPSHOT_WORKSPACE, f"dir_{i // 50}")
        os.makedirs(subdir, exist_ok=True)
        with open(os.path.join(subdir, f"file_{i}.txt"), "wb") as f:
            f.write(content)


def snapshot_cleanup_workspace():
    """Remove all files from workspace (keep dir)."""
    if os.path.isdir(SNAPSHOT_WORKSPACE):
        for entry in os.listdir(SNAPSHOT_WORKSPACE):
            p = os.path.join(SNAPSHOT_WORKSPACE, entry)
            if os.path.islink(p):
                os.unlink(p)
            elif os.path.isdir(p):
                shutil.rmtree(p)
            else:
                os.remove(p)


async def _snapshot_bench(client, table):
    results = {}
    for n_files in SNAPSHOT_FILE_COUNTS:
        label = f"{n_files} files"
        run_results = {}

        snapshot_cleanup_workspace()
        snapshot_populate_workspace(n_files)

        # create
        snap_name = f"bench_{n_files}"
        create_out, create_ms, ok, err = await snapshot_run(
            client, "create", {"name": snap_name}
        )
        run_results["create_ms"] = create_ms
        run_results["create_ok"] = ok
        status = "ok" if ok else f"FAIL: {err[:60]}" if err else "FAIL"
        table.add_row("create", label, f"{create_ms}", status)

        checkpoint = None
        try:
            create_data = json.loads(create_out)
            if isinstance(create_data, dict):
                checkpoint = create_data.get("checkpoint")
        except (json.JSONDecodeError, TypeError):
            pass

        # Modify a file so there's a diff for revert.
        marker = os.path.join(SNAPSHOT_WORKSPACE, "dir_0", "file_0.txt")
        if os.path.exists(marker):
            with open(marker, "w") as f:
                f.write("modified for bench -- different content")
                f.flush()
                os.fsync(f.fileno())

        # list
        _, list_ms, ok, err = await snapshot_run(client, "list", {"format": "json"})
        run_results["list_ms"] = list_ms
        run_results["list_ok"] = ok
        status = "ok" if ok else f"FAIL: {err[:60]}" if err else "FAIL"
        table.add_row("list", label, f"{list_ms}", status)

        # changes
        _, changes_ms, ok, err = await snapshot_run(
            client, "changes", {"format": "json"}
        )
        run_results["changes_ms"] = changes_ms
        run_results["changes_ok"] = ok
        status = "ok" if ok else f"FAIL: {err[:60]}" if err else "FAIL"
        table.add_row("changes", label, f"{changes_ms}", status)

        # revert
        _, revert_ms, ok, err = await snapshot_run(
            client, "revert", {"path": "dir_0/file_0.txt"}
        )
        run_results["revert_ms"] = revert_ms
        run_results["revert_ok"] = ok
        status = "ok" if ok else f"FAIL: {err[:60]}" if err else "FAIL"
        table.add_row("revert", label, f"{revert_ms}", status)

        # delete
        if checkpoint:
            _, delete_ms, ok, err = await snapshot_run(
                client, "delete", {"checkpoint": checkpoint}
            )
        else:
            delete_ms = 0.0
            ok = False
            err = "no checkpoint from create"
            for cp_idx in range(3, 20):
                _, delete_ms, ok, err = await snapshot_run(
                    client, "delete", {"checkpoint": f"cp-{cp_idx}"}
                )
                if ok:
                    break
        run_results["delete_ms"] = delete_ms
        run_results["delete_ok"] = ok
        status = "ok" if ok else f"FAIL: {err[:60]}" if err else "FAIL"
        table.add_row("delete", label, f"{delete_ms}", status)

        table.add_section()
        results[f"{n_files}_files"] = run_results

    return results


def snapshot_bench():
    """Benchmark snapshot operations through one persistent MCP connection."""
    table = Table(title=Text("Snapshot Operations (e2e via MCP)"))
    table.add_column("Operation", style="bold")
    table.add_column("Files", justify="right")
    table.add_column("Latency (ms)", justify="right")
    table.add_column("Status")

    async def run():
        transport = StdioTransport(command=MCP_SERVER, args=[])
        async with Client(transport) as client:
            return await _snapshot_bench(client, table)

    try:
        results = asyncio.run(run())
    finally:
        snapshot_cleanup_workspace()

    console.print(table)
    return results
