import pytest
import uuid
import json
import time

from test_commands import run_cli, _provision_vm
from helpers.service import wait_exec_ready
from helpers.uds_client import UdsHttpClient

pytestmark = pytest.mark.integration

def test_fork_and_boot_from_image(uds_path):
    """Test the full fork lifecycle: boot VM -> modify -> fork -> boot image -> verify modification."""
    vm_name = f"base-{uuid.uuid4().hex[:8]}"

    # 1. Start base VM and wait for it to be ready
    stdout, stderr, rc = run_cli("create", "--name", vm_name, uds_path=uds_path)
    assert rc == 0
    client = UdsHttpClient(uds_path)
    wait_exec_ready(client, vm_name)

    # 2. Modify the workspace in the base VM
    stdout, stderr, rc = run_cli("exec", vm_name, "echo 'hello from fork' > /root/fork_test.txt", uds_path=uds_path)
    assert rc == 0

    stdout, stderr, rc = run_cli("exec", vm_name, "cat /root/fork_test.txt", uds_path=uds_path)
    assert rc == 0
    assert "hello from fork" in stdout

    # 3. Fork the running VM into an image
    image_name = f"my-image-{uuid.uuid4().hex[:8]}"
    stdout, stderr, rc = run_cli("fork", vm_name, image_name, "--description", "A test image", uds_path=uds_path)
    assert rc == 0, f"Fork failed: stdout: {stdout}, stderr: {stderr}"
    assert f"Forked VM {vm_name} to image '{image_name}'" in stdout

    # 4. Verify image exists in `image list`
    stdout, stderr, rc = run_cli("image", "list", uds_path=uds_path)
    assert rc == 0
    assert image_name in stdout
    assert vm_name in stdout # source_vm

    # 5. Verify `image inspect`
    stdout, stderr, rc = run_cli("image", "inspect", image_name, uds_path=uds_path)
    assert rc == 0
    info = json.loads(stdout)
    assert info["name"] == image_name
    assert info["description"] == "A test image"
    assert info["source_vm"] == vm_name

    # 6. Boot a new VM from the image
    forked_vm_name = f"forked-{uuid.uuid4().hex[:8]}"
    stdout, stderr, rc = run_cli("create", "--name", forked_vm_name, "--image", image_name, uds_path=uds_path)
    assert rc == 0

    # 7. Verify the modification exists in the new VM
    wait_exec_ready(client, forked_vm_name)
    stdout, stderr, rc = run_cli("exec", forked_vm_name, "cat /root/fork_test.txt", uds_path=uds_path)
    assert rc == 0
    assert "hello from fork" in stdout

    # 8. Clean up
    run_cli("delete", forked_vm_name, uds_path=uds_path)
    run_cli("delete", vm_name, uds_path=uds_path)
    stdout, stderr, rc = run_cli("image", "delete", image_name, uds_path=uds_path)
    assert rc == 0
    assert "deleted" in stdout

    # Verify image is gone
    stdout, stderr, rc = run_cli("image", "list", uds_path=uds_path)
    assert rc == 0
    assert image_name not in stdout


def test_fork_stopped_vm(uds_path):
    """Fork a stopped persistent VM -- image should preserve workspace state."""
    vm_name = f"st-{uuid.uuid4().hex[:6]}"
    image_name = f"si-{uuid.uuid4().hex[:6]}"
    forked_vm = f"sf-{uuid.uuid4().hex[:6]}"
    client = UdsHttpClient(uds_path)

    try:
        # Create, wait, write marker
        run_cli("create", "--name", vm_name, uds_path=uds_path)
        assert wait_exec_ready(client, vm_name), f"VM {vm_name} never exec-ready"
        stdout, _, rc = run_cli("exec", vm_name, "echo 'stopped-marker' > /root/stop_test.txt", uds_path=uds_path)
        assert rc == 0

        # Stop the VM
        _, _, rc = run_cli("stop", vm_name, uds_path=uds_path)
        assert rc == 0

        # Fork the stopped VM
        stdout, stderr, rc = run_cli("fork", vm_name, image_name, uds_path=uds_path)
        assert rc == 0, f"Fork stopped VM failed: {stderr}"

        # Boot from image and verify state
        _, _, rc = run_cli("create", "--name", forked_vm, "--image", image_name, uds_path=uds_path)
        assert rc == 0
        assert wait_exec_ready(client, forked_vm), f"Forked VM {forked_vm} never exec-ready"

        stdout, _, rc = run_cli("exec", forked_vm, "cat /root/stop_test.txt", uds_path=uds_path)
        assert rc == 0
        assert "stopped-marker" in stdout
    finally:
        for vm in [forked_vm, vm_name]:
            run_cli("delete", vm, uds_path=uds_path)
        run_cli("image", "delete", image_name, uds_path=uds_path)


def test_fork_nonexistent_vm(uds_path):
    """Forking a non-existent VM should fail."""
    _, _, rc = run_cli("fork", "ghost-vm-999", "some-image", uds_path=uds_path)
    assert rc != 0


def test_fork_duplicate_image_name(uds_path):
    """Forking to an already-existing image name should fail."""
    vm_name = f"ds-{uuid.uuid4().hex[:6]}"
    image_name = f"di-{uuid.uuid4().hex[:6]}"
    client = UdsHttpClient(uds_path)

    try:
        run_cli("create", "--name", vm_name, uds_path=uds_path)
        assert wait_exec_ready(client, vm_name), f"VM {vm_name} never exec-ready"

        # First fork succeeds
        _, _, rc = run_cli("fork", vm_name, image_name, uds_path=uds_path)
        assert rc == 0

        # Second fork to same name should fail
        _, stderr, rc = run_cli("fork", vm_name, image_name, uds_path=uds_path)
        assert rc != 0
    finally:
        run_cli("delete", vm_name, uds_path=uds_path)
        run_cli("image", "delete", image_name, uds_path=uds_path)


def test_create_from_nonexistent_image(uds_path):
    """Creating a VM from a non-existent image should fail."""
    _, _, rc = run_cli("create", "--name", f"vm-{uuid.uuid4().hex[:8]}",
                       "--image", "no-such-image-999", uds_path=uds_path)
    assert rc != 0


def test_inspect_nonexistent_image(uds_path):
    """Inspecting a non-existent image should fail."""
    _, _, rc = run_cli("image", "inspect", "no-such-image-999", uds_path=uds_path)
    assert rc != 0


def test_delete_nonexistent_image(uds_path):
    """Deleting a non-existent image should fail."""
    _, _, rc = run_cli("image", "delete", "no-such-image-999", uds_path=uds_path)
    assert rc != 0


def test_multiple_vms_from_same_image(uds_path):
    """Boot two VMs from the same image -- each should be independent."""
    vm_name = f"ms-{uuid.uuid4().hex[:6]}"
    image_name = f"mi-{uuid.uuid4().hex[:6]}"
    vm_a = f"ma-{uuid.uuid4().hex[:6]}"
    vm_b = f"mb-{uuid.uuid4().hex[:6]}"
    client = UdsHttpClient(uds_path)

    try:
        # Create base VM with shared marker
        run_cli("create", "--name", vm_name, uds_path=uds_path)
        assert wait_exec_ready(client, vm_name), f"VM {vm_name} never exec-ready"
        run_cli("exec", vm_name, "echo 'shared' > /root/shared.txt", uds_path=uds_path)

        # Fork to image
        _, _, rc = run_cli("fork", vm_name, image_name, uds_path=uds_path)
        assert rc == 0

        # Boot two VMs from the same image
        run_cli("create", "--name", vm_a, "--image", image_name, uds_path=uds_path)
        run_cli("create", "--name", vm_b, "--image", image_name, uds_path=uds_path)
        assert wait_exec_ready(client, vm_a), f"VM-A {vm_a} never exec-ready"
        assert wait_exec_ready(client, vm_b), f"VM-B {vm_b} never exec-ready"

        # Both should have the shared marker
        stdout_a, _, _ = run_cli("exec", vm_a, "cat /root/shared.txt", uds_path=uds_path)
        stdout_b, _, _ = run_cli("exec", vm_b, "cat /root/shared.txt", uds_path=uds_path)
        assert "shared" in stdout_a
        assert "shared" in stdout_b

        # Write different data to prove independence
        run_cli("exec", vm_a, "echo 'only-a' > /root/unique.txt", uds_path=uds_path)
        run_cli("exec", vm_b, "echo 'only-b' > /root/unique.txt", uds_path=uds_path)

        stdout_a, _, _ = run_cli("exec", vm_a, "cat /root/unique.txt", uds_path=uds_path)
        stdout_b, _, _ = run_cli("exec", vm_b, "cat /root/unique.txt", uds_path=uds_path)
        assert "only-a" in stdout_a
        assert "only-b" in stdout_b
    finally:
        for vm in [vm_a, vm_b, vm_name]:
            run_cli("delete", vm, uds_path=uds_path)
        run_cli("image", "delete", image_name, uds_path=uds_path)
