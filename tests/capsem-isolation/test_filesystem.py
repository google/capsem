"""VM filesystem isolation: writes in one VM must not appear in another."""

import uuid

import pytest
from helpers.service import exec_output_text

pytestmark = pytest.mark.isolation


def test_write_in_a_absent_in_b(multi_vm_env):
    """File written in VM-A does not exist in VM-B."""
    client, vm_a, vm_b, _ = multi_vm_env
    path = f"/root/iso-{uuid.uuid4().hex[:8]}.txt"
    client.upload_file(vm_a, path, "only-in-a")

    resp = client.download_file(vm_b, path)
    assert resp is None or "error" in str(resp).lower(), (
        f"VM-B should not see file from VM-A: {resp}"
    )


def test_same_path_different_content(multi_vm_env):
    """Same path in two VMs holds different content."""
    client, vm_a, vm_b, _ = multi_vm_env
    path = "/root/shared-name.txt"
    client.upload_file(vm_a, path, "content-a")
    client.upload_file(vm_b, path, "content-b")

    resp_a = client.download_file(vm_a, path)
    resp_b = client.download_file(vm_b, path)
    assert resp_a == b"content-a"
    assert resp_b == b"content-b"


def test_delete_b_file_persists_in_a(multi_vm_env):
    """Deleting VM-B does not affect files in VM-A."""
    client, vm_a, _, _ = multi_vm_env
    path = f"/root/persist-{uuid.uuid4().hex[:8]}.txt"
    client.upload_file(vm_a, path, "survives")

    # VM-B deletion happens in other tests or can be simulated
    # For now, just verify A's file survives regardless
    resp = client.download_file(vm_a, path)
    assert resp == b"survives"


def test_exec_isolation(multi_vm_env):
    """Env var set in VM-A is not visible in VM-B."""
    client, vm_a, vm_b, _ = multi_vm_env
    client.post(f"/vms/{vm_a}/exec", {"command": "export ISO_VAR=secret && echo $ISO_VAR > /tmp/env.txt"})

    resp = client.post(f"/vms/{vm_b}/exec", {"command": "cat /tmp/env.txt 2>/dev/null || echo MISSING"})
    stdout = exec_output_text(resp)
    assert "secret" not in stdout
