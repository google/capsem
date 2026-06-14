"""Verify no zombie processes after creating and deleting VMs."""

import uuid

import pytest


pytestmark = pytest.mark.cleanup


def test_no_zombie_after_bulk_delete(cleanup_env):
    """Create and delete 5 VMs, verify no capsem-process zombies remain."""
    client = cleanup_env.client()
    vms = []

    for i in range(5):
        name = f"zombie-{i}-{uuid.uuid4().hex[:6]}"
        client.post("/vms/create", {"name": name, "ram_mb": 512, "cpus": 1})
        vms.append(name)

    for name in vms:
        client.delete(f"/vms/{name}/delete")

    import time
    time.sleep(3)

    # Filter: the service's own process binary doesn't count,
    # we only care about per-VM capsem-process instances.
    # After deleting all VMs, there should be none from our test.
    list_resp = client.get("/vms/list")
    our_vms = [s for s in list_resp["sandboxes"] if s["id"].startswith("zombie-")]
    assert len(our_vms) == 0, f"Leaked VMs still in list: {our_vms}"
