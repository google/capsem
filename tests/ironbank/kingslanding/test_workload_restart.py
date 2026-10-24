"""Saved app sessions pick up controller fixes without losing user state."""

from pathlib import Path

import pytest
from helpers.image_session import image_session, wait_running, workload_exec
from helpers.service import vm_session_dir

from tests.ironbank.kingslanding.test_run import service
from tests.ironbank.kingslanding.test_workload_terminal import (
    probe,
    resize_workload,
    terminal,
)

__all__ = ["service"]
pytestmark = pytest.mark.integration
PROJECT_ROOT = Path(__file__).resolve().parents[3]


def test_saved_workload_refreshes_its_controller_and_terminal_after_restarts(service, tmp_path):
    client = service.client()
    current = (PROJECT_ROOT / "guest/artifacts/container/launch.py").read_bytes()
    # A stopped session's saved controller must be refreshed before execution.
    # Poison it independently of the current implementation: mutating a source
    # literal stopped testing resume when the console code moved to a helper.
    stale = b'raise SystemExit("stale controller executed before refresh")\n'
    with image_session(service, tmp_path, "restart-terminal") as vm_id:
        stage = vm_session_dir(service.tmp_dir, client, vm_id) / "guest/workspace/.capsem-image"
        options = (stage / "options.json").read_bytes()
        result = workload_exec(client, vm_id, "printf preserved > restart-proof.txt")
        assert result["exit_code"] == 0, result
        for _ in range(2):
            client.post(f"/vms/{vm_id}/stop", {}, timeout=60)
            (stage / "launch.py").write_bytes(stale)
            client.post(f"/vms/{vm_id}/resume", {}, timeout=120)
            wait_running(client, vm_id)
            assert (stage / "launch.py").read_bytes() == current
            assert (stage / "options.json").read_bytes() == options
            result = workload_exec(client, vm_id, "cat restart-proof.txt")
            assert result["exit_code"] == 0, result
            assert result["stdout_text"] == "preserved", result
            with terminal(service, vm_id) as socket:
                assert probe(socket) == ("1000", "100000", "workload")
                resize_workload(socket)
