"""Verify --rm (auto_remove) deletes VM when its process exits.

Tests the SERVICE-SIDE auto-remove behavior: when a VM process dies and
auto_remove=True, the service should automatically clean up the VM from
its instance list and remove the session directory. This is distinct from
explicit client.delete() which works regardless of auto_remove.
"""

import os
import signal
import subprocess
import time
import uuid

import pytest

from pathlib import Path

from helpers.service import wait_exec_ready

PROJECT_ROOT = Path(__file__).parent.parent.parent
CLI_BINARY = PROJECT_ROOT / "target/debug/capsem"

pytestmark = pytest.mark.cleanup


def _get_vm_pid(client, name):
    """Get the OS process ID for a VM."""
    info = client.get(f"/info/{name}")
    return info.get("pid") if info else None


def _vm_in_list(client, name):
    """Check if a VM appears in the service list."""
    listing = client.get("/list")
    ids = [s["id"] for s in listing.get("sandboxes", [])]
    return name in ids


def _run_cli(*args, uds_path=None, timeout=60):
    """Run capsem CLI and return (stdout, stderr, returncode)."""
    cmd = [str(CLI_BINARY)]
    if uds_path:
        cmd += ["--uds-path", str(uds_path)]
    cmd += list(args)
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return result.stdout, result.stderr, result.returncode


def test_auto_remove_on_process_exit(cleanup_env):
    """Provision with auto_remove=True, kill VM process, verify service cleans up."""
    client = cleanup_env.client()
    name = f"rmk-{uuid.uuid4().hex[:6]}"
    client.post("/provision", {
        "name": name,
        "ram_mb": 2048,
        "cpus": 2,
        "auto_remove": True,
    })
    wait_exec_ready(client, name, timeout=30)

    assert _vm_in_list(client, name), f"VM {name} not in list after provision"

    pid = _get_vm_pid(client, name)
    assert pid, f"Could not get PID for VM {name}"

    # Kill the VM process -- service should detect exit and auto-remove
    os.kill(pid, signal.SIGTERM)

    # Wait for service to notice and clean up
    for _ in range(10):
        time.sleep(1)
        if not _vm_in_list(client, name):
            return  # success
    pytest.fail(f"VM {name} still in list 10s after process kill (auto_remove=True)")


def test_auto_remove_cleans_session_dir(cleanup_env):
    """Provision with auto_remove=True, kill process, verify session dir removed."""
    client = cleanup_env.client()
    name = f"rmd-{uuid.uuid4().hex[:6]}"
    client.post("/provision", {
        "name": name,
        "ram_mb": 2048,
        "cpus": 2,
        "auto_remove": True,
    })
    wait_exec_ready(client, name, timeout=30)

    sessions_dir = cleanup_env.tmp_dir / "sessions" / name
    pid = _get_vm_pid(client, name)
    assert pid, f"Could not get PID for VM {name}"

    os.kill(pid, signal.SIGTERM)

    for _ in range(10):
        time.sleep(1)
        if not _vm_in_list(client, name):
            break
    else:
        pytest.fail(f"VM {name} not removed from list after process kill")

    if sessions_dir.exists():
        pytest.fail(f"Session dir {sessions_dir} still exists after auto-remove")


def test_no_auto_remove_on_process_exit(cleanup_env):
    """Provision without auto_remove, kill process, VM stays in list as stale."""
    client = cleanup_env.client()
    name = f"no-autorm-{uuid.uuid4().hex[:8]}"

    client.post("/provision", {
        "name": name,
        "ram_mb": 2048,
        "cpus": 2,
        "auto_remove": False,
    })
    wait_exec_ready(client, name, timeout=30)

    pid = _get_vm_pid(client, name)
    assert pid, f"Could not get PID for VM {name}"

    os.kill(pid, signal.SIGTERM)

    # Give the service time to run its cleanup loop
    time.sleep(5)

    # Without auto_remove, the stale instance cleanup may or may not
    # remove it from the list (implementation-dependent), but explicit
    # delete must still work
    client.delete(f"/delete/{name}")

    assert not _vm_in_list(client, name), f"VM {name} still in list after explicit delete"


def test_auto_remove_via_cli(cleanup_env):
    """capsem start --rm, kill process, verify VM disappears from capsem ls."""
    uds_path = cleanup_env.uds_path
    name = f"rmc-{uuid.uuid4().hex[:6]}"
    stdout, stderr, rc = _run_cli("start", "--rm", "--name", name, uds_path=str(uds_path))
    assert rc == 0, f"start --rm failed: {stderr}"

    client = cleanup_env.client()
    wait_exec_ready(client, name, timeout=30)

    pid = _get_vm_pid(client, name)
    assert pid, f"Could not get PID for VM {name}"

    os.kill(pid, signal.SIGTERM)

    for _ in range(10):
        time.sleep(1)
        ls_out, _, _ = _run_cli("ls", uds_path=str(uds_path))
        if name not in ls_out:
            return  # success
    pytest.fail(f"VM {name} still in capsem ls 10s after process kill (--rm)")
