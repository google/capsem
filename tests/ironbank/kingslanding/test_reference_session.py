"""The reference image boots as a user's session, and exec reaches its workload.

The reference image is the official `dev` image at the digest
`config/gate.toml [functional.reference_image]` pins: the product's own shape
(a non-root image user, /workspace, git, a toolchain) with no agent. Every
runtime test that boots a workload boots this one, through `image_session`,
admitted as an administrator would grant one image.

A command meant for the workload must land there whether the caller names the
workload or omits the target, as `capsem exec` does; and when the workload is
gone, it must fail rather than run in the VM.
"""

import time

import pytest
from helpers.image_session import (
    DEFAULT_TARGET,
    WORKSPACE,
    NotInWorkload,
    image_session,
    workload_exec,
)
from helpers.service import exec_output_text

from tests.fixtures.oci.pinned_image import reference_image
from tests.ironbank.kingslanding.test_run import service

__all__ = ["service"]

pytestmark = pytest.mark.integration

IDENTITY = "id -u; pwd; git --version; cc --version | head -1"


@pytest.mark.parametrize(
    "target", ["workload", DEFAULT_TARGET], ids=["explicit", "omitted"]
)
def test_the_reference_image_runs_as_its_user_in_the_workspace(
    service, tmp_path, target
):
    client = service.client()
    with image_session(service, tmp_path, "reference") as vm_id:
        status = client.get(f"/vms/{vm_id}/container")
        assert status["state"] == "running", status
        assert status["digest"] == reference_image.pinned(), status
        result = workload_exec(client, vm_id, IDENTITY, target=target)
        assert result.get("exit_code") == 0, result
        uid, cwd, git, compiler = result["stdout_text"].splitlines()
        assert (uid, cwd) == ("1000", WORKSPACE), result
        assert git.startswith("git version"), git
        assert compiler.startswith("cc ("), compiler


def test_the_probe_tells_the_vm_from_the_workload(service, tmp_path):
    """The harness's own negative control: the same probe, sent to the VM,
    reports the VM's identity map, so a command that fell back to the VM is
    caught rather than passed."""
    client = service.client()
    with image_session(service, tmp_path, "reference-probe") as vm_id:
        in_vm = client.post(
            f"/vms/{vm_id}/exec",
            {
                "command": "head -1 /proc/self/uid_map",
                "target": "vm",
                "timeout_secs": 30,
            },
            timeout=40,
        )
        assert in_vm.get("exit_code") == 0, in_vm
        assert exec_output_text(in_vm).split() == ["0", "0", "4294967295"], in_vm
        in_workload = workload_exec(client, vm_id, "true")
        assert in_workload.get("exit_code") == 0, in_workload


def test_a_workload_exec_without_a_workload_fails_instead_of_running_in_the_vm(
    service, tmp_path
):
    client = service.client()
    with image_session(service, tmp_path, "reference-gone") as vm_id:
        killed = client.post(
            f"/vms/{vm_id}/exec",
            {
                "command": "kill -9 $(cat /var/tmp/capsem-container/workload.pid)",
                "target": "vm",
                "timeout_secs": 30,
            },
            timeout=40,
        )
        assert killed.get("exit_code") == 0, killed
        # The status follows: a workload that ran and ended is exited, never
        # running forever.
        deadline = time.monotonic() + 30
        status = client.get(f"/vms/{vm_id}/container")
        while status["state"] != "exited" and time.monotonic() < deadline:
            time.sleep(0.5)
            status = client.get(f"/vms/{vm_id}/container")
        assert status["state"] == "exited", status
        assert status["exit_code"] == 137, status
        for target in ("workload", DEFAULT_TARGET):
            with pytest.raises(NotInWorkload) as refused:
                workload_exec(client, vm_id, "true", target=target)
            # Refused with no shell at all is the contract; a fallback that
            # ran it in the VM is the failure this guards.
            assert "ran in the VM" not in str(refused.value), refused.value
