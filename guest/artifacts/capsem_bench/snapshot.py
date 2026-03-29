"""Snapshot operation benchmarks (end-to-end via MCP gateway)."""

import json
import os
import shutil
import subprocess
import time

from rich.table import Table
from rich.text import Text

from .helpers import console

SNAPSHOT_WORKSPACE = "/root"
SNAPSHOT_FILE_COUNTS = [10, 100, 500]
SNAPSHOT_FILE_SIZE = 4096  # 4K per file


def snapshot_run(args):
    """Run the snapshots CLI tool and return (stdout, duration_ms, ok, stderr)."""
    start = time.monotonic()
    try:
        result = subprocess.run(
            ["snapshots"] + args + ["--json"],
            capture_output=True, text=True, timeout=30,
        )
        elapsed_ms = round((time.monotonic() - start) * 1000, 1)
        return result.stdout.strip(), elapsed_ms, result.returncode == 0, result.stderr.strip()
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError) as e:
        elapsed_ms = round((time.monotonic() - start) * 1000, 1)
        return str(e), elapsed_ms, False, ""


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
            if os.path.isdir(p):
                shutil.rmtree(p)
            else:
                os.remove(p)


def snapshot_bench():
    """Benchmark snapshot operations end-to-end via MCP gateway."""
    table = Table(title=Text("Snapshot Operations (e2e via MCP)"))
    table.add_column("Operation", style="bold")
    table.add_column("Files", justify="right")
    table.add_column("Latency (ms)", justify="right")
    table.add_column("Status")

    results = {}

    for n_files in SNAPSHOT_FILE_COUNTS:
        label = f"{n_files} files"
        run_results = {}

        snapshot_cleanup_workspace()
        snapshot_populate_workspace(n_files)

        # create
        snap_name = f"bench_{n_files}"
        create_out, create_ms, ok, err = snapshot_run(["create", snap_name])
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
        _, list_ms, ok, err = snapshot_run(["list"])
        run_results["list_ms"] = list_ms
        run_results["list_ok"] = ok
        status = "ok" if ok else f"FAIL: {err[:60]}" if err else "FAIL"
        table.add_row("list", label, f"{list_ms}", status)

        # changes
        _, changes_ms, ok, err = snapshot_run(["changes"])
        run_results["changes_ms"] = changes_ms
        run_results["changes_ok"] = ok
        status = "ok" if ok else f"FAIL: {err[:60]}" if err else "FAIL"
        table.add_row("changes", label, f"{changes_ms}", status)

        # revert
        _, revert_ms, ok, err = snapshot_run(["revert", "dir_0/file_0.txt"])
        run_results["revert_ms"] = revert_ms
        run_results["revert_ok"] = ok
        status = "ok" if ok else f"FAIL: {err[:60]}" if err else "FAIL"
        table.add_row("revert", label, f"{revert_ms}", status)

        # delete
        if checkpoint:
            _, delete_ms, ok, err = snapshot_run(["delete", checkpoint])
        else:
            delete_ms = 0.0
            ok = False
            err = "no checkpoint from create"
            for cp_idx in range(3, 20):
                _, delete_ms, ok, err = snapshot_run(["delete", f"cp-{cp_idx}"])
                if ok:
                    break
        run_results["delete_ms"] = delete_ms
        run_results["delete_ok"] = ok
        status = "ok" if ok else f"FAIL: {err[:60]}" if err else "FAIL"
        table.add_row("delete", label, f"{delete_ms}", status)

        table.add_section()
        results[f"{n_files}_files"] = run_results

    snapshot_cleanup_workspace()

    console.print(table)
    return results
