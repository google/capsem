"""Stress variant of test_suspend_and_resume_persistent.

Parametrized 50x. MUST run serially (`-n 1` under pytest-xdist, or
without xdist). Running at higher concurrency spawns multiple
capsem-service processes, and the ServiceState::save_restore_lock
that serializes Apple VZ save/restore on the host is scoped to a
single service -- see docs/gotchas/concurrent-suspend-resume.mdx for
the full story. A deployed host always has exactly one service, so
the serial measurement matches production; -n 2+ measures a state
that never occurs outside the test harness.

Gated behind `CAPSEM_STRESS` so `just test` runs don't get swamped.

Run with:
    CAPSEM_STRESS=1 uv run pytest tests/capsem-mcp/test_stress_suspend_resume.py \
        -n 1 --tb=line -q
"""

import os

import pytest

from helpers.constants import EXEC_READY_TIMEOUT
from helpers.mcp import parse_content, wait_exec_ready

pytestmark = [
    pytest.mark.mcp,
    pytest.mark.skipif(
        not os.environ.get("CAPSEM_STRESS"),
        reason="stress harness: set CAPSEM_STRESS=1 to enable",
    ),
]


@pytest.mark.parametrize("run", range(50))
def test_suspend_and_resume_persistent_stress(fresh_vm, mcp_session, run):
    """Mirror of test_suspend_and_resume_persistent, multiplied by 50.

    Each parameterized run is independent: its own fresh_vm, its own
    mcp_session, its own full suspend/resume cycle. Expected to fail N/50
    times pre-fix and ~0/50 post-fix if the handshake+reader fixes
    addressed the observed contention mode.
    """
    vm_name = fresh_vm()
    assert wait_exec_ready(mcp_session, vm_name, timeout=EXEC_READY_TIMEOUT), (
        f"[run {run}] {vm_name} never exec-ready"
    )

    mcp_session.call_tool("capsem_write_file", {
        "id": vm_name,
        "path": "/root/marker.txt",
        "content": f"persisted-through-suspend-{run}",
    })

    mcp_session.call_tool("capsem_suspend", {"id": vm_name})
    info = parse_content(mcp_session.call_tool("capsem_info", {"id": vm_name}))
    assert info["status"] == "Suspended", (
        f"[run {run}] status after suspend: {info['status']!r}"
    )

    mcp_session.call_tool("capsem_resume", {"name": vm_name})
    assert wait_exec_ready(mcp_session, vm_name, timeout=EXEC_READY_TIMEOUT), (
        f"[run {run}] VM did not become exec-ready after resume"
    )

    info = parse_content(mcp_session.call_tool("capsem_info", {"id": vm_name}))
    assert info["status"] == "Running", (
        f"[run {run}] status after resume: {info['status']!r}"
    )

    res = mcp_session.call_tool("capsem_read_file", {
        "id": vm_name,
        "path": "/root/marker.txt",
    })
    assert parse_content(res)["content"] == f"persisted-through-suspend-{run}"
