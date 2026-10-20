"""Mandatory for every image: it survives the lifecycle a user puts it through.

A stop and a resume relaunch the image's own command from the VM's boot (on
its held terminal, like the first launch), with the workspace intact; a fork
is a session of the same digest that carries the files. Both keep what the
workload wrote to its own root -- the session's layer above the image.
"""

import pytest
from helpers.image_session import WORKSPACE, wait_running, workload_exec

from tests.qualification.image_command import assert_image_command

pytestmark = pytest.mark.integration


def test_a_stop_and_a_resume_relaunch_the_image_with_its_workspace(
    service, candidate, session
):
    client = service.client()
    wrote = workload_exec(
        client,
        session,
        f"echo remembered > {WORKSPACE}/kept && echo layered > ~/.capsem-layer",
    )
    assert wrote.get("exit_code") == 0, wrote
    client.post(f"/vms/{session}/stop", {}, timeout=60)
    client.post(f"/vms/{session}/resume", {}, timeout=180)
    wait_running(client, session, timeout=300)
    assert_image_command(client, session, candidate)
    kept = workload_exec(client, session, f"cat {WORKSPACE}/kept ~/.capsem-layer")
    assert kept["stdout_text"] == "remembered\nlayered\n", kept


def test_a_fork_is_the_same_image_with_the_files(service, candidate, session):
    client = service.client()
    workload_exec(
        client,
        session,
        f"echo carried > {WORKSPACE}/carried && echo layered > ~/.capsem-layer",
    )
    fork_id = client.post(f"/vms/{session}/fork", {"name": "qualify-fork"})["id"]
    try:
        client.post(f"/vms/{fork_id}/resume", {}, timeout=180)
        wait_running(client, fork_id, timeout=300)
        assert client.get(f"/vms/{fork_id}/container")["digest"] == candidate.digest
        assert_image_command(client, fork_id, candidate)
        carried = workload_exec(
            client, fork_id, f"cat {WORKSPACE}/carried ~/.capsem-layer"
        )
        assert carried["stdout_text"] == "carried\nlayered\n", carried
    finally:
        client.delete(f"/vms/{fork_id}/delete", timeout=60)
