"""Mandatory for every image: it survives the lifecycle a user puts it through.

A stop and a resume relaunch the image's own command from the VM's boot (on
its held terminal, like the first launch), with the workspace intact; a fork
is a session of the same digest that carries the files.
"""

import pytest
from helpers.image_session import WORKSPACE, wait_running, workload_exec

pytestmark = pytest.mark.integration


def pid1(client, vm_id):
    result = workload_exec(client, vm_id, "tr '\\0' '\\n' < /proc/1/cmdline")
    assert result.get("exit_code") == 0, result
    return result["stdout_text"].splitlines()


def test_a_stop_and_a_resume_relaunch_the_image_with_its_workspace(service, candidate, session):
    client = service.client()
    wrote = workload_exec(client, session, f"echo remembered > {WORKSPACE}/kept")
    assert wrote.get("exit_code") == 0, wrote
    client.post(f"/vms/{session}/stop", {}, timeout=60)
    client.post(f"/vms/{session}/resume", {}, timeout=180)
    wait_running(client, session, timeout=300)
    assert pid1(client, session) == candidate.command
    kept = workload_exec(client, session, f"cat {WORKSPACE}/kept")
    assert kept["stdout_text"] == "remembered\n", kept


def test_a_fork_is_the_same_image_with_the_files(service, candidate, session):
    client = service.client()
    workload_exec(client, session, f"echo carried > {WORKSPACE}/carried")
    fork_id = client.post(f"/vms/{session}/fork", {"name": "qualify-fork"})["id"]
    try:
        client.post(f"/vms/{fork_id}/resume", {}, timeout=180)
        wait_running(client, fork_id, timeout=300)
        assert client.get(f"/vms/{fork_id}/container")["digest"] == candidate.digest
        assert pid1(client, fork_id) == candidate.command
        carried = workload_exec(client, fork_id, f"cat {WORKSPACE}/carried")
        assert carried["stdout_text"] == "carried\n", carried
    finally:
        client.delete(f"/vms/{fork_id}/delete", timeout=60)
